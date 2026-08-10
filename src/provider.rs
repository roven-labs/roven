//! OpenAI-compatible wire adapter for Roven's provider-independent agent protocol.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    sync::atomic::{AtomicBool, Ordering},
};

use serde::Serialize;
use thiserror::Error;

use crate::{
    agent::{AgentMessage, AgentRequest, ModelProvider, ProviderEvent},
    tools::{RovenToolCall, RovenToolDefinition},
};

const MAX_COMPLETION_TOKENS: u16 = 4096;

#[derive(Debug, Error)]
pub(crate) enum ProviderError {
    #[error("{0}")]
    Request(String),
    #[error("Provider rate limit reached (HTTP 429){0}")]
    RateLimited(String),
    #[error("Provider returned an invalid streaming response")]
    Stream,
    #[error("Provider reported an error: {0}")]
    Remote(String),
}

#[derive(Debug, Clone)]
pub(crate) struct OpenAiCompatibleProvider {
    endpoint: String,
    model: String,
}

impl OpenAiCompatibleProvider {
    pub(crate) fn new(endpoint: String, model: String) -> Self {
        Self { endpoint, model }
    }

    fn request_endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl ModelProvider for OpenAiCompatibleProvider {
    type Error = ProviderError;

    fn stream(
        &self,
        api_key: &str,
        request: &AgentRequest,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) -> Result<(), Self::Error> {
        let payload =
            serde_json::to_string(&ChatCompletionsRequest::from_agent(&self.model, request))
                .map_err(|_| {
                    ProviderError::Request("Roven could not encode the provider request".to_owned())
                })?;
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let mut response = agent
            .post(self.request_endpoint())
            .header("Authorization", &format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .send(payload)
            .map_err(request_error)?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(response_error(
                status,
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok()),
            ));
        }
        let reader = BufReader::new(response.body_mut().as_reader());
        let mut tool_call_parts = BTreeMap::new();

        for line in reader.lines() {
            if cancelled.load(Ordering::Relaxed) {
                emit(ProviderEvent::Cancelled);
                return Ok(());
            }
            let line = line.map_err(|_| ProviderError::Stream)?;
            append_tool_call_deltas(&line, &mut tool_call_parts)?;
            for event in parse_sse_line(&line)? {
                if matches!(event, ProviderEvent::Finished) {
                    if tool_call_parts.is_empty() {
                        emit(ProviderEvent::Finished);
                    } else {
                        emit(ProviderEvent::ToolCalls(finish_tool_calls(
                            tool_call_parts,
                        )?));
                    }
                    return Ok(());
                }
                emit(event);
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            emit(ProviderEvent::Cancelled);
            Ok(())
        } else {
            Err(ProviderError::Stream)
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<OpenRouterMessage>,
    stream: bool,
    max_tokens: u16,
    tool_choice: &'static str,
    tools: Vec<OpenRouterTool>,
}

impl ChatCompletionsRequest {
    fn from_agent(model: &str, request: &AgentRequest) -> Self {
        Self {
            model: model.to_owned(),
            messages: request
                .messages
                .iter()
                .map(OpenRouterMessage::from)
                .collect(),
            stream: true,
            max_tokens: MAX_COMPLETION_TOKENS,
            tool_choice: "auto",
            tools: request.tools.iter().map(OpenRouterTool::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum OpenRouterMessage {
    Content {
        role: &'static str,
        content: String,
    },
    AssistantToolCalls {
        role: &'static str,
        content: Option<String>,
        tool_calls: Vec<OpenRouterToolCall>,
    },
    ToolResult {
        role: &'static str,
        tool_call_id: String,
        content: String,
    },
}

impl From<&AgentMessage> for OpenRouterMessage {
    fn from(message: &AgentMessage) -> Self {
        match message {
            AgentMessage::System { content } => Self::Content {
                role: "system",
                content: content.clone(),
            },
            AgentMessage::User { content } => Self::Content {
                role: "user",
                content: content.clone(),
            },
            AgentMessage::Assistant {
                content,
                reasoning: _,
                tool_calls,
            } if tool_calls.is_empty() => Self::Content {
                role: "assistant",
                content: content.clone(),
            },
            AgentMessage::Assistant {
                content,
                reasoning: _,
                tool_calls,
            } => Self::AssistantToolCalls {
                role: "assistant",
                content: (!content.is_empty()).then(|| content.clone()),
                tool_calls: tool_calls.iter().map(OpenRouterToolCall::from).collect(),
            },
            AgentMessage::Tool { result } => Self::ToolResult {
                role: "tool",
                tool_call_id: result.tool_call_id.clone(),
                content: serde_json::to_string(&result.result)
                    .expect("tool results are JSON serializable"),
            },
        }
    }
}

#[derive(Serialize)]
struct OpenRouterTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenRouterToolFunction,
}

impl From<&RovenToolDefinition> for OpenRouterTool {
    fn from(tool: &RovenToolDefinition) -> Self {
        Self {
            kind: "function",
            function: OpenRouterToolFunction {
                name: tool.name,
                description: tool.description,
                parameters: tool.input_schema.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct OpenRouterToolFunction {
    name: &'static str,
    description: &'static str,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct OpenRouterToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenRouterToolCallFunction,
}

impl From<&RovenToolCall> for OpenRouterToolCall {
    fn from(call: &RovenToolCall) -> Self {
        Self {
            id: call.id.clone(),
            kind: "function",
            function: OpenRouterToolCallFunction {
                name: call.name.clone(),
                arguments: serde_json::to_string(&call.arguments)
                    .expect("tool arguments are JSON serializable"),
            },
        }
    }
}

#[derive(Serialize)]
struct OpenRouterToolCallFunction {
    name: String,
    arguments: String,
}

fn request_error(error: ureq::Error) -> ProviderError {
    match error {
        ureq::Error::StatusCode(status) => response_error(status, None),
        _ => ProviderError::Request(
            "Roven could not connect to the configured provider before streaming began".to_owned(),
        ),
    }
}

fn response_error(status: u16, retry_after: Option<&str>) -> ProviderError {
    if status == 429 {
        let detail = retry_after
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("; retry after {value} seconds"))
            .unwrap_or_else(|| {
                ". Wait briefly and try again; the configured model or account may have no capacity."
                    .to_owned()
            });
        ProviderError::RateLimited(detail)
    } else {
        ProviderError::Request(format!("Provider rejected the request (HTTP {status})"))
    }
}

fn parse_sse_line(line: &str) -> Result<Vec<ProviderEvent>, ProviderError> {
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(Vec::new());
    };
    if data == "[DONE]" {
        return Ok(vec![ProviderEvent::Finished]);
    }
    let value: serde_json::Value = serde_json::from_str(data).map_err(|_| ProviderError::Stream)?;
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown provider error")
            .to_owned();
        return Err(ProviderError::Remote(message));
    }
    let choice = value.get("choices").and_then(|choices| choices.get(0));
    let delta = choice.and_then(|choice| choice.get("delta"));
    let mut events = Vec::new();
    if let Some(thought) = delta.and_then(reasoning_from_delta) {
        events.push(ProviderEvent::Thought(thought));
    }
    let text = delta
        .and_then(|delta| delta.get("content"))
        .and_then(serde_json::Value::as_str);
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        events.push(ProviderEvent::Text(text.to_owned()));
    }
    if choice
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        events.push(ProviderEvent::Finished);
    }
    Ok(events)
}

#[derive(Default)]
struct ToolCallParts {
    id: String,
    name: String,
    arguments: String,
}

fn append_tool_call_deltas(
    line: &str,
    tool_call_parts: &mut BTreeMap<usize, ToolCallParts>,
) -> Result<(), ProviderError> {
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(());
    };
    if data == "[DONE]" {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_str(data).map_err(|_| ProviderError::Stream)?;
    let calls = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("tool_calls"))
        .and_then(serde_json::Value::as_array);
    let Some(calls) = calls else {
        return Ok(());
    };
    for call in calls {
        let index = call
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .ok_or(ProviderError::Stream)? as usize;
        let parts = tool_call_parts.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(serde_json::Value::as_str) {
            parts.id = id.to_owned();
        }
        if let Some(name) = call
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(serde_json::Value::as_str)
        {
            parts.name.push_str(name);
        }
        if let Some(arguments) = call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(serde_json::Value::as_str)
        {
            parts.arguments.push_str(arguments);
        }
    }
    Ok(())
}

fn finish_tool_calls(
    tool_call_parts: BTreeMap<usize, ToolCallParts>,
) -> Result<Vec<RovenToolCall>, ProviderError> {
    tool_call_parts
        .into_values()
        .map(|parts| {
            if parts.id.is_empty() || parts.name.is_empty() {
                return Err(ProviderError::Stream);
            }
            let arguments =
                serde_json::from_str(&parts.arguments).map_err(|_| ProviderError::Stream)?;
            Ok(RovenToolCall {
                id: parts.id,
                name: parts.name,
                arguments,
            })
        })
        .collect()
}

fn reasoning_from_delta(delta: &serde_json::Value) -> Option<String> {
    delta
        .get("reasoning")
        .or_else(|| delta.get("reasoning_content"))
        .and_then(serde_json::Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            delta
                .get("reasoning_details")
                .and_then(serde_json::Value::as_array)
                .and_then(|details| {
                    let text = details
                        .iter()
                        .filter_map(|detail| {
                            detail
                                .get("text")
                                .or_else(|| detail.get("summary"))
                                .and_then(serde_json::Value::as_str)
                        })
                        .filter(|text| !text.is_empty())
                        .collect::<String>();
                    (!text.is_empty()).then_some(text)
                })
        })
}

#[cfg(test)]
mod tests {
    use crate::{
        agent::{AgentMessage, AgentRequest, ProviderEvent},
        tools::RovenToolCall,
    };

    use super::{
        ChatCompletionsRequest, OpenAiCompatibleProvider, parse_sse_line, request_error,
        response_error,
    };

    type StreamEvent = ProviderEvent;

    #[test]
    fn provider_uses_the_configured_endpoint_without_appending_a_path() {
        let provider = OpenAiCompatibleProvider::new(
            "https://ollama.com/v1/chat/completions".to_owned(),
            "gemma4:31b-cloud".to_owned(),
        );

        assert_eq!(
            provider.request_endpoint(),
            "https://ollama.com/v1/chat/completions"
        );
    }

    #[test]
    fn request_uses_profile_model_and_standard_openai_fields() {
        let request = AgentRequest::new(vec![AgentMessage::User {
            content: "Hello".to_owned(),
        }]);
        let request = ChatCompletionsRequest::from_agent("llama-3.3-70b-versatile", &request);
        let value = serde_json::to_value(&request).unwrap();

        assert_eq!(request.model, "llama-3.3-70b-versatile");
        assert!(request.stream);
        assert_eq!(request.max_tokens, 4096);
        assert_eq!(value["tool_choice"], "auto");
        assert_eq!(value["tools"][0]["function"]["name"], "prepare_project");
        assert!(value.get("reasoning").is_none());
        assert!(value.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn parser_returns_text_and_completion_events() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"choices":[{"delta":{"content":"Hi"},"finish_reason":null}]}"#
            )
            .unwrap(),
            vec![StreamEvent::Text("Hi".to_owned())]
        );
        assert_eq!(
            parse_sse_line("data: [DONE]").unwrap(),
            vec![StreamEvent::Finished]
        );
    }

    #[test]
    fn tool_call_chunks_become_roven_tool_calls() {
        let mut chunks = std::collections::BTreeMap::new();
        super::append_tool_call_deltas(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"prepare_","arguments":"{\"path\":\"C:"}}]}}]}"#,
            &mut chunks,
        )
        .unwrap();
        super::append_tool_call_deltas(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"project","arguments":"\\\\work\"}"}}]}}]}"#,
            &mut chunks,
        )
        .unwrap();

        assert_eq!(
            super::finish_tool_calls(chunks).unwrap(),
            vec![RovenToolCall {
                id: "call_1".to_owned(),
                name: "prepare_project".to_owned(),
                arguments: serde_json::json!({ "path": "C:\\work" }),
            }]
        );
    }

    #[test]
    fn request_error_keeps_the_http_status_without_exposing_credentials() {
        let error = request_error(ureq::Error::StatusCode(401));

        assert_eq!(
            error.to_string(),
            "Provider rejected the request (HTTP 401)"
        );
        assert!(!error.to_string().contains("Bearer"));
    }

    #[test]
    fn rate_limits_are_actionable_and_include_the_server_retry_delay() {
        assert_eq!(
            response_error(429, Some("30")).to_string(),
            "Provider rate limit reached (HTTP 429); retry after 30 seconds"
        );
        assert_eq!(
            response_error(429, None).to_string(),
            "Provider rate limit reached (HTTP 429). Wait briefly and try again; the configured model or account may have no capacity."
        );
    }
}
