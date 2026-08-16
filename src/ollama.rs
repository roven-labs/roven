use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde_json::{Value, json};
use url::Url;

use crate::{
    agent::{AgentMessage, AgentRequest, ProviderEvent},
    model_catalog::validate_model,
    provider::{Provider, ProviderError, request_error, response_error, stream_read_error},
    runtime_log::RuntimeLog,
    storage::now_ms,
    tools::{RovenToolCall, RovenToolDefinition},
};

const FAILURE_LOG_NAME: &str = "ollama-stream-failures.log";
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

pub(crate) fn is_endpoint(endpoint: &str) -> bool {
    let Ok(endpoint) = Url::parse(endpoint) else {
        return false;
    };
    endpoint.scheme() == "https" && endpoint.host_str() == Some("ollama.com")
}

pub(crate) fn is_native_endpoint(endpoint: &str) -> bool {
    let Ok(endpoint) = Url::parse(endpoint) else {
        return false;
    };
    is_endpoint(endpoint.as_str()) && endpoint.path().trim_end_matches('/') == "/api/chat"
}

pub(crate) struct OllamaProvider {
    endpoint: String,
    model: String,
}

impl OllamaProvider {
    pub(crate) fn new(endpoint: String, model: String) -> Self {
        Self { endpoint, model }
    }
}

impl Provider for OllamaProvider {
    fn stream(
        &self,
        api_key: &str,
        request: &AgentRequest,
        cancelled: &AtomicBool,
        runtime_log: Option<&RuntimeLog>,
        emit: &mut dyn FnMut(ProviderEvent),
    ) -> Result<(), ProviderError> {
        let payload =
            serde_json::to_string(&native_request(&self.model, request)).map_err(|error| {
                ProviderError::diagnostic(
                    "encode",
                    "json",
                    format!("could not encode the Ollama request: {error}"),
                )
            })?;
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let mut response = agent
            .post(&self.endpoint)
            .header("Authorization", &format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/x-ndjson")
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
        let mut capture = StreamCapture::new(&self.endpoint, &self.model);
        let mut tool_calls = Vec::new();
        let mut next_tool_call_id = 0usize;
        for line in reader.lines() {
            if cancelled.load(Ordering::Relaxed) {
                emit(ProviderEvent::Cancelled);
                return Ok(());
            }
            let line = line.map_err(stream_read_error)?;
            capture.record_line(&line);
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => {
                    let error = ProviderError::diagnostic(
                        "stream",
                        "json",
                        "invalid Ollama NDJSON stream event",
                    );
                    record_failure(&capture, runtime_log, &error);
                    return Err(error);
                }
            };
            if value.get("error").is_some() {
                let error = ProviderError::diagnostic(
                    "stream",
                    "remote_error",
                    "Ollama returned a streaming error",
                );
                record_failure(&capture, runtime_log, &error);
                return Err(error);
            }

            let message = value.get("message");
            if let Some(thinking) = message
                .and_then(|message| message.get("thinking"))
                .and_then(Value::as_str)
                .filter(|thinking| !thinking.is_empty())
            {
                emit(ProviderEvent::Thought(thinking.to_owned()));
            }
            if let Some(content) = message
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .filter(|content| !content.is_empty())
            {
                emit(ProviderEvent::Text(content.to_owned()));
            }
            if let Some(chunks) = message
                .and_then(|message| message.get("tool_calls"))
                .and_then(Value::as_array)
            {
                for chunk in chunks {
                    let function = match chunk.get("function") {
                        Some(function) => function,
                        None => {
                            let error = ProviderError::diagnostic(
                                "tool_calls",
                                "invalid_tool_call",
                                "Ollama tool call has no function",
                            );
                            record_failure(&capture, runtime_log, &error);
                            return Err(error);
                        }
                    };
                    let name = match function
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())
                    {
                        Some(name) => name,
                        None => {
                            let error = ProviderError::diagnostic(
                                "tool_calls",
                                "invalid_tool_call",
                                "Ollama tool call has no function name",
                            );
                            record_failure(&capture, runtime_log, &error);
                            return Err(error);
                        }
                    };
                    let arguments = match function
                        .get("arguments")
                        .filter(|arguments| arguments.is_object())
                    {
                        Some(arguments) => arguments.clone(),
                        None => {
                            let error = ProviderError::diagnostic(
                                "tool_calls",
                                "invalid_arguments",
                                "Ollama tool call arguments must be a JSON object",
                            );
                            record_failure(&capture, runtime_log, &error);
                            return Err(error);
                        }
                    };
                    tool_calls.push(RovenToolCall {
                        id: format!("ollama-call-{next_tool_call_id}"),
                        name: name.to_owned(),
                        arguments,
                    });
                    next_tool_call_id = next_tool_call_id.saturating_add(1);
                }
            }

            if value.get("done").and_then(Value::as_bool) == Some(true) {
                if let Some(prompt_tokens) = value
                    .get("prompt_eval_count")
                    .and_then(Value::as_u64)
                    .and_then(|tokens| usize::try_from(tokens).ok())
                {
                    emit(ProviderEvent::ContextUsage(prompt_tokens));
                }
                if !tool_calls.is_empty() {
                    emit(ProviderEvent::ToolCalls(std::mem::take(&mut tool_calls)));
                }
                emit(ProviderEvent::Finished);
                return Ok(());
            }
        }

