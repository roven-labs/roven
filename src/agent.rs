//! Provider-independent agent lifecycle and tool orchestration.

use std::sync::atomic::AtomicBool;

use crate::runtime_log::RuntimeLog;
use crate::tools::{
    RovenToolCall, RovenToolDefinition, RovenToolResult, ToolContext, definitions, dispatch,
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

pub(crate) trait ModelProvider {
    type Error: std::error::Error;

    fn stream(
        &self,
        api_key: &str,
        request: &AgentRequest,
        cancelled: &AtomicBool,
        emit: &mut dyn FnMut(ProviderEvent),
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AgentEvent {
    Thought(String),
    Text(String),
    ToolResult(RovenToolResult),
    Finished,
    Cancelled,
}

/// Continue the same user turn until the provider produces a final response.
/// Tool execution and authorization stay here; adapters only transport events.
pub(crate) fn run<P: ModelProvider>(
    provider: &P,
    api_key: &str,
    mut messages: Vec<AgentMessage>,
    context: &ToolContext,
    cancelled: &AtomicBool,
    runtime_log: Option<&RuntimeLog>,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<(), P::Error> {
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
                let result = dispatch(context, call);
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
                emit(AgentEvent::ToolResult(result.clone()));
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

    use crate::{runtime_log::RuntimeLog, tools::ToolContext};

    use super::{AgentEvent, AgentMessage, AgentRequest, ModelProvider, ProviderEvent, run};

    #[derive(Debug)]
    struct TestError;

    impl std::fmt::Display for TestError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("test provider error")
        }
    }

    impl std::error::Error for TestError {}

    struct FinalProvider;

    impl ModelProvider for FinalProvider {
        type Error = TestError;

        fn stream(
            &self,
            _: &str,
            request: &AgentRequest,
            _: &AtomicBool,
            emit: &mut dyn FnMut(ProviderEvent),
        ) -> Result<(), Self::Error> {
            assert_eq!(request.tools.len(), 3);
            assert_eq!(request.tools[0].name, "prepare_project");
            assert_eq!(request.tools[1].name, "list_directory");
            assert_eq!(request.tools[2].name, "list_tools");
            emit(ProviderEvent::Text("done".to_owned()));
            emit(ProviderEvent::Finished);
            Ok(())
        }
    }

    #[test]
    fn final_provider_response_ends_the_turn_without_a_tool_cycle_limit() {
        let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
        let events = RefCell::new(Vec::new());

        run(
            &FinalProvider,
            "key",
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &ToolContext::new(workspace).unwrap(),
            &AtomicBool::new(false),
            None,
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            vec![AgentEvent::Text("done".to_owned()), AgentEvent::Finished]
        );
    }

    struct ToolThenFinalProvider {
        requests: RefCell<usize>,
    }

    impl ModelProvider for ToolThenFinalProvider {
        type Error = TestError;

        fn stream(
            &self,
            _: &str,
            request: &AgentRequest,
            _: &AtomicBool,
            emit: &mut dyn FnMut(ProviderEvent),
        ) -> Result<(), Self::Error> {
            let request_number = *self.requests.borrow();
            *self.requests.borrow_mut() += 1;
            if request_number == 0 {
                emit(ProviderEvent::ToolCalls(vec![
                    crate::tools::RovenToolCall {
                        id: "call_prepare".to_owned(),
                        name: "prepare_project".to_owned(),
                        arguments: serde_json::json!({ "path": "does-not-exist" }),
                    },
                ]));
            } else {
                assert!(matches!(
                    request.messages.last(),
                    Some(AgentMessage::Tool { result })
                        if result.result["reason"] == "invalid_path"
                ));
                emit(ProviderEvent::Text(
                    "registration needs a valid path".to_owned(),
                ));
                emit(ProviderEvent::Finished);
            }
            Ok(())
        }
    }

    #[test]
    fn tool_results_return_to_the_generic_agent_loop_before_final_response() {
        let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
        let provider = ToolThenFinalProvider {
            requests: RefCell::new(0),
        };
        let events = RefCell::new(Vec::new());

        run(
            &provider,
            "key",
            vec![AgentMessage::User {
                content: "register this".to_owned(),
            }],
            &ToolContext::new(workspace).unwrap(),
            &AtomicBool::new(false),
            None,
            &mut |event| events.borrow_mut().push(event),
        )
        .unwrap();

        assert_eq!(*provider.requests.borrow(), 2);
        assert!(matches!(
            events.borrow().first(),
            Some(AgentEvent::ToolResult(result)) if result.name == "prepare_project"
        ));
        assert!(matches!(events.borrow().last(), Some(AgentEvent::Finished)));
    }

    struct FailingProvider;

    impl ModelProvider for FailingProvider {
        type Error = TestError;

        fn stream(
            &self,
            _: &str,
            _: &AgentRequest,
            _: &AtomicBool,
            _: &mut dyn FnMut(ProviderEvent),
        ) -> Result<(), Self::Error> {
            Err(TestError)
        }
    }

    #[test]
    fn provider_failures_are_written_to_the_runtime_log() {
        let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
        let log_path =
            std::env::temp_dir().join(format!("roven-agent-log-{}.md", uuid::Uuid::now_v7()));
        let log = RuntimeLog::for_file(&log_path).unwrap();

        let result = run(
            &FailingProvider,
            "key",
            vec![AgentMessage::User {
                content: "hello".to_owned(),
            }],
            &ToolContext::new(workspace).unwrap(),
            &AtomicBool::new(false),
            Some(&log),
            &mut |_| {},
        );

        assert!(result.is_err());
        let contents = fs::read_to_string(log_path).unwrap();
        assert!(contents.contains("component=agent"));
        assert!(contents.contains("event=model_request_started"));
        assert!(contents.contains("event=model_request_failed"));
        assert!(contents.contains("error=test provider error"));
    }
}
