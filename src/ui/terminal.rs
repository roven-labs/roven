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
        KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use serde_json::Value;

use crate::{
    agent::{self, AgentEvent, AgentMessage},
    credentials::{self, SecretStore},
    model_catalog::{ProviderKind, validate_model},
    ollama, openrouter,
    profiles::{ProviderProfile, ProviderProfiles},
    provider::{OpenAiCompatibleProvider, Provider},
    runtime_log::RuntimeLog,
    storage::{ConversationEvent, EventKind, ProjectStore, SessionMeta},
    tools::{RovenToolCall, RovenToolResult, ToolContext},
};

use super::{
    startup,
    state::{
        AppState, Message, ProviderAccessState, ProviderChoice, ResumeEntry, Role, SlashCommand,
        slash_command,
    },
    view,
};

#[derive(Debug)]
enum WorkerEvent {
    Thought(String),
    Text(String),
    ContextUsage(usize),
    FunctionCallOutput {
        call: RovenToolCall,
        result: RovenToolResult,
    },
    Finished,
    Cancelled,
    Error(String),
}

const TOOL_USE_POLICY: &str = r#"Treat every request as read-only unless the user explicitly asks to prepare, register, add, modify, delete, or configure something. Call `prepare_project` only when the user explicitly asks to prepare, register, or add a project; never call it merely because a trusted workspace is available. For an explicit prepare/register/add request about the current trusted workspace, use `prepare_project` with {"path":"."}. When the user asks which Roven tools or capabilities are available, call `list_tools` with {} and rely on its returned names, descriptions, and input schemas. When the user asks for the current workspace path, call `list_directory` with {"path":"."} and report its `workspace_path` value verbatim; never report `.` as the human-facing path. When the user asks about a file's contents, call `list_directory` to locate it, then call `read_file` with a non-empty workspace-relative path. `list_directory` lists only immediate entries and may return `truncated: true`; use returned entry paths as the next tool input. If a filesystem tool returns an error, correct the path from its reason and do not retry the unchanged request. Rely on the returned content and never claim that a file was read without a tool result."#;
const REGISTER_PROJECT_PROMPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/prompts/register-project.md"
));