        let error = ProviderError::diagnostic(
            "stream",
            "unexpected_eof",
            "Ollama closed the NDJSON stream before done=true",
        );
        record_failure(&capture, runtime_log, &error);
        Err(error)
    }
}

pub(crate) fn context_window(api_key: &str, endpoint: &str, model: &str) -> Option<usize> {
    if !is_native_endpoint(endpoint) || !validate_model(endpoint, model) {
        return None;
    }
    let url = api_url(endpoint, "/api/show")?;
    let payload = serde_json::to_string(&json!({"model": model, "verbose": true})).ok()?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .build(),
    );
    let mut response = agent
        .post(url)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send(payload)
        .ok()?;
    if !(200..300).contains(&response.status().as_u16()) {
        return None;
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_string()
        .ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    value
        .get("model_info")
        .and_then(Value::as_object)
        .and_then(|model_info| {
            model_info.iter().find_map(|(key, value)| {
                key.ends_with(".context_length")
                    .then(|| value.as_u64())
                    .flatten()
                    .filter(|tokens| *tokens > 0)
                    .and_then(|tokens| usize::try_from(tokens).ok())
            })
        })
}

fn api_url(endpoint: &str, path: &str) -> Option<String> {
    let mut url = Url::parse(endpoint).ok()?;
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn native_request(model: &str, request: &AgentRequest) -> Value {
    json!({
        "model": model,
        "messages": request.messages.iter().map(native_message).collect::<Vec<_>>(),
        "tools": request.tools.iter().map(native_tool).collect::<Vec<_>>(),
        "stream": true,
        "think": true,
    })
}

fn native_message(message: &AgentMessage) -> Value {
    match message {
        AgentMessage::System { content } => json!({"role": "system", "content": content}),
        AgentMessage::User { content } => json!({"role": "user", "content": content}),
        AgentMessage::Assistant {
            content,
            reasoning,
            tool_calls,
        } => {
            let mut message = json!({"role": "assistant", "content": content});
            if let Some(reasoning) = reasoning {
                message["thinking"] = json!(reasoning);
            }
            if !tool_calls.is_empty() {
                message["tool_calls"] = json!(
                    tool_calls
                        .iter()
                        .enumerate()
                        .map(|(index, call)| json!({
                            "type": "function",
                            "function": {
                                "index": index,
                                "name": call.name,
                                "arguments": call.arguments,
                            }
                        }))
                        .collect::<Vec<_>>()
                );
            }
            message
        }
        AgentMessage::Tool { result } => json!({
            "role": "tool",
            "tool_name": result.name,
            "content": serde_json::to_string(&result.result).unwrap_or_default(),
        }),
    }
}

fn native_tool(tool: &RovenToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

pub(crate) fn record_failure(
    capture: &StreamCapture,
    runtime_log: Option<&RuntimeLog>,
    error: &ProviderError,
) {
    match capture.write_failure(runtime_log, &error.to_string()) {
        Ok(Some(path)) => {
            if let Some(log) = runtime_log {
                log.record(
                    "ollama",
                    "stream_capture_written",
                    &format!("path={}", path.display()),
                );
            }
        }
        Ok(None) => {}
        Err(capture_error) => {
            if let Some(log) = runtime_log {
                log.record(
                    "ollama",
                    "stream_capture_failed",
                    &format!("error={capture_error}"),
                );
            }
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ToolCallAccumulator {
    calls: Vec<ToolCallParts>,
    by_id: HashMap<String, usize>,
    by_index: HashMap<usize, usize>,
}

#[derive(Debug, Default)]
struct ToolCallParts {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    pub(crate) fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    pub(crate) fn append_line(&mut self, line: &str) -> Result<(), ProviderError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let value: Value = serde_json::from_str(data).map_err(|_| {
            ProviderError::diagnostic("tool_calls", "json", "invalid JSON tool-call event")
        })?;
        let calls = value
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("tool_calls"))
            .and_then(Value::as_array);
        let Some(calls) = calls else {
            return Ok(());
        };
        for call in calls {
            let index = call.get("index").and_then(Value::as_u64).ok_or_else(|| {
                ProviderError::diagnostic(
                    "tool_calls",
                    "invalid_tool_call",
                    "tool call has no index",
                )
            })? as usize;
            let id = call.get("id").and_then(Value::as_str);
            let position = if let Some(id) = id {
                if let Some(position) = self.by_id.get(id).copied() {
                    position
                } else {
                    let position = self.calls.len();
                    self.calls.push(ToolCallParts {
                        id: id.to_owned(),
                        ..ToolCallParts::default()
                    });
                    self.by_id.insert(id.to_owned(), position);
                    position
                }
            } else if let Some(position) = self.by_index.get(&index).copied() {
                position
            } else {
                let position = self.calls.len();
                self.calls.push(ToolCallParts::default());
                position
            };
            self.by_index.insert(index, position);
            let parts = &mut self.calls[position];
            if let Some(name) = call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            {
                parts.name.push_str(name);
            }
            if let Some(arguments) = call
                .get("function")
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
            {
                parts.arguments.push_str(arguments);
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<Vec<RovenToolCall>, ProviderError> {
        self.calls
            .into_iter()
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
}

pub(crate) struct StreamCapture {
    enabled: bool,
    endpoint: String,
    model: String,
    lines: Vec<u8>,
    truncated: bool,
}

impl StreamCapture {
    pub(crate) fn new(endpoint: &str, model: &str) -> Self {
        Self {
            enabled: is_endpoint(endpoint),
            endpoint: endpoint.to_owned(),
            model: model.to_owned(),
            lines: Vec::new(),
            truncated: false,
        }
    }

    pub(crate) fn record_line(&mut self, line: &str) {
        if !self.enabled || self.truncated {
            return;
        }
        let required = line.len().saturating_add(1);
        if self.lines.len().saturating_add(required) > MAX_CAPTURE_BYTES {
            self.truncated = true;
            return;
        }
        self.lines.extend_from_slice(line.as_bytes());
        self.lines.push(b'\n');
    }

    pub(crate) fn write_failure(
        &self,
        runtime_log: Option<&RuntimeLog>,
        error: &str,
    ) -> io::Result<Option<PathBuf>> {
        if !self.enabled {
            return Ok(None);
        }
        let Some(runtime_log) = runtime_log else {
            return Ok(None);
        };
        let Some(parent) = runtime_log.path().parent() else {
            return Ok(None);
        };
        let path = parent.join(FAILURE_LOG_NAME);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(b"\n=== Ollama stream failure ===\n")?;
        serde_json::to_writer(
            &mut file,
            &json!({
                "timestamp_ms": now_ms(),
                "endpoint": self.endpoint,
                "model": self.model,
                "error": error,
                "truncated": self.truncated,
            }),
        )?;
        file.write_all(b"\n--- raw Ollama stream lines ---\n")?;
        file.write_all(&self.lines)?;
        if self.truncated {
            file.write_all(b"[capture truncated at 1048576 bytes]\n")?;
        }
        file.write_all(b"--- end Ollama stream failure ---\n")?;
        file.sync_data()?;
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::atomic::AtomicBool};

    use serde_json::json;

    use super::{OllamaProvider, StreamCapture, ToolCallAccumulator, api_url, is_native_endpoint};
    use crate::runtime_log::RuntimeLog;
    use crate::{
        agent::{AgentMessage, AgentRequest, ProviderEvent},
        provider::{Provider, test_support},
    };

    #[test]
    fn native_endpoint_uses_the_documented_ollama_routes() {
        assert!(is_native_endpoint("https://ollama.com/api/chat"));
        assert!(!is_native_endpoint(
            "https://ollama.com/v1/chat/completions"
        ));
        assert_eq!(
            api_url("https://ollama.com/api/chat", "/api/show").as_deref(),
            Some("https://ollama.com/api/show")
        );
    }

    #[test]
    fn native_stream_accumulates_tool_calls_and_reports_prompt_usage() {
        let body = concat!(
            r#"{"model":"test-model","message":{"thinking":"checking","content":"","tool_calls":[{"type":"function","function":{"index":0,"name":"list_directory","arguments":{"path":"."}}}]},"done":false}"#,
            "\n",
            r#"{"model":"test-model","message":{"thinking":"","content":"done"},"done":false}"#,
            "\n",
            r#"{"model":"test-model","done":true,"prompt_eval_count":37,"eval_count":4}"#,
            "\n"
        );
        let (endpoint, handle) = test_support::serve(vec![test_support::response(
            "200 OK",
            &[("Content-Type", "application/x-ndjson")],
            body,
        )]);
        let provider = OllamaProvider::new(endpoint, "test-model".to_owned());
        let request = AgentRequest::new(vec![AgentMessage::User {
            content: "list the files".to_owned(),
        }]);
        let mut events = Vec::new();

        provider
            .stream(
                "test-key",
                &request,
                &AtomicBool::new(false),
                None,
                &mut |event| events.push(event),
            )
            .unwrap();

        let sent_request = handle.join().unwrap().pop().unwrap();
        let sent_request: serde_json::Value = serde_json::from_str(&sent_request).unwrap();
        assert_eq!(sent_request["model"], "test-model");
        assert_eq!(sent_request["stream"], true);
        assert_eq!(sent_request["messages"][0]["role"], "user");

        assert!(matches!(events[0], ProviderEvent::Thought(ref text) if text == "checking"));
        assert!(matches!(events[1], ProviderEvent::Text(ref text) if text == "done"));
        assert_eq!(events[2], ProviderEvent::ContextUsage(37));
        match &events[3] {
            ProviderEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_directory");
                assert_eq!(calls[0].arguments, json!({"path": "."}));
            }
            event => panic!("expected tool calls, got {event:?}"),
        }
        assert_eq!(events[4], ProviderEvent::Finished);
    }

    #[test]
    fn failure_capture_preserves_raw_ollama_sse_lines() {
        let root =
            std::env::temp_dir().join(format!("roven-ollama-capture-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&root).unwrap();
        let runtime_log = RuntimeLog::for_file(root.join("log.md")).unwrap();
        let raw_line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\\"path\\":\\"."}}}]}}]}"#;
        let mut capture =
            StreamCapture::new("https://ollama.com/v1/chat/completions", "minimax-m3:cloud");
        capture.record_line(raw_line);

        let path = capture
            .write_failure(Some(&runtime_log), "tool call arguments are not valid JSON")
            .unwrap()
            .unwrap();
        let contents = fs::read_to_string(path).unwrap();

        assert!(contents.contains(raw_line));
        assert!(contents.contains("minimax-m3:cloud"));
        assert!(!contents.contains("Authorization"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_ollama_endpoints_do_not_create_failure_captures() {
        let root =
            std::env::temp_dir().join(format!("roven-openrouter-capture-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&root).unwrap();
        let runtime_log = RuntimeLog::for_file(root.join("log.md")).unwrap();
        let mut capture = StreamCapture::new(
            "https://openrouter.ai/api/v1/chat/completions",
            "openai/gpt-oss-20b",
        );
        capture.record_line("data: {\"choices\":[]}");

        assert!(
            capture
                .write_failure(Some(&runtime_log), "failure")
                .unwrap()
                .is_none()
        );
        assert!(!root.join("ollama-stream-failures.log").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn groups_parallel_calls_by_id_when_ollama_reuses_index_zero() {
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"call_one","index":0,"function":{"name":"list_directory","arguments":"{\"path\":\"rag\"}"}},{"id":"call_two","index":0,"function":{"name":"read_file","arguments":"{\"path\":\".gitignore\"}"}},{"id":"call_three","index":0,"function":{"name":"read_file","arguments":"{\"path\":\"run_streamlit_ui_command.txt\"}"}}]}}]}"#;
        let mut accumulator = ToolCallAccumulator::default();
        accumulator.append_line(line).unwrap();

        let calls = accumulator.finish().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].id, "call_one");
        assert_eq!(calls[0].arguments, json!({"path": "rag"}));
        assert_eq!(calls[1].id, "call_two");
        assert_eq!(calls[1].arguments, json!({"path": ".gitignore"}));
        assert_eq!(calls[2].id, "call_three");
        assert_eq!(
            calls[2].arguments,
            json!({"path": "run_streamlit_ui_command.txt"})
        );
    }
}
