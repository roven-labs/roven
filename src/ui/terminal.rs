use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use serde_json::Value;

use crate::{
    agent::{self, AgentEvent, AgentMessage},
    credentials::{self, SecretStore},
    profiles::ProviderProfiles,
    provider::OpenAiCompatibleProvider,
    runtime_log::RuntimeLog,
    storage::{ConversationEvent, EventKind, ProjectStore, SessionMeta},
    tools::{RovenToolCall, RovenToolResult, ToolContext},
};

use super::{
    state::{AppState, Message, ResumeEntry, Role},
    view,
};

#[derive(Debug)]
enum WorkerEvent {
    Thought(String),
    Text(String),
    FunctionCallOutput {
        call: RovenToolCall,
        result: RovenToolResult,
    },
    Finished,
    Cancelled,
    Error(String),
}

const TOOL_USE_POLICY: &str = r#"Treat every request as read-only unless the user explicitly asks to prepare, register, add, modify, delete, or configure something. Call `prepare_project` only when the user explicitly asks to prepare, register, or add a project; never call it merely because a trusted workspace is available. For an explicit prepare/register/add request about the current trusted workspace, use `prepare_project` with {"path":"."}. When the user asks which Roven tools or capabilities are available, call `list_tools` with {} and rely on its returned names, descriptions, and input schemas. When the user asks for the current workspace path, call `list_directory` with {"path":"."} and report its `workspace_path` value verbatim; never report `.` as the human-facing path."#;

pub(crate) fn run(runtime_log: Option<RuntimeLog>) -> anyhow::Result<()> {
    log_event(runtime_log.as_ref(), "terminal_starting", "outcome=started");
    let mut guard = TerminalGuard::enter()?;
    let result = run_loop(runtime_log.as_ref());
    let restore_result = guard.restore();
    log_event(
        runtime_log.as_ref(),
        "terminal_stopped",
        if result.is_ok() && restore_result.is_ok() {
            "outcome=ok"
        } else {
            "outcome=error"
        },
    );
    result?;
    restore_result?;
    Ok(())
}

