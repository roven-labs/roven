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

use crate::{
    agent::{self, AgentEvent, AgentMessage},
    credentials,
    provider::OpenRouterProvider,
    storage::{ConversationEvent, EventKind, ProjectStore, SessionMeta, now_ms},
    tools::ToolContext,
};

use super::{
    state::{AppState, Message, ResumeEntry, Role},
    view,
};

#[derive(Debug)]
enum WorkerEvent {
    Thought(String),
    Text(String),
    Finished,
    Cancelled,
    Activity(String),
    Error(String),
}

const TOOL_USE_POLICY: &str = "The current trusted workspace is the project the user means by `this project`, `these projects`, `this workspace`, or an unqualified request to add/register the project. Do not ask for a path in those cases: call `prepare_project` with `{\"path\":\".\"}`. When the user asks for the current workspace path, call `list_directory` with `{\"path\":\".\"}` and report its `workspace_path` value verbatim; never report `.` as the human-facing path. When the user asks to inspect, explain, or diagram the current project's structure, first call `list_directory` with `{\"path\":\".\"}` and base the response only on its returned entries. Call `list_directory` again for a named subdirectory only when needed; it never recurses automatically.";

pub(crate) fn run() -> anyhow::Result<()> {
    let mut guard = TerminalGuard::enter()?;
    let result = run_loop();
    let restore_result = guard.restore();
    result?;
    restore_result?;
    Ok(())
}

fn run_loop() -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = AppState::new();
    state.project_path = std::env::current_dir()?
        .canonicalize()?
        .to_string_lossy()
        .into_owned();
    let (sender, receiver) = mpsc::channel();
    let mut store: Option<ProjectStore> = None;
    let mut session: Option<SessionMeta> = None;
    let mut project_instructions = String::new();
    let mut cancellation: Option<Arc<AtomicBool>> = None;

    loop {
        while let Ok(worker_event) = receiver.try_recv() {
            apply_worker_event(&mut state, store.as_ref(), session.as_ref(), worker_event);
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
                    let initialized = ProjectStore::for_current_directory()?;
                    project_instructions = read_project_instructions().unwrap_or_default();
                    store = Some(initialized);
                    state.trusted = true;
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
                } else if state.submit() {
                    let user = state.last_user_message().unwrap_or_default().to_owned();
                    let project_store = store.as_ref().expect("trusted store");
                    if session.is_none() {
                        session = Some(project_store.create_session(&user)?);
                    }
                    let active_session = session.as_ref().expect("created session");
                    project_store.append_event(
                        &active_session.id,
                        &ConversationEvent {
                            kind: EventKind::User,
                            content: user.clone(),
                            duration_ms: None,
                            created_at_ms: now_ms(),
                        },
                    )?;
                    let messages =
                        request_messages(project_store, active_session, &project_instructions)?;
                    state.start_agent();
                    let flag = Arc::new(AtomicBool::new(false));
                    cancellation = Some(flag.clone());
                    let tool_context =
                        ToolContext::new(std::path::PathBuf::from(&state.project_path))?;
                    spawn_worker(sender.clone(), flag, messages, tool_context);
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
) {
    thread::spawn(move || {
        let result = match credentials::load_openrouter_api_key() {
            Ok(Some(key)) => agent::run(
                &OpenRouterProvider,
                &key,
                messages,
                &tool_context,
                &cancelled,
                &mut |event| {
                    let message = match event {
                        AgentEvent::Thought(thought) => WorkerEvent::Thought(thought),
                        AgentEvent::Text(text) => WorkerEvent::Text(text),
                        AgentEvent::ToolResult(result) => {
                            WorkerEvent::Activity(format!("{} completed", result.name))
                        }
                        AgentEvent::Finished => WorkerEvent::Finished,
                        AgentEvent::Cancelled => WorkerEvent::Cancelled,
                    };
                    let _ = sender.send(message);
                },
            )
            .map_err(|error| error.to_string()),
            Ok(None) => Err("OpenRouter key missing. Run `roven auth set`.".to_owned()),
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = result {
            let _ = sender.send(WorkerEvent::Error(error));
        }
    });
}

fn apply_worker_event(
    state: &mut AppState,
    store: Option<&ProjectStore>,
    session: Option<&SessionMeta>,
    event: WorkerEvent,
) {
    match event {
        WorkerEvent::Thought(thought) => state.append_thought(thought),
        WorkerEvent::Text(text) => state.append_agent_text(text),
        WorkerEvent::Activity(message) => state.activity(message),
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
        }
        WorkerEvent::Error(error) => {
            state.finish_agent();
            persist_generation(state, store, session, EventKind::Assistant);
            if let (Some(project_store), Some(active_session)) = (store, session) {
                let _ = project_store.append_event(
                    &active_session.id,
                    &ConversationEvent {
                        kind: EventKind::Error,
                        content: error.clone(),
                        duration_ms: None,
                        created_at_ms: now_ms(),
                    },
                );
            }
            state.agent_error(error);
        }
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
                &ConversationEvent {
                    kind: event_kind,
                    content: message.content.clone(),
                    duration_ms: message.duration_ms,
                    created_at_ms: now_ms(),
                },
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
    for event in store.events(&session.id)? {
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
            EventKind::Tool | EventKind::Error => {}
        }
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
        EventKind::Tool => Role::Activity,
        EventKind::Assistant | EventKind::Error | EventKind::Cancelled => Role::Roven,
    };
    Message {
        role,
        content: event.content,
        duration_ms: event.duration_ms,
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

    use crate::storage::{ConversationEvent, EventKind, ProjectStore};

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
            ConversationEvent {
                kind: EventKind::User,
                content: "Explain the failure".to_owned(),
                duration_ms: None,
                created_at_ms: 1,
            },
            ConversationEvent {
                kind: EventKind::Thought,
                content: "I should inspect the error.".to_owned(),
                duration_ms: Some(757),
                created_at_ms: 2,
            },
            ConversationEvent {
                kind: EventKind::Assistant,
                content: "The request was rate limited.".to_owned(),
                duration_ms: None,
                created_at_ms: 3,
            },
        ] {
            store.append_event(&session.id, &event).unwrap();
        }

        let messages = super::request_messages(&store, &session, "").unwrap();

        assert!(matches!(
            &messages[0],
            crate::agent::AgentMessage::System { content }
                if content.contains("`prepare_project` with `{\"path\":\".\"}`")
                    && content.contains("`list_directory` with `{\"path\":\".\"}`")
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
}
