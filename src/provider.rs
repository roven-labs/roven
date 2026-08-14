//! OpenAI-compatible wire adapter for Roven's provider-independent agent protocol.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    sync::atomic::{AtomicBool, Ordering},
};

use serde::Serialize;
use thiserror::Error;

use crate::{
    agent::{AgentMessage, AgentRequest, ProviderEvent},
    tools::{RovenToolCall, RovenToolDefinition},
};

const MAX_COMPLETION_TOKENS: u16 = 4096;

#[derive(Debug, Error)]
pub(crate) enum ProviderError {
    #[error("Provider {stage} failed ({category}): {detail}")]
    Diagnostic {
        stage: &'static str,
        category: &'static str,
        detail: String,
    },
    #[error("Provider rate limit reached (HTTP 429){0}")]
    RateLimited(String),
    #[error("Provider rejected the request (HTTP {status})")]
    HttpStatus { status: u16 },
}

impl ProviderError {
    fn diagnostic(
        stage: &'static str,
        category: &'static str,
        detail: impl std::fmt::Display,
    ) -> Self {
        Self::Diagnostic {
            stage,
            category,
            detail: sanitize_diagnostic(&detail.to_string()),
        }
    }
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

impl OpenAiCompatibleProvider {
    pub(crate) fn stream(
        &self,
        api_key: &str,
        request: &AgentRequest,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) -> Result<(), ProviderError> {
        let payload =
            serde_json::to_string(&ChatCompletionsRequest::from_agent(&self.model, request))
                .map_err(|error| {
                    ProviderError::diagnostic(
                        "encode",
                        "json",
                        format!("could not encode the provider request: {error}"),
                    )
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
            let line = line.map_err(stream_read_error)?;
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
            Err(ProviderError::diagnostic(
                "stream",
                "unexpected_eof",
                "provider closed the stream before signalling completion",
            ))
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<ChatCompletionMessage>,
    stream: bool,
    max_tokens: u16,
    tool_choice: &'static str,
    tools: Vec<ChatCompletionTool>,
}

impl ChatCompletionsRequest {
    fn from_agent(model: &str, request: &AgentRequest) -> Self {
        Self {
            model: model.to_owned(),
            messages: request
                .messages
                .iter()
                .map(ChatCompletionMessage::from)
                .collect(),
            stream: true,
            max_tokens: MAX_COMPLETION_TOKENS,
            tool_choice: "auto",
            tools: request.tools.iter().map(ChatCompletionTool::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum ChatCompletionMessage {
    Content {
        role: &'static str,
        content: String,
    },
    AssistantToolCalls {
        role: &'static str,
        content: Option<String>,
        tool_calls: Vec<ChatCompletionToolCall>,
    },
    ToolResult {
        role: &'static str,
        tool_call_id: String,
        content: String,
    },
}

impl From<&AgentMessage> for ChatCompletionMessage {
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
                tool_calls: tool_calls
                    .iter()
                    .map(ChatCompletionToolCall::from)
                    .collect(),
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
struct ChatCompletionTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatCompletionToolFunction,
}

impl From<&RovenToolDefinition> for ChatCompletionTool {
    fn from(tool: &RovenToolDefinition) -> Self {
        Self {
            kind: "function",
            function: ChatCompletionToolFunction {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct ChatCompletionToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatCompletionToolCallFunction,
}

impl From<&RovenToolCall> for ChatCompletionToolCall {
    fn from(call: &RovenToolCall) -> Self {
        Self {
            id: call.id.clone(),
            kind: "function",
            function: ChatCompletionToolCallFunction {
                name: call.name.clone(),
                arguments: serde_json::to_string(&call.arguments)
                    .expect("tool arguments are JSON serializable"),
            },
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionToolCallFunction {
    name: String,
    arguments: String,
}

fn request_error(error: ureq::Error) -> ProviderError {
    match &error {
        ureq::Error::StatusCode(status) => response_error(*status, None),
        ureq::Error::HostNotFound => ProviderError::diagnostic("request", "dns", "host not found"),
        ureq::Error::Timeout(_) => ProviderError::diagnostic("request", "timeout", error),
        ureq::Error::Tls(_) => ProviderError::diagnostic("request", "tls", error),
        ureq::Error::InvalidProxyUrl | ureq::Error::ConnectProxyFailed(_) => {
            ProviderError::diagnostic("request", "proxy", error)
        }
        ureq::Error::ConnectionFailed => {
            ProviderError::diagnostic("request", "connection", "connection failed")
        }
        ureq::Error::Io(error) => io_error("request", error),
        ureq::Error::Protocol(_) | ureq::Error::Http(_) => {
            ProviderError::diagnostic("request", "protocol", error)
        }
        ureq::Error::RedirectFailed | ureq::Error::TooManyRedirects => {
            ProviderError::diagnostic("request", "redirect", error)
        }
        _ => ProviderError::diagnostic("request", "transport", error),
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
        ProviderError::HttpStatus { status }
    }
}

fn stream_read_error(error: std::io::Error) -> ProviderError {
    io_error("stream", &error)
}

fn io_error(stage: &'static str, error: &std::io::Error) -> ProviderError {
    ProviderError::diagnostic(
        stage,
        "io",
        format!(
            "kind={} detail={error}",
            normalized_error_kind(error.kind())
        ),
    )
}

fn normalized_error_kind(kind: std::io::ErrorKind) -> String {
    let mut normalized = String::new();
    for (index, character) in format!("{kind:?}").chars().enumerate() {
        if index > 0 && character.is_ascii_uppercase() {
            normalized.push('_');
        }
        normalized.push(character.to_ascii_lowercase());
    }
    normalized
}

fn sanitize_diagnostic(detail: &str) -> String {
    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut safe = String::new();
    let mut remainder = detail.as_str();

    loop {
        let lower = remainder.to_ascii_lowercase();
        let Some(offset) = lower.find("bearer ") else {
            safe.push_str(remainder);
            break;
        };
        let token_start = offset + "bearer ".len();
        safe.push_str(&remainder[..token_start]);
        safe.push_str("[redacted]");
        let token_length = remainder[token_start..]
            .find(char::is_whitespace)
            .unwrap_or(remainder[token_start..].len());
        remainder = &remainder[token_start + token_length..];
    }

    const MAX_DIAGNOSTIC_CHARS: usize = 512;
    let mut truncated = safe.chars().take(MAX_DIAGNOSTIC_CHARS).collect::<String>();
    if safe.chars().count() > MAX_DIAGNOSTIC_CHARS {
        truncated.push('…');
    }
    truncated
}

fn parse_sse_line(line: &str) -> Result<Vec<ProviderEvent>, ProviderError> {
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(Vec::new());
    };
    if data == "[DONE]" {
        return Ok(vec![ProviderEvent::Finished]);
    }
    let value: serde_json::Value = serde_json::from_str(data)
        .map_err(|_| ProviderError::diagnostic("stream", "json", "invalid JSON stream event"))?;
    if let Some(error) = value.get("error") {
        let code = error
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unspecified");
        return Err(ProviderError::diagnostic(
            "stream",
            "remote_error",
            format!("provider reported a stream error code={code}"),
        ));
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
    let value: serde_json::Value = serde_json::from_str(data).map_err(|_| {
        ProviderError::diagnostic("tool_calls", "json", "invalid JSON tool-call event")
    })?;
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
            .ok_or_else(|| {
                ProviderError::diagnostic(
                    "tool_calls",
                    "invalid_tool_call",
                    "tool call has no index",
                )
            })? as usize;
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
                return Err(ProviderError::diagnostic(
                    "tool_calls",
                    "invalid_tool_call",
                    "tool call is missing an ID or function name",
                ));
            }
            let arguments = serde_json::from_str(&parts.arguments).map_err(|_| {
                ProviderError::diagnostic(
                    "tool_calls",
                    "invalid_arguments",
                    "tool call arguments are not valid JSON",
                )
            })?;
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
pub(crate) mod test_support {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        thread::{self, JoinHandle},
    };

    pub(crate) fn response(status: &str, headers: &[(&str, &str)], body: &str) -> String {
        let headers = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>();
        format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    pub(crate) fn sse(body: &str) -> String {
        response("200 OK", &[("Content-Type", "text/event-stream")], body)
    }

    pub(crate) fn serve(responses: Vec<String>) -> (String, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            responses
                .into_iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&stream);
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.flush().unwrap();
                    request
                })
                .collect()
        });
        (endpoint, handle)
    }

    fn read_request(stream: &TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .or_else(|| {
                headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
            })
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();
        String::from_utf8(body).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::io;

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
        assert_eq!(
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec![
                "max_tokens",
                "messages",
                "model",
                "stream",
                "tool_choice",
                "tools"
            ]
        );
        assert_eq!(
            value["messages"],
            serde_json::json!([{
                "role": "user",
                "content": "Hello"
            }])
        );
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
    fn request_errors_preserve_the_transport_failure_category() {
        assert_eq!(
            request_error(ureq::Error::HostNotFound).to_string(),
            "Provider request failed (dns): host not found"
        );
        assert_eq!(
            request_error(ureq::Error::ConnectionFailed).to_string(),
            "Provider request failed (connection): connection failed"
        );
        assert_eq!(
            request_error(ureq::Error::Io(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "connection refused by provider",
            )))
            .to_string(),
            "Provider request failed (io): kind=connection_refused detail=connection refused by provider"
        );
        assert_eq!(
            request_error(ureq::Error::BodyExceedsLimit(42)).to_string(),
            "Provider request failed (transport): the response body is larger than request limit: 42"
        );
    }

    #[test]
    fn streamed_data_errors_report_the_stage_that_failed() {
        assert_eq!(
            parse_sse_line("data: not-json").unwrap_err().to_string(),
            "Provider stream failed (json): invalid JSON stream event"
        );
        assert_eq!(
            super::finish_tool_calls(std::collections::BTreeMap::new())
                .unwrap()
                .len(),
            0
        );

        let error = super::stream_read_error(io::Error::new(
            io::ErrorKind::TimedOut,
            "provider stopped responding",
        ));
        assert_eq!(
            error.to_string(),
            "Provider stream failed (io): kind=timed_out detail=provider stopped responding"
        );
    }

    #[test]
    fn diagnostic_detail_removes_bearer_tokens_and_line_breaks() {
        assert_eq!(
            super::sanitize_diagnostic("Authorization: Bearer secret-value\nrequest failed"),
            "Authorization: Bearer [redacted] request failed"
        );
    }

    #[test]
    fn provider_stream_errors_expose_a_code_without_logging_the_response_message() {
        let error = parse_sse_line(
            r#"data: {"error":{"code":"model_unavailable","message":"Bearer secret-value"}}"#,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            error,
            "Provider stream failed (remote_error): provider reported a stream error code=model_unavailable"
        );
        assert!(!error.contains("secret-value"));
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

    #[test]
    fn stream_reports_real_http_rate_limits_and_unexpected_eof() {
        let request = AgentRequest::new(vec![AgentMessage::User {
            content: "Hello".to_owned(),
        }]);
        let (endpoint, server) = super::test_support::serve(vec![super::test_support::response(
            "429 Too Many Requests",
            &[("retry-after", "7")],
            "",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let error = provider
            .stream(
                "key",
                &request,
                &std::sync::atomic::AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Provider rate limit reached (HTTP 429); retry after 7 seconds"
        );
        assert_eq!(server.join().unwrap().len(), 1);

        let (endpoint, server) = super::test_support::serve(vec![super::test_support::sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let error = provider
            .stream(
                "key",
                &request,
                &std::sync::atomic::AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Provider stream failed (unexpected_eof): provider closed the stream before signalling completion"
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