fn run_loop(runtime_log: Option<&RuntimeLog>) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = AppState::new();
    state.project_path = std::env::current_dir()?
        .canonicalize()?
        .to_string_lossy()
        .into_owned();
    log_event(
        runtime_log,
        "workspace_detected",
        &format!("path={}", state.project_path),
    );
    let (sender, receiver) = mpsc::channel();
    let mut store: Option<ProjectStore> = None;
    let mut session: Option<SessionMeta> = None;
    let mut project_instructions = String::new();
    let mut cancellation: Option<Arc<AtomicBool>> = None;
    let mut tool_context: Option<ToolContext> = None;

    loop {
        while let Ok(worker_event) = receiver.try_recv() {
            apply_worker_event(
                &mut state,
                store.as_ref(),
                session.as_ref(),
                runtime_log,
                worker_event,
            );
            if !state.running {
                cancellation = None;
            }
        }
        terminal.draw(|frame| view::draw(frame, &mut state))?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let event = event::read()?;
        if is_ctrl_c(&event) {
            log_event(runtime_log, "application_exit", "reason=ctrl_c");
            return Ok(());
        }
        if !state.trusted {
            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Down | KeyCode::Tab,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => state.toggle_trust_selection(),
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) if state.trust_yes_selected => {
                    let initialized =
                        ProjectStore::for_current_directory().inspect_err(|error| {
                            log_event(
                                runtime_log,
                                "workspace_storage_failed",
                                &format!("error={error}"),
                            );
                        })?;
                    project_instructions = read_project_instructions().unwrap_or_default();
                    store = Some(initialized);
                    let context = ToolContext::new(std::path::PathBuf::from(&state.project_path))
                        .inspect_err(|error| {
                        log_event(
                            runtime_log,
                            "tool_context_create_failed",
                            &format!("error={error}"),
                        );
                    })?;
                    tool_context = Some(context);
                    state.trusted = true;
                    log_event(runtime_log, "workspace_trusted", "outcome=granted");
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Enter | KeyCode::Esc,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => return Ok(()),
                _ => {}
            }
            continue;
        }
        if state.resume_entries.is_some() {
            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => state.close_resume(),
                Event::Key(KeyEvent {
                    code: KeyCode::Up, ..
                }) => state.select_previous_resume(),
                Event::Key(KeyEvent {
                    code: KeyCode::Down,
                    ..
                }) => state.select_next_resume(),
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    if let (Some(project_store), Some(id)) = (
                        store.as_ref(),
                        state.selected_resume_id().map(str::to_owned),
                    ) {
                        let events = project_store.events(&id)?;
                        state.replace_messages(events.into_iter().map(event_to_message).collect());
                        session = project_store
                            .list_sessions()?
                            .into_iter()
                            .find(|item| item.id == id);
                    }
                }
                _ => {}
            }
            continue;
        }
        if state.running {
            if matches!(
                event,
                Event::Key(KeyEvent {
                    code: KeyCode::Esc,
                    ..
                })
            ) {
                if let Some(flag) = &cancellation {
                    flag.store(true, Ordering::Relaxed);
                    log_event(runtime_log, "agent_cancellation_requested", "source=escape");
                }
                state.status = Some("Stopping agent...".to_owned());
            }
            continue;
        }
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) if modifiers.contains(KeyModifiers::ALT) => state.insert_newline(),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) => {
                let raw = state.input.clone();
                if raw.trim() == "/resume" {
                    state.input.clear();
                    let entries = store
                        .as_ref()
                        .expect("trusted store")
                        .list_sessions()?
                        .into_iter()
                        .map(|item| ResumeEntry {
                            id: item.id,
                            title: item.title,
                            updated_at_ms: item.updated_at_ms,
                        })
                        .collect();
                    state.open_resume(entries);
                    log_event(runtime_log, "resume_picker_opened", "outcome=ok");
                } else if state.submit() {
                    let user = state.last_user_message().unwrap_or_default().to_owned();
                    let project_store = store.as_ref().expect("trusted store");
                    if session.is_none() {
                        session =
                            Some(project_store.create_session(&user).inspect_err(|error| {
                                log_event(
                                    runtime_log,
                                    "session_create_failed",
                                    &format!("error={error}"),
                                );
                            })?);
                        log_event(runtime_log, "session_created", "outcome=ok");
                    }
                    let active_session = session.as_ref().expect("created session");
                    project_store
                        .append_event(
                            &active_session.id,
                            &ConversationEvent::message(EventKind::User, user.clone(), None),
                        )
                        .inspect_err(|error| {
                            log_event(
                                runtime_log,
                                "session_event_write_failed",
                                &format!("error={error}"),
                            );
                        })?;
                    let messages =
                        request_messages(project_store, active_session, &project_instructions)?;
                    state.start_agent();
                    let flag = Arc::new(AtomicBool::new(false));
                    cancellation = Some(flag.clone());
                    let tool_context = tool_context.as_ref().expect("trusted tool context").clone();
                    log_event(
                        runtime_log,
                        "agent_turn_started",
                        &format!("messages={}", messages.len()),
                    );
                    spawn_worker(
                        sender.clone(),
                        flag,
                        messages,
                        tool_context,
                        runtime_log.cloned(),
                    );
                }
            }
            Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                ..
            }) => state.backspace(),
            Event::Key(KeyEvent {
                code: KeyCode::Char(character),
                modifiers,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) if !modifiers.contains(KeyModifiers::CONTROL) => state.insert_char(character),
            Event::Key(KeyEvent {
                code: KeyCode::PageUp,
                ..
            })
            | Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..
            }) => state.scroll_up(3),
            Event::Key(KeyEvent {
                code: KeyCode::PageDown,
                ..
            })
            | Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..
            }) => state.scroll_down(3),
            _ => {}
        }
    }
}