fn slash_command_prompt(input: &str) -> Option<&'static str> {
    (slash_command(input) == Some(SlashCommand::Register)).then_some(REGISTER_PROJECT_PROMPT)
}

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
    refresh_startup_provider_status(&mut state);
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
                    refresh_provider_model(&mut state);
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
                Event::Mouse(crossterm::event::MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    row,
                    ..
                }) => {
                    if let Some(index) = state.resume_index_at_row(row) {
                        state.select_resume(index);
                    }
                }
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
        if state.model_selection.is_some() {
            if let Event::Key(key) = event
                && let Err(error) = handle_model_selection_key(
                    &mut state,
                    &ProviderProfiles::for_current_user()?,
                    key,
                )
            {
                state.activity(error);
            }
            continue;
        }
        if state.running {
            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => {
                    if let Some(flag) = &cancellation {
                        flag.store(true, Ordering::Relaxed);
                        log_event(runtime_log, "agent_cancellation_requested", "source=escape");
                    }
                    state.status = Some("Stopping agent...".to_owned());
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) if modifiers.contains(KeyModifiers::ALT) => state.insert_newline(),
                Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
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
            continue;
        }
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) if state.slash_command_menu_open() => state.close_slash_command_menu(),
            Event::Key(KeyEvent {
                code: KeyCode::Up,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) if state.slash_command_menu_open() => state.select_previous_slash_command(),
            Event::Key(KeyEvent {
                code: KeyCode::Down,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) if state.slash_command_menu_open() => state.select_next_slash_command(),
            Event::Key(KeyEvent {
                code: KeyCode::Tab,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) if state.slash_command_menu_open() => state.insert_selected_slash_command(),
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
            }) if state.slash_command_menu_open() => state.insert_selected_slash_command(),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }) => {
                let raw = state.input.clone();
                let command = slash_command(&raw);
                if command == Some(SlashCommand::Resume) {
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
                } else if command == Some(SlashCommand::Model) {
                    state.input.clear();
                    if let Err(error) = open_model_switch(&mut state) {
                        state.activity(error);
                    }
                } else if state.submit() {
                    refresh_provider_model(&mut state);
                    let user = state.last_user_message().unwrap_or_default().to_owned();
                    let stored_user = slash_command_prompt(&user).unwrap_or(&user).to_owned();
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
                            &ConversationEvent::message(EventKind::User, stored_user, None),
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
            let key = resolve_profile_api_key(
                &profile,
                &credentials::OsCredentialStore::for_profile_id(&profile.id),
            )?;
            let endpoint = profile.endpoint;
            let model = profile.model;
            let is_ollama = ollama::is_native_endpoint(&endpoint);
            let context_window = if is_ollama {
                ollama::context_window(&key, &endpoint, &model)
            } else {
                openrouter::context_window(&key, &endpoint, &model)
            };
            let provider: Box<dyn Provider> = if is_ollama {
                Box::new(ollama::OllamaProvider::new(endpoint, model))
            } else {
                Box::new(OpenAiCompatibleProvider::new(endpoint, model))
            };
            agent::run(
                agent::AgentRun {
                    provider: provider.as_ref(),
                    api_key: &key,
                    tool_context: &tool_context,
                    context_window,
                    cancelled: &cancelled,
                    runtime_log: runtime_log.as_ref(),
                },
                messages,
                &mut |event| {
                    let message = match event {
                        AgentEvent::Thought(thought) => WorkerEvent::Thought(thought),
                        AgentEvent::Text(text) => WorkerEvent::Text(text),
                        AgentEvent::ContextUsage(prompt_tokens) => {
                            WorkerEvent::ContextUsage(prompt_tokens)
                        }
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

fn resolve_profile_api_key(
    profile: &ProviderProfile,
    store: &impl SecretStore,
) -> Result<String, String> {
    credentials::resolve_api_key(profile, store)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "API key missing for provider profile `{}`. Run `roven auth set`.",
                profile.name
            )
        })
}

fn open_model_switch(state: &mut AppState) -> Result<(), String> {
    let profiles = ProviderProfiles::for_current_user().map_err(|error| error.to_string())?;
    open_model_switch_with(state, &profiles, provider_access_state)
}

fn open_model_switch_with<F>(
    state: &mut AppState,
    profiles: &ProviderProfiles,
    access_state_for: F,
) -> Result<(), String>
where
    F: Fn(&ProviderProfile) -> ProviderAccessState,
{
    let entries = profiles
        .list()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|profile| {
            let access = access_state_for(&profile);
            ProviderChoice {
                id: profile.id,
                name: profile.name,
                endpoint: profile.endpoint,
                model: profile.model,
                access,
            }
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Err("No provider profiles. Run `roven auth set`.".to_owned());
    }
    let default_profile_id = profiles
        .default_profile()
        .map_err(|error| error.to_string())?
        .map(|profile| profile.id);
    state.open_model_selection(entries, default_profile_id.as_deref());
    Ok(())
}

fn handle_model_selection_key(
    state: &mut AppState,
    profiles: &ProviderProfiles,
    key: KeyEvent,
) -> Result<bool, String> {
    match key.code {
        KeyCode::Esc => {
            state.close_model_selection();
            Ok(false)
        }
        KeyCode::Up => {
            state.select_previous_model_provider();
            Ok(false)
        }
        KeyCode::Down => {
            state.select_next_model_provider();
            Ok(false)
        }
        KeyCode::Enter => {
            let Some(selection) = state.model_selection.clone() else {
                return Ok(false);
            };
            match selection {
                super::state::ModelSelection::Provider { .. } => {
                    state.begin_model_entry();
                    Ok(false)
                }
                super::state::ModelSelection::Model { choice, value, .. } => {
                    let model = value.trim().to_owned();
                    if model.is_empty() {
                        state.close_model_selection();
                        return Ok(false);
                    }
                    if !choice.access.is_ready() {
                        state.set_model_entry_error(match choice.access {
                            ProviderAccessState::MissingApiKey => format!(
                                "{} is missing an API key. Run `roven auth set` before switching to it.",
                                choice.name
                            ),
                            ProviderAccessState::CredentialStoreUnavailable => format!(
                                "{} is unavailable because the credential store could not be read.",
                                choice.name
                            ),
                            ProviderAccessState::Ready => String::new(),
                        });
                        return Ok(false);
                    }
                    if !validate_model(&choice.endpoint, &model) {
                        state.set_model_entry_error(unsupported_model_message(&choice, &model));
                        return Ok(false);
                    }
                    let updated = profiles
                        .switch_model_and_default(&choice.id, &model)
                        .map_err(|error| error.to_string())?;
                    refresh_provider_model_from(state, profiles);
                    state.close_model_selection();
                    state.activity(format!(
                        "Active provider set to {} · {}",
                        updated.name, updated.model
                    ));
                    Ok(true)
                }
            }
        }
        KeyCode::Backspace => {
            state.backspace_model_entry();
            Ok(false)
        }
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.push_model_entry_char(character);
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn provider_access_state(profile: &ProviderProfile) -> ProviderAccessState {
    match credentials::resolve_api_key(
        profile,
        &credentials::OsCredentialStore::for_profile_id(&profile.id),
    ) {
        Ok(Some(_)) => ProviderAccessState::Ready,
        Ok(None) => ProviderAccessState::MissingApiKey,
        Err(_) => ProviderAccessState::CredentialStoreUnavailable,
    }
}

fn unsupported_model_message(choice: &ProviderChoice, model: &str) -> String {
    match ProviderKind::from_endpoint(&choice.endpoint) {
        Some(ProviderKind::OllamaCloud) => format!(
            "`{model}` is not a supported Ollama Cloud model for {}. Enter one of Ollama's documented cloud model IDs.",
            choice.name
        ),
        Some(ProviderKind::OpenRouter) => format!(
            "`{model}` is not a valid OpenRouter model ID for {}. Enter it as `provider/model`.",
            choice.name
        ),
        None => format!("`{model}` is not supported for {}.", choice.name),
    }
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
        WorkerEvent::ContextUsage(prompt_tokens) => state.context_percent = Some(prompt_tokens),
        WorkerEvent::FunctionCallOutput { call, result } => {
            persist_function_call_output(store, session, &call, &result);
            state.tool(call.name, call.arguments, result.result);
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

fn refresh_provider_model(state: &mut AppState) {
    if let Ok(profiles) = ProviderProfiles::for_current_user() {
        refresh_provider_model_from(state, &profiles);
    } else {
        state.provider_model = None;
    }
}

fn refresh_startup_provider_status(state: &mut AppState) {
    match ProviderProfiles::for_current_user() {
        Ok(profiles) => refresh_startup_provider_status_from(state, &profiles),
        Err(_) => {
            state.startup_provider_status = Some(startup::detect_provider_status(
                &[],
                |_| false,
                credentials::has_provider_env_api_key,
            ));
        }
    }
}

fn refresh_startup_provider_status_from(state: &mut AppState, profiles: &ProviderProfiles) {
    let profiles = profiles.list().unwrap_or_default();
    state.startup_provider_status = Some(startup::detect_provider_status(
        &profiles,
        |profile| {
            credentials::resolve_api_key(
                profile,
                &credentials::OsCredentialStore::for_profile_id(&profile.id),
            )
            .ok()
            .flatten()
            .is_some()
        },
        credentials::has_provider_env_api_key,
    ));
}

fn refresh_provider_model_from(state: &mut AppState, profiles: &ProviderProfiles) {
    state.provider_model = profiles
        .default_profile()
        .ok()
        .flatten()
        .map(|profile| format!("{} · {}", profile.name, profile.model));
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
                    content: slash_command_prompt(&event.content)
                        .unwrap_or(&event.content)
                        .to_owned(),
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
        _ => Message::text(
            role,
            if event.content == REGISTER_PROJECT_PROMPT {
                "/register".to_owned()
            } else {
                event.content
            },
            event.duration_ms,
        ),
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
    use std::{
        cell::RefCell,
        fs,
        sync::{Mutex, OnceLock},
    };

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{
        agent::AgentMessage,
        credentials::{CredentialError, SecretStore},
        profiles::{ProviderProfile, ProviderProfiles},
        storage::{ConversationEvent, EventKind, ProjectStore},
        ui::{
            startup::StartupProviderStatus,
            state::{AppState, ModelSelection, ResumeEntry, Role},
        },
    };

    use super::{WorkerEvent, apply_worker_event};
    use crate::tools::{RovenToolCall, RovenToolResult};

    #[derive(Default)]
    struct MemoryStore {
        value: RefCell<Option<String>>,
    }

    impl SecretStore for MemoryStore {
        fn get(&self) -> Result<Option<String>, CredentialError> {
            Ok(self.value.borrow().clone())
        }

        fn set(&self, secret: &str) -> Result<(), CredentialError> {
            *self.value.borrow_mut() = Some(secret.to_owned());
            Ok(())
        }

        fn delete(&self) -> Result<bool, CredentialError> {
            Ok(self.value.borrow_mut().take().is_some())
        }
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).expect("temporary root should exist");
        path
    }

    fn profile(id: &str, endpoint: &str) -> ProviderProfile {
        ProviderProfile {
            id: id.to_owned(),
            name: "provider".to_owned(),
            endpoint: endpoint.to_owned(),
            model: "model".to_owned(),
        }
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock should not be poisoned")
    }

    fn with_provider_envs<T>(
        openrouter: Option<&str>,
        ollama: Option<&str>,
        run: impl FnOnce() -> T,
    ) -> T {
        let previous_openrouter = std::env::var_os("OPENROUTER_API_KEY");
        let previous_ollama = std::env::var_os("OLLAMA_API_KEY");
        match openrouter {
            Some(value) => unsafe { std::env::set_var("OPENROUTER_API_KEY", value) },
            None => unsafe { std::env::remove_var("OPENROUTER_API_KEY") },
        }
        match ollama {
            Some(value) => unsafe { std::env::set_var("OLLAMA_API_KEY", value) },
            None => unsafe { std::env::remove_var("OLLAMA_API_KEY") },
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        match previous_openrouter {
            Some(value) => unsafe { std::env::set_var("OPENROUTER_API_KEY", value) },
            None => unsafe { std::env::remove_var("OPENROUTER_API_KEY") },
        }
        match previous_ollama {
            Some(value) => unsafe { std::env::set_var("OLLAMA_API_KEY", value) },
            None => unsafe { std::env::remove_var("OLLAMA_API_KEY") },
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[test]
    fn register_slash_command_expands_only_in_agent_context() {
        let prompt = super::slash_command_prompt("  /register  ").expect("command should expand");

        assert!(prompt.contains("prepare_project"));
        assert!(prompt.contains("summary"));
        assert!(super::slash_command("/unknown").is_none());
        assert_eq!(
            super::slash_command("/resume"),
            Some(super::SlashCommand::Resume)
        );
        assert_eq!(
            super::slash_command("/model"),
            Some(super::SlashCommand::Model)
        );
    }

    #[test]
    fn resume_picker_maps_session_rows_to_indices() {
        let mut state = AppState::new();
        state.open_resume(vec![
            ResumeEntry {
                id: "first".to_owned(),
                title: "First".to_owned(),
                updated_at_ms: 0,
            },
            ResumeEntry {
                id: "second".to_owned(),
                title: "Second".to_owned(),
                updated_at_ms: 0,
            },
        ]);
        state.set_resume_viewport(6, 2, 0);

        assert_eq!(state.resume_index_at_row(6), Some(0));
        assert_eq!(state.resume_index_at_row(7), Some(1));
        assert_eq!(state.resume_index_at_row(5), None);
        assert_eq!(state.resume_index_at_row(8), None);
    }

    #[test]
    fn register_slash_command_stays_visible_while_agent_context_expands() {
        let root = temp_root("register-command-display");
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        let store = ProjectStore::for_project(&root, &project).unwrap();
        let session = store.create_session("/register").unwrap();
        let prompt = super::slash_command_prompt("/register").unwrap();
        store
            .append_event(
                &session.id,
                &ConversationEvent::message(EventKind::User, prompt.to_owned(), None),
            )
            .unwrap();

        let messages = super::request_messages(&store, &session, "").unwrap();

        assert_eq!(session.title, "/register");
        assert_eq!(store.events(&session.id).unwrap()[0].content, prompt);
        assert!(matches!(
            messages.last(),
            Some(AgentMessage::User { content }) if content == prompt
        ));
        assert_eq!(
            super::event_to_message(store.events(&session.id).unwrap()[0].clone()).content,
            "/register"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_register_events_expand_in_agent_context() {
        let root = temp_root("legacy-register-command");
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        let store = ProjectStore::for_project(&root, &project).unwrap();
        let session = store.create_session("/register").unwrap();
        store
            .append_event(
                &session.id,
                &ConversationEvent::message(EventKind::User, "/register".to_owned(), None),
            )
            .unwrap();

        let messages = super::request_messages(&store, &session, "").unwrap();

        assert!(matches!(
            messages.last(),
            Some(AgentMessage::User { content }) if content == super::REGISTER_PROJECT_PROMPT
        ));
        fs::remove_dir_all(root).unwrap();
    }

    fn refreshed_startup_provider_status(
        openrouter: Option<&str>,
        ollama: Option<&str>,
    ) -> StartupProviderStatus {
        let _guard = env_lock();
        let data_root = temp_root("startup-provider-status");
        let profiles = ProviderProfiles::for_data_root(data_root.clone());
        let mut state = AppState::new();
        let status = with_provider_envs(openrouter, ollama, || {
            super::refresh_startup_provider_status_from(&mut state, &profiles);
            state
                .startup_provider_status
                .expect("startup provider status should be set")
        });
        fs::remove_dir_all(data_root).expect("temporary root should be removed");
        status
    }

    #[test]
    fn runtime_key_resolution_prefers_environment_and_falls_back_to_store() {
        let _guard = env_lock();
        let profile = profile(
            "openrouter",
            "https://openrouter.ai/api/v1/chat/completions",
        );
        let store = MemoryStore {
            value: RefCell::new(Some("stored-secret".into())),
        };
        let previous = std::env::var_os("OPENROUTER_API_KEY");

        unsafe { std::env::set_var("OPENROUTER_API_KEY", "env-secret") };
        assert_eq!(
            super::resolve_profile_api_key(&profile, &store).unwrap(),
            "env-secret"
        );

        unsafe { std::env::set_var("OPENROUTER_API_KEY", "   ") };
        assert_eq!(
            super::resolve_profile_api_key(&profile, &store).unwrap(),
            "stored-secret"
        );

        match previous {
            Some(value) => unsafe { std::env::set_var("OPENROUTER_API_KEY", value) },
            None => unsafe { std::env::remove_var("OPENROUTER_API_KEY") },
        }
    }

    #[test]
    fn startup_status_detects_openrouter_env_without_saved_profile() {
        assert_eq!(
            refreshed_startup_provider_status(Some("openrouter-env-secret"), None),
            StartupProviderStatus::OpenRouterOnly
        );
    }

    #[test]
    fn startup_status_detects_ollama_env_without_saved_profile() {
        assert_eq!(
            refreshed_startup_provider_status(None, Some("ollama-env-secret")),
            StartupProviderStatus::OllamaOnly
        );
    }

    #[test]
    fn startup_status_detects_both_envs_without_saved_profiles() {
        assert_eq!(
            refreshed_startup_provider_status(
                Some("openrouter-env-secret"),
                Some("ollama-env-secret")
            ),
            StartupProviderStatus::BothConfigured
        );
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

    #[test]
    fn live_function_call_output_is_appended_as_a_structured_tool_message() {
        let mut state = AppState::new();
        let call = RovenToolCall {
            id: "call-1".to_owned(),
            name: "list_directory".to_owned(),
            arguments: serde_json::json!({"path": "."}),
        };
        let result = RovenToolResult {
            tool_call_id: "call-1".to_owned(),
            name: "list_directory".to_owned(),
            result: serde_json::json!({"status": "ok"}),
        };

        apply_worker_event(
            &mut state,
            None,
            None,
            None,
            WorkerEvent::FunctionCallOutput { call, result },
        );

        assert_eq!(state.messages.len(), 1);
        let message = &state.messages[0];
        assert_eq!(message.role, Role::Activity);
        assert!(message.content.is_empty());
        assert!(matches!(
            message.kind,
            crate::ui::state::MessageKind::Tool {
                ref name,
                ref input,
                ref output,
            } if name == "list_directory"
                && input["path"] == "."
                && output["status"] == "ok"
        ));
    }

    #[test]
    fn worker_errors_and_cancellation_update_the_ui_and_persist_outcomes() {
        let data = temp_root("worker-events-data");
        let project = temp_root("worker-events-project");
        let store = ProjectStore::for_project(&data, &project).unwrap();
        let session = store.create_session("Inspect the workspace").unwrap();
        let mut state = AppState::new();

        state.input = "Inspect the workspace".to_owned();
        assert!(state.submit());
        state.start_agent();
        state.append_agent_text("partial response".to_owned());
        apply_worker_event(
            &mut state,
            Some(&store),
            Some(&session),
            None,
            WorkerEvent::Error("provider failed".to_owned()),
        );
        assert!(!state.running);
        assert!(
            state
                .messages
                .iter()
                .any(|message| message.content == "Error: provider failed")
        );

        state.input = "Cancel the request".to_owned();
        assert!(state.submit());
        state.start_agent();
        state.append_agent_text("stopped response".to_owned());
        apply_worker_event(
            &mut state,
            Some(&store),
            Some(&session),
            None,
            WorkerEvent::Cancelled,
        );
        assert!(!state.running);
        assert!(
            state
                .messages
                .iter()
                .any(|message| message.content == "Agent stopped")
        );

        let events = store.events(&session.id).unwrap();
        assert!(events.iter().any(|event| {
            event.kind == EventKind::Assistant && event.content == "partial response"
        }));
        assert!(
            events.iter().any(|event| {
                event.kind == EventKind::Error && event.content == "provider failed"
            })
        );
        assert!(events.iter().any(|event| {
            event.kind == EventKind::Cancelled && event.content == "stopped response"
        }));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn model_switch_changes_the_selected_provider_and_model() {
        let data_root = temp_root("model-switch-success");
        let profiles = crate::profiles::ProviderProfiles::for_data_root(data_root.clone());
        let first = profiles
            .create(
                "OpenRouter",
                "https://openrouter.ai/api/v1/chat/completions",
                "openai/gpt-oss-20b",
            )
            .unwrap();
        let second = profiles
            .create("Ollama", "https://ollama.com/api/chat", "minimax-m3:cloud")
            .unwrap();
        profiles.set_default(&first.id).unwrap();

        let mut state = AppState::new();
        super::open_model_switch_with(&mut state, &profiles, |_profile| {
            crate::ui::state::ProviderAccessState::Ready
        })
        .unwrap();
        super::handle_model_selection_key(
            &mut state,
            &profiles,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        )
        .unwrap();
        super::handle_model_selection_key(
            &mut state,
            &profiles,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
        .unwrap();
        match state.model_selection.as_mut().unwrap() {
            ModelSelection::Model { value, .. } => *value = "gpt-oss:120b-cloud".to_owned(),
            selection => panic!("expected model entry, got {selection:?}"),
        }

        assert!(
            super::handle_model_selection_key(
                &mut state,
                &profiles,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            )
            .unwrap()
        );
        assert!(state.model_selection.is_none());
        assert_eq!(profiles.default_profile().unwrap().unwrap().id, second.id);
        assert_eq!(
            profiles.default_profile().unwrap().unwrap().model,
            "gpt-oss:120b-cloud"
        );
        assert_eq!(
            state.provider_model.as_deref(),
            Some("Ollama · gpt-oss:120b-cloud")
        );
        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn blank_model_entry_cancels_without_changing_the_current_selection() {
        let data_root = temp_root("model-switch-blank-cancel");
        let profiles = crate::profiles::ProviderProfiles::for_data_root(data_root.clone());
        let default = profiles
            .create(
                "OpenRouter",
                "https://openrouter.ai/api/v1/chat/completions",
                "openai/gpt-oss-20b",
            )
            .unwrap();
        let second = profiles
            .create("Ollama", "https://ollama.com/api/chat", "minimax-m3:cloud")
            .unwrap();
        profiles.set_default(&default.id).unwrap();

        let mut state = AppState::new();
        state.provider_model = Some("OpenRouter · openai/gpt-oss-20b".to_owned());
        super::open_model_switch_with(&mut state, &profiles, |_profile| {
            crate::ui::state::ProviderAccessState::Ready
        })
        .unwrap();
        super::handle_model_selection_key(
            &mut state,
            &profiles,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        )
        .unwrap();
        super::handle_model_selection_key(
            &mut state,
            &profiles,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
        .unwrap();
        match state.model_selection.as_mut().unwrap() {
            ModelSelection::Model { value, .. } => *value = "   ".to_owned(),
            selection => panic!("expected model entry, got {selection:?}"),
        }

        assert!(
            !super::handle_model_selection_key(
                &mut state,
                &profiles,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            )
            .unwrap()
        );
        assert!(state.model_selection.is_none());
        assert_eq!(profiles.default_profile().unwrap().unwrap().id, default.id);
        assert_eq!(
            profiles
                .list()
                .unwrap()
                .into_iter()
                .find(|profile| profile.id == second.id)
                .unwrap()
                .model,
            "minimax-m3:cloud"
        );
        assert_eq!(
            state.provider_model.as_deref(),
            Some("OpenRouter · openai/gpt-oss-20b")
        );
        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn escape_cancels_provider_selection_without_changing_the_current_selection() {
        let data_root = temp_root("model-switch-escape-cancel");
        let profiles = crate::profiles::ProviderProfiles::for_data_root(data_root.clone());
        let default = profiles
            .create(
                "OpenRouter",
                "https://openrouter.ai/api/v1/chat/completions",
                "openai/gpt-oss-20b",
            )
            .unwrap();
        profiles
            .create("Ollama", "https://ollama.com/api/chat", "minimax-m3:cloud")
            .unwrap();
        profiles.set_default(&default.id).unwrap();

        let mut state = AppState::new();
        state.provider_model = Some("OpenRouter · openai/gpt-oss-20b".to_owned());
        super::open_model_switch_with(&mut state, &profiles, |_profile| {
            crate::ui::state::ProviderAccessState::Ready
        })
        .unwrap();

        assert!(
            !super::handle_model_selection_key(
                &mut state,
                &profiles,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            )
            .unwrap()
        );
        assert!(state.model_selection.is_none());
        assert_eq!(profiles.default_profile().unwrap().unwrap().id, default.id);
        assert_eq!(
            state.provider_model.as_deref(),
            Some("OpenRouter · openai/gpt-oss-20b")
        );
        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn unsupported_model_keeps_the_previous_selection_and_shows_a_friendly_error() {
        let data_root = temp_root("model-switch-invalid");
        let profiles = crate::profiles::ProviderProfiles::for_data_root(data_root.clone());
        let default = profiles
            .create(
                "OpenRouter",
                "https://openrouter.ai/api/v1/chat/completions",
                "openai/gpt-oss-20b",
            )
            .unwrap();
        profiles
            .create("Ollama", "https://ollama.com/api/chat", "minimax-m3:cloud")
            .unwrap();
        profiles.set_default(&default.id).unwrap();

        let mut state = AppState::new();
        state.provider_model = Some("OpenRouter · openai/gpt-oss-20b".to_owned());
        super::open_model_switch_with(&mut state, &profiles, |_profile| {
            crate::ui::state::ProviderAccessState::Ready
        })
        .unwrap();
        super::handle_model_selection_key(
            &mut state,
            &profiles,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        )
        .unwrap();
        super::handle_model_selection_key(
            &mut state,
            &profiles,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
        .unwrap();
        match state.model_selection.as_mut().unwrap() {
            ModelSelection::Model { value, .. } => *value = "llama3.1:8b".to_owned(),
            selection => panic!("expected model entry, got {selection:?}"),
        }

        assert!(
            !super::handle_model_selection_key(
                &mut state,
                &profiles,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            )
            .unwrap()
        );
        match state.model_selection.as_ref().unwrap() {
            ModelSelection::Model { error, .. } => {
                let error = error.as_deref().unwrap();
                assert!(error.contains("llama3.1:8b"));
                assert!(error.contains("Ollama"));
                assert!(error.contains("supported"));
            }
            selection => panic!("expected model entry, got {selection:?}"),
        }
        assert_eq!(profiles.default_profile().unwrap().unwrap().id, default.id);
        assert_eq!(
            state.provider_model.as_deref(),
            Some("OpenRouter · openai/gpt-oss-20b")
        );
        fs::remove_dir_all(data_root).unwrap();
    }
}
