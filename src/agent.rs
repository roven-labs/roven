//! Provider-independent agent lifecycle and tool orchestration.

use std::sync::atomic::AtomicBool;

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
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<(), P::Error> {
    loop {
        let request = AgentRequest::new(messages.clone());
        let mut response_content = String::new();
        let mut response_reasoning = String::new();
        let mut tool_calls = None;
        let mut finished = false;
        let mut was_cancelled = false;

        provider.stream(api_key, &request, cancelled, &mut |event| match event {
            ProviderEvent::Thought(thought) => {
                response_reasoning.push_str(&thought);
                emit(AgentEvent::Thought(thought));
            }
            ProviderEvent::Text(text) => {
                response_content.push_str(&text);
                emit(AgentEvent::Text(text));
            }
            ProviderEvent::ToolCalls(calls) => tool_calls = Some(calls),
            ProviderEvent::Finished => finished = true,
            ProviderEvent::Cancelled => was_cancelled = true,
        })?;

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
                let result = dispatch(context, call);
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, sync::atomic::AtomicBool};

    use crate::tools::ToolContext;

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
            assert_eq!(request.tools.len(), 2);
            assert_eq!(request.tools[0].name, "prepare_project");
            assert_eq!(request.tools[1].name, "list_directory");
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
}