fn spawn_worker(
    sender: mpsc::Sender<WorkerEvent>,
    cancelled: Arc<AtomicBool>,
    messages: Vec<AgentMessage>,
    tool_context: ToolContext,
    runtime_log: Option<RuntimeLog>,
) {
    thread::spawn(move || {
        log_event(runtime_log.as_ref(), "worker_started", "outcome=started");
        let result = (|| -> Result<(), String> {
            let profiles =
                ProviderProfiles::for_current_user().map_err(|error| error.to_string())?;
            let profile = profiles
                .default_profile()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "No default provider profile. Run `roven auth set`.".to_owned())?;
            let key = credentials::OsCredentialStore::for_profile_id(&profile.id)
                .get()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    format!(
                        "API key missing for provider profile `{}`. Run `roven auth set`.",
                        profile.name
                    )
                })?;
            let provider = OpenAiCompatibleProvider::new(profile.endpoint, profile.model);
            agent::run(
                &provider,
                &key,
                messages,
                &tool_context,
                &cancelled,
                runtime_log.as_ref(),
                &mut |event| {
                    let message = match event {
                        AgentEvent::Thought(thought) => WorkerEvent::Thought(thought),
                        AgentEvent::Text(text) => WorkerEvent::Text(text),
                        AgentEvent::ToolResult { call, result } => {
                            WorkerEvent::FunctionCallOutput { call, result }
                        }
                        AgentEvent::Finished => WorkerEvent::Finished,
                        AgentEvent::Cancelled => WorkerEvent::Cancelled,
                    };
                    let _ = sender.send(message);
                },
            )
            .map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            log_event(
                runtime_log.as_ref(),
                "worker_failed",
                &format!("error={error}"),
            );
            let _ = sender.send(WorkerEvent::Error(error));
        } else {
            log_event(runtime_log.as_ref(), "worker_finished", "outcome=ok");
        }
    });
}

fn apply_worker_event(
    state: &mut AppState,
    store: Option<&ProjectStore>,
    session: Option<&SessionMeta>,
    runtime_log: Option<&RuntimeLog>,
    event: WorkerEvent,
) {
    match event {
        WorkerEvent::Thought(thought) => state.append_thought(thought),
        WorkerEvent::Text(text) => state.append_agent_text(text),
        WorkerEvent::FunctionCallOutput { call, result } => {
            persist_function_call_output(store, session, &call, &result);
            state.activity(format!("{} completed", result.name));
        }
        WorkerEvent::Finished | WorkerEvent::Cancelled => {
            let stopped = matches!(event, WorkerEvent::Cancelled);
            state.finish_agent();
            persist_generation(
                state,
                store,
                session,
                if stopped {
                    EventKind::Cancelled
                } else {
                    EventKind::Assistant
                },
            );
            if stopped {
                state.activity("Agent stopped");
            }
            log_event(
                runtime_log,
                "agent_turn_finished",
                if stopped {
                    "outcome=cancelled"
                } else {
                    "outcome=ok"
                },
            );
        }
        WorkerEvent::Error(error) => {
            state.finish_agent();
            persist_generation(state, store, session, EventKind::Assistant);
            if let (Some(project_store), Some(active_session)) = (store, session) {
                let _ = project_store.append_event(
                    &active_session.id,
                    &ConversationEvent::message(EventKind::Error, error.clone(), None),
                );
            }
            state.agent_error(error);
            log_event(runtime_log, "agent_turn_finished", "outcome=error");
        }
    }
}

fn log_event(runtime_log: Option<&RuntimeLog>, event: &str, detail: &str) {
    if let Some(log) = runtime_log {
        log.record("terminal", event, detail);
    }
}

fn persist_function_call_output(
    store: Option<&ProjectStore>,
    session: Option<&SessionMeta>,
    call: &RovenToolCall,
    result: &RovenToolResult,
) {
    if let (Some(project_store), Some(active_session)) = (store, session) {
        let _ = project_store.append_event(
            &active_session.id,
            &ConversationEvent::function_call_output(
                call.id.clone(),
                call.name.clone(),
                call.arguments.clone(),
                result.result.clone(),
            ),
        );
    }
}

fn persist_generation(
    state: &AppState,
    store: Option<&ProjectStore>,
    session: Option<&SessionMeta>,
    kind: EventKind,
) {
    if let (Some(project_store), Some(active_session)) = (store, session) {
        for message in state.generated_messages() {
            let event_kind = match message.role {
                Role::Thought => EventKind::Thought,
                Role::Roven => kind.clone(),
                Role::User | Role::Activity => continue,
            };
            if message.content.is_empty() {
                continue;
            }
            let _ = project_store.append_event(
                &active_session.id,
                &ConversationEvent::message(
                    event_kind,
                    message.content.clone(),
                    message.duration_ms,
                ),
            );
        }
    }
}

