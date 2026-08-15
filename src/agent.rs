//! Provider-independent agent lifecycle and tool orchestration.

use std::sync::atomic::AtomicBool;

use crate::tools::{
    RovenToolCall, RovenToolDefinition, RovenToolResult, ToolContext, definitions, dispatch,
};
use crate::{
    provider::{OpenAiCompatibleProvider, ProviderError},
    runtime_log::RuntimeLog,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AgentMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
        reasoning: Option<String>,
        tool_calls: Vec<RovenToolCall>,
    },
    Tool {
        result: RovenToolResult,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentRequest {
    pub(crate) messages: Vec<AgentMessage>,
    pub(crate) tools: Vec<RovenToolDefinition>,
}

impl AgentRequest {
    pub(crate) fn new(messages: Vec<AgentMessage>) -> Self {
        Self {
            messages,
            tools: definitions(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProviderEvent {
    Thought(String),
    Text(String),
    ToolCalls(Vec<RovenToolCall>),
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AgentEvent {
    Thought(String),
    Text(String),
    ToolResult {
        call: RovenToolCall,
        result: RovenToolResult,
    },
    Finished,
    Cancelled,
}

/// Continue the same user turn until the provider produces a final response.
/// Tool execution and authorization stay here; adapters only transport events.
pub(crate) fn run(
    provider: &OpenAiCompatibleProvider,
    api_key: &str,
    mut messages: Vec<AgentMessage>,
    context: &ToolContext,
    cancelled: &AtomicBool,
    runtime_log: Option<&RuntimeLog>,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<(), ProviderError> {
    loop {
        let request = AgentRequest::new(messages.clone());
        record(
            runtime_log,
            "model_request_started",
            &format!(
                "messages={} tools={}",
                request.messages.len(),
                request.tools.len()
            ),
        );
        let mut response_content = String::new();
        let mut response_reasoning = String::new();
        let mut tool_calls = None;
        let mut finished = false;
        let mut was_cancelled = false;

        if let Err(error) =
            provider.stream(api_key, &request, cancelled, &mut |event| match event {
                ProviderEvent::Thought(thought) => {
                    response_reasoning.push_str(&thought);
                    record(
                        runtime_log,
                        "reasoning_received",
                        &format!("characters={}", thought.chars().count()),
                    );
                    emit(AgentEvent::Thought(thought));
                }
                ProviderEvent::Text(text) => {
                    response_content.push_str(&text);
                    record(
                        runtime_log,
                        "text_received",
                        &format!("characters={}", text.chars().count()),
                    );
                    emit(AgentEvent::Text(text));
                }
                ProviderEvent::ToolCalls(calls) => {
                    record(
                        runtime_log,
                        "tool_calls_received",
                        &format!("count={}", calls.len()),
                    );
                    tool_calls = Some(calls);
                }
                ProviderEvent::Finished => {
                    record(runtime_log, "model_response_finished", "outcome=finished");
                    finished = true;
                }
                ProviderEvent::Cancelled => {
                    record(runtime_log, "model_response_cancelled", "outcome=cancelled");
                    was_cancelled = true;
                }
            })
        {
            record(
                runtime_log,
                "model_request_failed",
                &format!("error={error}"),
            );
            return Err(error);
        }

        if was_cancelled {
            emit(AgentEvent::Cancelled);
            return Ok(());
        }
        if let Some(tool_calls) = tool_calls {
            messages.push(AgentMessage::Assistant {
                content: response_content,
                reasoning: (!response_reasoning.is_empty()).then_some(response_reasoning),
                tool_calls: tool_calls.clone(),
            });
            for call in tool_calls {
                record(
                    runtime_log,
                    "tool_dispatch_started",
                    &format!("name={}", call.name),
                );
                let result = dispatch(context, call.clone());
                let status = result
                    .result
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                let reason = result
                    .result
                    .get("reason")
                    .and_then(serde_json::Value::as_str);
                record(
                    runtime_log,
                    "tool_dispatch_finished",
                    &format!(
                        "name={} status={}{}",
                        result.name,
                        status,
                        reason.map_or_else(String::new, |reason| format!(" reason={reason}"))
                    ),
                );
                emit(AgentEvent::ToolResult {
                    call,
                    result: result.clone(),
                });
                messages.push(AgentMessage::Tool { result });
            }
            continue;
        }
        if finished {
            emit(AgentEvent::Finished);
        }
        return Ok(());
    }
}

fn record(runtime_log: Option<&RuntimeLog>, event: &str, detail: &str) {
    if let Some(log) = runtime_log {
        log.record("agent", event, detail);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, sync::atomic::AtomicBool};

    use crate::{
        provider::{OpenAiCompatibleProvider, ProviderError, test_support},
        runtime_log::RuntimeLog,
        tools::ToolContext,
    };

    use super::{AgentEvent, AgentMessage, run};

    fn workspace() -> std::path::PathBuf {
        std::env::current_dir().unwrap().canonicalize().unwrap()
    }

    #[test]
    fn final_response_streams_every_chunk_and_finishes() {
        let (endpoint, server) = test_support::serve(vec![test_support::sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"done \"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"now\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let events = RefCell::new(Vec::new());

        run(
            &provider,
            "key",
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &ToolContext::new(workspace()).unwrap(),
            &AtomicBool::new(false),
            None,
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            vec![
                AgentEvent::Text("done ".to_owned()),
                AgentEvent::Text("now".to_owned()),
                AgentEvent::Finished,
            ]
        );
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("\"model\":\"test-model\""));
    }

    #[test]
    fn tool_calls_round_trip_through_the_real_provider() {
        let (endpoint, server) = test_support::serve(vec![
            test_support::sse(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_prepare","function":{"name":"prepare_project","arguments":"{\"path\":\"does-not-exist\"}"}}]},"finish_reason":"tool_calls"}]}

"#,
            ),
            test_support::sse(
                "data: {\"choices\":[{\"delta\":{\"content\":\"registration needs a valid path\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n",
            ),
        ]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let events = RefCell::new(Vec::new());

        run(
            &provider,
            "key",
            vec![AgentMessage::User {
                content: "register this".to_owned(),
            }],
            &ToolContext::new(workspace()).unwrap(),
            &AtomicBool::new(false),
            None,
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();

        assert!(matches!(
            events.borrow().first(),
            Some(AgentEvent::ToolResult { result, .. }) if result.result["reason"] == "invalid_path"
        ));
        assert!(matches!(events.borrow().last(), Some(AgentEvent::Finished)));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("\"tool_call_id\":\"call_prepare\""));
    }

    #[test]
    fn read_file_tool_calls_round_trip_through_the_real_provider() {
        let (endpoint, server) = test_support::serve(vec![
            test_support::sse(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_read_file","function":{"name":"read_file","arguments":"{\"path\":\"Cargo.toml\"}"}}]},"finish_reason":"tool_calls"}]}

"#,
            ),
            test_support::sse(
                "data: {\"choices\":[{\"delta\":{\"content\":\"file read\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n",
            ),
        ]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let events = RefCell::new(Vec::new());

        run(
            &provider,
            "key",
            vec![AgentMessage::User {
                content: "read Cargo.toml".to_owned(),
            }],
            &ToolContext::new(workspace()).unwrap(),
            &AtomicBool::new(false),
            None,
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();

        assert!(events.borrow().iter().any(|event| matches!(
            event,
            AgentEvent::ToolResult { result, .. }
                if result.result["status"] == "ok"
                    && result.result["path"] == "Cargo.toml"
                    && result.result["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("[package]"))
        )));
        assert!(matches!(events.borrow().last(), Some(AgentEvent::Finished)));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("\"tool_call_id\":\"call_read_file\""));
        assert!(requests[1].contains("\\\"status\\\":\\\"ok\\\""));
    }

    #[test]
    fn provider_failures_are_logged_and_cancellation_is_reported() {
        let (endpoint, server) = test_support::serve(vec![test_support::response(
            "500 Internal Server Error",
            &[],
            "",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let log_path =
            std::env::temp_dir().join(format!("roven-agent-log-{}.md", uuid::Uuid::now_v7()));
        let log = RuntimeLog::for_file(&log_path).unwrap();

        let result = run(
            &provider,
            "key",
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &ToolContext::new(workspace()).unwrap(),
            &AtomicBool::new(false),
            Some(&log),
            &mut |_| {},
        );

        assert!(matches!(
            result,
            Err(ProviderError::HttpStatus { status: 500 })
        ));
        assert!(
            fs::read_to_string(log_path)
                .unwrap()
                .contains("event=model_request_failed")
        );
        assert_eq!(server.join().unwrap().len(), 1);

        let (endpoint, server) = test_support::serve(vec![test_support::sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ignored\"},\"finish_reason\":null}]}\n\n",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let events = RefCell::new(Vec::new());
        run(
            &provider,
            "key",
            vec![AgentMessage::User {
                content: "cancel".to_owned(),
            }],
            &ToolContext::new(workspace()).unwrap(),
            &AtomicBool::new(true),
            None,
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();
        assert_eq!(events.into_inner(), vec![AgentEvent::Cancelled]);
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
