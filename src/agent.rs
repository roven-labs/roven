//! Provider-independent agent lifecycle and tool orchestration.

use std::sync::atomic::AtomicBool;

use crate::tools::{
    definitions, dispatch, RovenToolCall, RovenToolDefinition, RovenToolResult, ToolContext,
};
use crate::{
    context,
    provider::{Provider, ProviderError},
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
    pub(crate) fn new(messages: Vec<AgentMessage>, allow_tools: bool) -> Self {
        Self {
            messages,
            tools: allow_tools.then(definitions).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProviderEvent {
    Thought(String),
    Text(String),
    ToolCalls(Vec<RovenToolCall>),
    ContextUsage(usize),
    ResponseFinished,
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
    ContextUsage(usize),
    Finished,
    Cancelled,
}

pub(crate) struct AgentRun<'a> {
    pub(crate) provider: &'a dyn Provider,
    pub(crate) api_key: &'a str,
    pub(crate) tool_context: &'a ToolContext,
    pub(crate) context_window: Option<usize>,
    pub(crate) cancelled: &'a AtomicBool,
    pub(crate) runtime_log: Option<&'a RuntimeLog>,
    pub(crate) allow_tools: bool,
}

/// Continue the same user turn until the provider produces a final response.
/// Tool execution and authorization stay here; adapters only transport events.
pub(crate) fn run(
    agent_run: AgentRun<'_>,
    mut messages: Vec<AgentMessage>,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<(), ProviderError> {
    let AgentRun {
        provider,
        api_key,
        tool_context,
        context_window,
        cancelled,
        runtime_log,
        allow_tools,
    } = agent_run;
    loop {
        let request = AgentRequest::new(messages.clone(), allow_tools);
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
        let mut unexpected_tool_calls = false;
        let mut finished = false;
        let mut was_cancelled = false;

        if let Err(error) = provider.stream(
            api_key,
            &request,
            cancelled,
            runtime_log,
            &mut |event| match event {
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
                    if !allow_tools {
                        unexpected_tool_calls = true;
                        return;
                    }
                    tool_calls = Some(calls);
                }
                ProviderEvent::ContextUsage(prompt_tokens) => {
                    if let Some(context_window) = context_window {
                        let context_percent = context::percent(prompt_tokens, context_window);
                        record(
                            runtime_log,
                            "context_usage_received",
                            &format!("percent={context_percent}"),
                        );
                        emit(AgentEvent::ContextUsage(context_percent));
                    }
                }
                ProviderEvent::ResponseFinished => {}
                ProviderEvent::Finished => {
                    record(runtime_log, "model_response_finished", "outcome=finished");
                    finished = true;
                }
                ProviderEvent::Cancelled => {
                    record(runtime_log, "model_response_cancelled", "outcome=cancelled");
                    was_cancelled = true;
                }
            },
        ) {
            record(
                runtime_log,
                "model_request_failed",
                &format!("error={error}"),
            );
            return Err(error);
        }

        if unexpected_tool_calls {
            return Err(ProviderError::diagnostic(
                "agent",
                "tool_calls",
                "unexpected tool call in no-tools turn",
            ));
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
                let result = dispatch(tool_context, call.clone());
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
        provider::{test_support, OpenAiCompatibleProvider, Provider, ProviderError},
        runtime_log::RuntimeLog,
        tools::ToolContext,
    };

    use super::{run, AgentEvent, AgentMessage, AgentRun};

    fn workspace() -> std::path::PathBuf {
        std::env::current_dir().unwrap().canonicalize().unwrap()
    }

    struct ContextUsageProvider;

    impl Provider for ContextUsageProvider {
        fn stream(
            &self,
            _api_key: &str,
            _request: &super::AgentRequest,
            _cancelled: &AtomicBool,
            _runtime_log: Option<&RuntimeLog>,
            emit: &mut dyn FnMut(super::ProviderEvent),
        ) -> Result<(), ProviderError> {
            emit(super::ProviderEvent::ContextUsage(50));
            emit(super::ProviderEvent::Finished);
            Ok(())
        }
    }

    struct ToolCallProvider;

    impl Provider for ToolCallProvider {
        fn stream(
            &self,
            _api_key: &str,
            _request: &super::AgentRequest,
            _cancelled: &AtomicBool,
            _runtime_log: Option<&RuntimeLog>,
            emit: &mut dyn FnMut(super::ProviderEvent),
        ) -> Result<(), ProviderError> {
            emit(super::ProviderEvent::ToolCalls(vec![
                crate::tools::RovenToolCall {
                    id: "unexpected".to_owned(),
                    name: "list_tools".to_owned(),
                    arguments: serde_json::json!({}),
                },
            ]));
            emit(super::ProviderEvent::Finished);
            Ok(())
        }
    }

    #[test]
    fn no_tools_turn_rejects_provider_tool_calls_without_dispatch() {
        let provider = ToolCallProvider;
        let tool_context = ToolContext::new(workspace()).unwrap();
        let events = RefCell::new(Vec::new());
        let result = run(
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &tool_context,
                context_window: None,
                cancelled: &AtomicBool::new(false),
                runtime_log: None,
                allow_tools: false,
            },
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &mut |event| events.borrow_mut().push(event),
        );

        assert!(result.is_err());
        assert!(!events
            .borrow()
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolResult { .. })));
    }

    #[test]
    fn no_tools_request_has_no_tool_definitions() {
        let request = super::AgentRequest::new(
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            false,
        );
        assert!(request.tools.is_empty());
    }

    #[test]
    fn final_response_streams_every_chunk_and_finishes() {
        let (endpoint, server) = test_support::serve(vec![test_support::sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"done \"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"now\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let events = RefCell::new(Vec::new());

        run(
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &ToolContext::new(workspace()).unwrap(),
                context_window: None,
                cancelled: &AtomicBool::new(false),
                runtime_log: None,
                allow_tools: true,
            },
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
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
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &ToolContext::new(workspace()).unwrap(),
                context_window: None,
                cancelled: &AtomicBool::new(false),
                runtime_log: None,
                allow_tools: true,
            },
            vec![AgentMessage::User {
                content: "register this".to_owned(),
            }],
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
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &ToolContext::new(workspace()).unwrap(),
                context_window: None,
                cancelled: &AtomicBool::new(false),
                runtime_log: None,
                allow_tools: true,
            },
            vec![AgentMessage::User {
                content: "read Cargo.toml".to_owned(),
            }],
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
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &ToolContext::new(workspace()).unwrap(),
                context_window: None,
                cancelled: &AtomicBool::new(false),
                runtime_log: Some(&log),
                allow_tools: true,
            },
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &mut |_| {},
        );

        assert!(matches!(
            result,
            Err(ProviderError::HttpStatus { status: 500 })
        ));
        assert!(fs::read_to_string(log_path)
            .unwrap()
            .contains("event=model_request_failed"));
        assert_eq!(server.join().unwrap().len(), 1);

        let (endpoint, server) = test_support::serve(vec![test_support::sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ignored\"},\"finish_reason\":null}]}\n\n",
        )]);
        let provider = OpenAiCompatibleProvider::new(endpoint, "test-model".to_owned());
        let events = RefCell::new(Vec::new());
        run(
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &ToolContext::new(workspace()).unwrap(),
                context_window: None,
                cancelled: &AtomicBool::new(true),
                runtime_log: None,
                allow_tools: true,
            },
            vec![AgentMessage::User {
                content: "cancel".to_owned(),
            }],
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();
        assert_eq!(events.into_inner(), vec![AgentEvent::Cancelled]);
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn agent_run_groups_runtime_dependencies_without_changing_context_usage() {
        let provider = ContextUsageProvider;
        let events = RefCell::new(Vec::new());
        let tool_context = ToolContext::new(workspace()).unwrap();
        let cancelled = AtomicBool::new(false);

        run(
            AgentRun {
                provider: &provider,
                api_key: "key",
                tool_context: &tool_context,
                context_window: Some(100),
                cancelled: &cancelled,
                runtime_log: None,
                allow_tools: true,
            },
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            vec![AgentEvent::ContextUsage(50), AgentEvent::Finished]
        );
    }
}