fn request_messages(
    store: &ProjectStore,
    session: &SessionMeta,
    project_instructions: &str,
) -> anyhow::Result<Vec<AgentMessage>> {
    let mut messages = vec![AgentMessage::System {
        content: format!(
            "You are Roven, a concise project assistant. The Roven harness authorizes and executes tools; do not claim a tool ran unless its result confirms it.\n\n{}{}",
            TOOL_USE_POLICY,
            if project_instructions.is_empty() {
                String::new()
            } else {
                format!("\n\nProject instructions:\n{project_instructions}")
            }
        ),
    }];
    let mut pending_reasoning: Option<String> = None;
    let events = store.events(&session.id)?;
    let mut index = 0;
    while index < events.len() {
        let event = events[index].clone();
        match event.kind {
            EventKind::User => {
                pending_reasoning = None;
                messages.push(AgentMessage::User {
                    content: event.content,
                });
            }
            EventKind::Thought => {
                pending_reasoning = Some(match pending_reasoning {
                    Some(mut current) => {
                        current.push_str(&event.content);
                        current
                    }
                    None => event.content,
                });
            }
            EventKind::Assistant | EventKind::Cancelled => {
                messages.push(AgentMessage::Assistant {
                    content: event.content,
                    reasoning: pending_reasoning.take(),
                    tool_calls: Vec::new(),
                });
            }
            EventKind::FunctionCallOutput => {
                let mut calls = Vec::new();
                let mut results = Vec::new();
                while index < events.len() {
                    let function_event = events[index].clone();
                    if function_event.kind != EventKind::FunctionCallOutput {
                        break;
                    }
                    if let (
                        Some(tool_call_id),
                        Some(tool_name),
                        Some(tool_input),
                        Some(tool_output),
                    ) = (
                        function_event.tool_call_id,
                        function_event.tool_name,
                        function_event.tool_input,
                        function_event.tool_output,
                    ) {
                        calls.push(RovenToolCall {
                            id: tool_call_id.clone(),
                            name: tool_name.clone(),
                            arguments: tool_input,
                        });
                        results.push(RovenToolResult {
                            tool_call_id,
                            name: tool_name,
                            result: tool_output,
                        });
                    }
                    index += 1;
                }
                if !calls.is_empty() {
                    messages.push(AgentMessage::Assistant {
                        content: String::new(),
                        reasoning: pending_reasoning.take(),
                        tool_calls: calls,
                    });
                    messages.extend(
                        results
                            .into_iter()
                            .map(|result| AgentMessage::Tool { result }),
                    );
                }
                continue;
            }
            EventKind::Error => {}
        }
        index += 1;
    }
    Ok(messages)
}

fn read_project_instructions() -> io::Result<String> {
    let path = std::env::current_dir()?.canonicalize()?.join("ROVEN.md");
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error),
    };
    if metadata.len() > 50 * 1024 {
        return Ok(String::new());
    }
    std::fs::read_to_string(path)
}

fn event_to_message(event: ConversationEvent) -> Message {
    let role = match event.kind {
        EventKind::User => Role::User,
        EventKind::Thought => Role::Thought,
        EventKind::FunctionCallOutput => Role::Activity,
        EventKind::Assistant | EventKind::Error | EventKind::Cancelled => Role::Roven,
    };
    match event.kind {
        EventKind::FunctionCallOutput => Message::tool(
            event.tool_name.unwrap_or_else(|| "unknown".to_owned()),
            event.tool_input.unwrap_or(Value::Null),
            event.tool_output.unwrap_or(Value::Null),
        ),
        _ => Message::text(role, event.content, event.duration_ms),
    }
}

fn is_ctrl_c(event: &Event) -> bool {
    matches!(event, Event::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, .. }) if modifiers.contains(KeyModifiers::CONTROL))
}

