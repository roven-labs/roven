//! Blocking OpenRouter streaming adapter used exclusively by the worker thread.

use std::{
    io::{BufRead, BufReader},
    sync::atomic::{AtomicBool, Ordering},
};

use serde::Serialize;
use thiserror::Error;

pub(crate) const MODEL: &str = "openai/gpt-oss-20b:free";
const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
const MAX_COMPLETION_TOKENS: u16 = 4096;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatMessage {
    pub(crate) role: &'static str,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReasoningOptions {
    enabled: bool,
    exclude: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatRequest {
    pub(crate) model: &'static str,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) stream: bool,
    pub(crate) max_tokens: u16,
    pub(crate) parallel_tool_calls: bool,
    pub(crate) reasoning: ReasoningOptions,
}

impl ChatRequest {
    pub(crate) fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            model: MODEL,
            messages,
            stream: true,
            max_tokens: MAX_COMPLETION_TOKENS,
            parallel_tool_calls: false,
            reasoning: ReasoningOptions {
                enabled: true,
                exclude: false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamEvent {
    Thought(String),
    Text(String),
    Finished,
    Cancelled,
}

#[derive(Debug, Error)]
pub(crate) enum ProviderError {
    #[error("{0}")]
    Request(String),
    #[error("OpenRouter returned an invalid streaming response")]
    Stream,
    #[error("OpenRouter reported an error: {0}")]
    Remote(String),
}

pub(crate) trait ModelProvider {
    fn stream(
        &self,
        api_key: &str,
        request: &ChatRequest,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(StreamEvent),
    ) -> Result<(), ProviderError>;
}

#[derive(Debug, Default)]
pub(crate) struct OpenRouterProvider;

impl ModelProvider for OpenRouterProvider {
    fn stream(
        &self,
        api_key: &str,
        request: &ChatRequest,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(StreamEvent),
    ) -> Result<(), ProviderError> {
        let payload = serde_json::to_string(request).map_err(|_| {
            ProviderError::Request("Roven could not encode the OpenRouter request".to_owned())
        })?;
        let mut response = ureq::post(ENDPOINT)
            .header("Authorization", &format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .send(payload)
            .map_err(request_error)?;
        let reader = BufReader::new(response.body_mut().as_reader());

        for line in reader.lines() {
            if cancelled.load(Ordering::Relaxed) {
                emit(StreamEvent::Cancelled);
                return Ok(());
            }
            let line = line.map_err(|_| ProviderError::Stream)?;
            for event in parse_sse_line(&line)? {
                if matches!(event, StreamEvent::Finished) {
                    emit(StreamEvent::Finished);
                    return Ok(());
                }
                emit(event);
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            emit(StreamEvent::Cancelled);
            Ok(())
        } else {
            Err(ProviderError::Stream)
        }
    }
}

fn request_error(error: ureq::Error) -> ProviderError {
    match error {
        ureq::Error::StatusCode(status) => {
            ProviderError::Request(format!("OpenRouter rejected the request (HTTP {status})"))
        }
        _ => ProviderError::Request(
            "Roven could not connect to OpenRouter before streaming began".to_owned(),
        ),
    }
}

fn parse_sse_line(line: &str) -> Result<Vec<StreamEvent>, ProviderError> {
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(Vec::new());
    };
    if data == "[DONE]" {
        return Ok(vec![StreamEvent::Finished]);
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
        events.push(StreamEvent::Thought(thought));
    }
    let text = delta
        .and_then(|delta| delta.get("content"))
        .and_then(serde_json::Value::as_str);
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        events.push(StreamEvent::Text(text.to_owned()));
    }
    if choice
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        events.push(StreamEvent::Finished);
    }
    Ok(events)
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
    use super::{ChatMessage, ChatRequest, MODEL, StreamEvent, parse_sse_line};

    #[test]
    fn request_uses_the_fixed_streaming_model_and_completion_limit() {
        let request = ChatRequest::new(vec![ChatMessage {
            role: "user",
            content: "Hello".to_owned(),
            reasoning: None,
        }]);
        assert_eq!(request.model, MODEL);
        assert!(request.stream);
        assert_eq!(request.max_tokens, 4096);
        assert!(!request.parallel_tool_calls);
        assert!(request.reasoning.enabled);
        assert!(!request.reasoning.exclude);
        assert_eq!(
            serde_json::to_value(&request).unwrap()["reasoning"],
            serde_json::json!({ "enabled": true, "exclude": false })
        );
    }

    #[test]
    fn sse_parser_returns_text_and_completion_events() {
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
    fn sse_parser_returns_provider_reasoning_before_text() {
        assert_eq!(
            parse_sse_line(
                r#"data: {"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"Check the project first."}],"content":"Here is the answer."}}]}"#
            )
            .unwrap(),
            vec![
                StreamEvent::Thought("Check the project first.".to_owned()),
                StreamEvent::Text("Here is the answer.".to_owned()),
            ]
        );
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"reasoning":"Check the error."}}]}"#)
                .unwrap(),
            vec![StreamEvent::Thought("Check the error.".to_owned())]
        );
    }

    #[test]
    fn request_error_keeps_the_http_status_without_exposing_credentials() {
        let error = super::request_error(ureq::Error::StatusCode(401));

        assert_eq!(
            error.to_string(),
            "OpenRouter rejected the request (HTTP 401)"
        );
        assert!(!error.to_string().contains("Bearer"));
    }
}