struct TerminalGuard {
    active: bool,
}
impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            Hide,
            Clear(ClearType::All)
        ) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(Self { active: true })
    }
    fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        let mut stdout = io::stdout();
        let terminal_result = execute!(stdout, Show, DisableMouseCapture, LeaveAlternateScreen);
        let raw_mode_result = terminal::disable_raw_mode();
        self.active = false;
        terminal_result.and(raw_mode_result)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        storage::{ConversationEvent, EventKind, ProjectStore},
        ui::state::Role,
    };

    fn temp_root(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).expect("temporary root should exist");
        path
    }

    #[test]
    fn resumed_history_returns_saved_thought_as_assistant_reasoning() {
        let data = temp_root("reasoning-data");
        let project = temp_root("reasoning-project");
        let store = ProjectStore::for_project(&data, &project).unwrap();
        let session = store.create_session("Explain the failure").unwrap();
        for event in [
            ConversationEvent::message(EventKind::User, "Explain the failure".to_owned(), None),
            ConversationEvent::message(
                EventKind::Thought,
                "I should inspect the error.".to_owned(),
                Some(757),
            ),
            ConversationEvent::message(
                EventKind::Assistant,
                "The request was rate limited.".to_owned(),
                None,
            ),
        ] {
            store.append_event(&session.id, &event).unwrap();
        }

        let messages = super::request_messages(&store, &session, "").unwrap();

        assert!(matches!(
            &messages[0],
            crate::agent::AgentMessage::System { content }
                if content.contains("Treat every request as read-only")
                    && content.contains("Call `prepare_project` only when the user explicitly asks")
                    && content.contains("report its `workspace_path` value verbatim")
        ));
        assert!(matches!(
            &messages[2],
            crate::agent::AgentMessage::Assistant {
                content,
                reasoning: Some(reasoning),
                tool_calls,
            } if content == "The request was rate limited."
                && reasoning == "I should inspect the error."
                && tool_calls.is_empty()
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn resumed_history_rebuilds_function_calls_and_outputs() {
        let data = temp_root("function-data");
        let project = temp_root("function-project");
        let store = ProjectStore::for_project(&data, &project).unwrap();
        let session = store.create_session("Inspect the workspace").unwrap();
        store
            .append_event(
                &session.id,
                &ConversationEvent::message(
                    EventKind::User,
                    "Inspect the workspace".to_owned(),
                    None,
                ),
            )
            .unwrap();
        for (id, name, input, output) in [
            (
                "call-1",
                "list_directory",
                serde_json::json!({"path": "."}),
                serde_json::json!({"status": "ok"}),
            ),
            (
                "call-2",
                "list_tools",
                serde_json::json!({}),
                serde_json::json!({"status": "ok"}),
            ),
        ] {
            store
                .append_event(
                    &session.id,
                    &ConversationEvent::function_call_output(
                        id.to_owned(),
                        name.to_owned(),
                        input,
                        output,
                    ),
                )
                .unwrap();
        }

        let messages = super::request_messages(&store, &session, "").unwrap();
        assert!(matches!(
            &messages[2],
            crate::agent::AgentMessage::Assistant { tool_calls, .. }
                if tool_calls.len() == 2
                    && tool_calls[0].name == "list_directory"
                    && tool_calls[1].name == "list_tools"
        ));
        assert!(matches!(
            &messages[3],
            crate::agent::AgentMessage::Tool { result }
                if result.tool_call_id == "call-1" && result.name == "list_directory"
        ));
        assert!(matches!(
            &messages[4],
            crate::agent::AgentMessage::Tool { result }
                if result.tool_call_id == "call-2" && result.name == "list_tools"
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn resumed_function_call_output_is_visible_in_history() {
        let data = temp_root("display-data");
        let project = temp_root("display-project");
        let store = ProjectStore::for_project(&data, &project).unwrap();
        let session = store.create_session("Inspect the workspace").unwrap();
        let event = ConversationEvent::function_call_output(
            "call-1".to_owned(),
            "list_directory".to_owned(),
            serde_json::json!({"path": "."}),
            serde_json::json!({"status": "ok"}),
        );
        store.append_event(&session.id, &event).unwrap();

        let message = super::event_to_message(event);
        assert_eq!(message.role, Role::Activity);
        assert!(matches!(
            message.kind,
            crate::ui::state::MessageKind::Tool { ref name, ref input, ref output }
                if name == "list_directory"
                    && input["path"] == "."
                    && output["status"] == "ok"
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }
}
