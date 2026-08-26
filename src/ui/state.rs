use std::time::Instant;

use serde_json::Value;

use super::startup::StartupProviderStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Thought,
    Roven,
    Activity,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Message {
    pub(crate) role: Role,
    pub(crate) content: String,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) kind: MessageKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MessageKind {
    Text,
    Tool {
        name: String,
        input: Value,
        output: Value,
    },
}

impl Message {
    pub(crate) fn text(role: Role, content: String, duration_ms: Option<u64>) -> Self {
        Self {
            role,
            content,
            duration_ms,
            kind: MessageKind::Text,
        }
    }

    pub(crate) fn tool(name: String, input: Value, output: Value) -> Self {
        Self {
            role: Role::Activity,
            content: String::new(),
            duration_ms: None,
            kind: MessageKind::Tool {
                name,
                input,
                output,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeEntry {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAccessState {
    Ready,
    MissingApiKey,
    CredentialStoreUnavailable,
}

impl ProviderAccessState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::MissingApiKey => "API key missing",
            Self::CredentialStoreUnavailable => "credential store unavailable",
        }
    }

    pub(crate) fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderChoice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) access: ProviderAccessState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelSelection {
    Provider {
        entries: Vec<ProviderChoice>,
        index: usize,
    },
    Model {
        choice: ProviderChoice,
        value: String,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashCommand {
    Register,
    Resume,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlashCommandInfo {
    pub(crate) command: SlashCommand,
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}

const SLASH_COMMANDS: [SlashCommandInfo; 3] = [
    SlashCommandInfo {
        command: SlashCommand::Register,
        name: "/register",
        description: "Prepare this project",
    },
    SlashCommandInfo {
        command: SlashCommand::Resume,
        name: "/resume",
        description: "Resume a conversation",
    },
    SlashCommandInfo {
        command: SlashCommand::Model,
        name: "/model",
        description: "Switch model",
    },
];

pub(crate) fn slash_command(input: &str) -> Option<SlashCommand> {
    let input = input.trim();
    SLASH_COMMANDS
        .iter()
        .find(|command| command.name == input)
        .map(|command| command.command)
}

#[derive(Debug, Default)]
pub(crate) struct AppState {
    pub(crate) messages: Vec<Message>,
    pub(crate) input: String,
    pub(crate) scroll_offset: u16,
    scroll_limit: u16,
    pub(crate) trusted: bool,
    pub(crate) project_path: String,
    pub(crate) trust_yes_selected: bool,
    pub(crate) running: bool,
    pub(crate) status: Option<String>,
    pub(crate) provider_model: Option<String>,
    pub(crate) context_percent: Option<usize>,
    generation_start_index: usize,
    generation_started_at: Option<Instant>,
    pub(crate) resume_entries: Option<Vec<ResumeEntry>>,
    pub(crate) resume_index: usize,
    pub(crate) model_selection: Option<ModelSelection>,
    pub(crate) startup_provider_status: Option<StartupProviderStatus>,
    slash_command_menu_open: bool,
    pub(crate) slash_command_index: usize,
}

impl AppState {
    pub(crate) fn new() -> Self {
        Self {
            trust_yes_selected: true,
            ..Self::default()
        }
    }

    pub(crate) fn toggle_trust_selection(&mut self) {
        self.trust_yes_selected = !self.trust_yes_selected;
    }

    pub(crate) fn insert_char(&mut self, character: char) {
        if self.resume_entries.is_none() && self.model_selection.is_none() {
            self.input.push(character);
            self.update_slash_command_menu();
        }
    }

    pub(crate) fn backspace(&mut self) {
        if self.resume_entries.is_none() && self.model_selection.is_none() {
            self.input.pop();
            self.update_slash_command_menu();
        }
    }

    pub(crate) fn insert_newline(&mut self) {
        if self.resume_entries.is_none() && self.model_selection.is_none() {
            self.input.push('\n');
            self.update_slash_command_menu();
        }
    }

    pub(crate) fn slash_commands(&self) -> impl Iterator<Item = SlashCommandInfo> + '_ {
        SLASH_COMMANDS.into_iter().filter(move |command| {
            self.slash_command_menu_open && !self.running && command.name.starts_with(&self.input)
        })
    }

    pub(crate) fn slash_command_menu_open(&self) -> bool {
        self.slash_commands().next().is_some()
    }

    pub(crate) fn close_slash_command_menu(&mut self) {
        self.slash_command_menu_open = false;
    }

    pub(crate) fn select_previous_slash_command(&mut self) {
        self.slash_command_index = self.slash_command_index.saturating_sub(1);
    }

    pub(crate) fn select_next_slash_command(&mut self) {
        self.slash_command_index =
            (self.slash_command_index + 1).min(self.slash_commands().count().saturating_sub(1));
    }

    pub(crate) fn insert_selected_slash_command(&mut self) {
        let command_name = self
            .slash_commands()
            .nth(self.slash_command_index)
            .map(|command| command.name);
        if let Some(command_name) = command_name {
            self.input = command_name.to_owned();
            self.close_slash_command_menu();
        }
    }

    pub(crate) fn submit(&mut self) -> bool {
        if self.running
            || self.resume_entries.is_some()
            || self.model_selection.is_some()
            || self.input.trim().is_empty()
        {
            return false;
        }

        let content = std::mem::take(&mut self.input);
        self.close_slash_command_menu();
        self.messages.push(Message::text(Role::User, content, None));
        self.scroll_offset = 0;
        true
    }

    pub(crate) fn last_user_message(&self) -> Option<&str> {
        self.messages
            .last()
            .and_then(|message| (message.role == Role::User).then_some(message.content.as_str()))
    }

    pub(crate) fn start_agent(&mut self) {
        self.running = true;
        self.status = Some("Agent working...".to_owned());
        self.context_percent = None;
        self.generation_start_index = self.messages.len();
        self.generation_started_at = Some(Instant::now());
    }

    pub(crate) fn append_thought(&mut self, text: String) {
        self.status = Some("Thinking...".to_owned());
        if let Some(message) = self
            .messages
            .last_mut()
            .filter(|message| message.role == Role::Thought)
        {
            message.content.push_str(&text);
            return;
        }
        self.messages.push(Message::text(Role::Thought, text, None));
    }

    pub(crate) fn append_agent_text(&mut self, text: String) {
        self.status = Some("Writing response...".to_owned());
        self.finish_active_thought();
        if let Some(message) = self
            .messages
            .last_mut()
            .filter(|message| message.role == Role::Roven)
        {
            message.content.push_str(&text);
            return;
        }
        self.messages.push(Message::text(Role::Roven, text, None));
    }

    pub(crate) fn finish_agent(&mut self) {
        self.finish_active_thought();
        self.running = false;
        self.status = None;
    }

    pub(crate) fn agent_error(&mut self, message: String) {
        self.finish_agent();
        self.messages.push(Message::text(
            Role::Roven,
            format!("Error: {message}"),
            None,
        ));
    }

    pub(crate) fn activity(&mut self, message: impl Into<String>) {
        self.messages
            .push(Message::text(Role::Activity, message.into(), None));
    }

    pub(crate) fn tool(&mut self, name: String, input: Value, output: Value) {
        self.messages.push(Message::tool(name, input, output));
    }

    pub(crate) fn open_resume(&mut self, entries: Vec<ResumeEntry>) {
        self.resume_entries = Some(entries);
        self.resume_index = 0;
    }

    pub(crate) fn close_resume(&mut self) {
        self.resume_entries = None;
    }

    pub(crate) fn open_model_selection(
        &mut self,
        entries: Vec<ProviderChoice>,
        default_profile_id: Option<&str>,
    ) {
        let index = default_profile_id
            .and_then(|id| entries.iter().position(|entry| entry.id == id))
            .unwrap_or(0);
        self.model_selection = Some(ModelSelection::Provider { entries, index });
    }

    pub(crate) fn close_model_selection(&mut self) {
        self.model_selection = None;
    }

    pub(crate) fn select_previous_model_provider(&mut self) {
        if let Some(ModelSelection::Provider { index, .. }) = self.model_selection.as_mut() {
            *index = index.saturating_sub(1);
        }
    }

    pub(crate) fn select_next_model_provider(&mut self) {
        if let Some(ModelSelection::Provider { entries, index }) = self.model_selection.as_mut() {
            *index = (*index + 1).min(entries.len().saturating_sub(1));
        }
    }

    pub(crate) fn begin_model_entry(&mut self) {
        let Some(ModelSelection::Provider { entries, index }) = &self.model_selection else {
            return;
        };
        let Some(choice) = entries.get(*index).cloned() else {
            return;
        };
        self.model_selection = Some(ModelSelection::Model {
            value: choice.model.clone(),
            choice,
            error: None,
        });
    }

    pub(crate) fn push_model_entry_char(&mut self, character: char) {
        if let Some(ModelSelection::Model { value, error, .. }) = self.model_selection.as_mut() {
            value.push(character);
            *error = None;
        }
    }

    pub(crate) fn backspace_model_entry(&mut self) {
        if let Some(ModelSelection::Model { value, error, .. }) = self.model_selection.as_mut() {
            value.pop();
            *error = None;
        }
    }

    pub(crate) fn set_model_entry_error(&mut self, message: String) {
        if let Some(ModelSelection::Model { error, .. }) = self.model_selection.as_mut() {
            *error = Some(message);
        }
    }

    pub(crate) fn selected_resume_id(&self) -> Option<&str> {
        self.resume_entries
            .as_ref()
            .and_then(|entries| entries.get(self.resume_index))
            .map(|entry| entry.id.as_str())
    }

    pub(crate) fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.scroll_offset = 0;
        self.close_resume();
        self.close_model_selection();
    }

    pub(crate) fn generated_messages(&self) -> &[Message] {
        &self.messages[self.generation_start_index..]
    }

    pub(crate) fn select_previous_resume(&mut self) {
        self.resume_index = self.resume_index.saturating_sub(1);
    }

    pub(crate) fn select_next_resume(&mut self) {
        if let Some(entries) = &self.resume_entries {
            self.resume_index = (self.resume_index + 1).min(entries.len().saturating_sub(1));
        }
    }

    pub(crate) fn scroll_up(&mut self, lines: u16) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(lines)
            .min(self.scroll_limit);
    }

    pub(crate) fn scroll_down(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub(crate) fn set_scroll_limit(&mut self, limit: u16) {
        self.scroll_limit = limit;
        self.scroll_offset = self.scroll_offset.min(limit);
    }

    pub(crate) fn is_scrolled_away_from_latest(&self) -> bool {
        self.scroll_offset > 0
    }

    fn finish_active_thought(&mut self) {
        let Some(started_at) = self.generation_started_at.take() else {
            return;
        };
        if let Some(message) = self
            .messages
            .last_mut()
            .filter(|message| message.role == Role::Thought)
        {
            message.duration_ms = Some(
                started_at
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX),
            );
        }
    }

    fn update_slash_command_menu(&mut self) {
        self.slash_command_menu_open = self.input.starts_with('/')
            && SLASH_COMMANDS
                .iter()
                .any(|command| command.name.starts_with(&self.input));
        self.slash_command_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, MessageKind, Role};

    #[test]
    fn slash_commands_filter_navigate_and_insert_without_submitting() {
        let mut state = AppState::new();
        state.insert_char('/');

        assert_eq!(state.slash_commands().count(), 3);
        state.select_next_slash_command();
        state.select_next_slash_command();
        state.insert_selected_slash_command();

        assert_eq!(state.input, "/model");
        assert!(!state.slash_command_menu_open());
        assert!(state.messages.is_empty());
    }

    #[test]
    fn slash_command_menu_hides_for_unknown_input_and_escape() {
        let mut state = AppState::new();
        for character in "/unknown".chars() {
            state.insert_char(character);
        }
        assert!(!state.slash_command_menu_open());

        for _ in 0..7 {
            state.backspace();
        }
        assert!(state.slash_command_menu_open());

        state.close_slash_command_menu();
        assert!(!state.slash_command_menu_open());
    }

    #[test]
    fn submit_appends_only_the_user_turn_until_the_worker_replies() {
        let mut state = AppState::new();
        for character in "Hello".chars() {
            state.insert_char(character);
        }

        assert!(state.submit());
        assert_eq!(state.messages.len(), 1);
        assert!(matches!(state.messages[0].role, Role::User));
        assert_eq!(state.messages[0].content, "Hello");
    }

    #[test]
    fn provider_thought_precedes_the_answer_and_records_its_duration() {
        let mut state = AppState::new();

        state.start_agent();
        assert_eq!(state.status.as_deref(), Some("Agent working..."));
        assert!(state.messages.is_empty());

        state.append_thought("Inspect the request.".to_owned());
        assert_eq!(state.status.as_deref(), Some("Thinking..."));
        state.append_agent_text("Hello".to_owned());
        assert_eq!(state.status.as_deref(), Some("Writing response..."));

        assert_eq!(state.messages[0].role, Role::Thought);
        assert_eq!(state.messages[0].content, "Inspect the request.");
        assert!(state.messages[0].duration_ms.is_some());
        assert_eq!(state.messages[1].role, Role::Roven);
        assert_eq!(state.messages[1].content, "Hello");
    }

    #[test]
    fn streamed_text_deltas_accumulate_into_one_visible_model_message() {
        let mut state = AppState::new();
        state.start_agent();

        state.append_agent_text("Here is ".to_owned());
        state.append_agent_text("the answer.".to_owned());

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].role, Role::Roven);
        assert_eq!(state.messages[0].content, "Here is the answer.");
    }

    #[test]
    fn whitespace_only_input_does_not_create_turns() {
        let mut state = AppState::new();
        state.insert_char(' ');

        assert!(!state.submit());
        assert!(state.messages.is_empty());
        assert_eq!(state.input, " ");
    }

    #[test]
    fn newline_and_backspace_edit_the_composer() {
        let mut state = AppState::new();
        state.insert_char('a');
        state.insert_newline();
        state.insert_char('b');
        state.backspace();

        assert_eq!(state.input, "a\n");
    }

    #[test]
    fn running_agent_keeps_draft_editable_but_blocks_submission_until_finished() {
        let mut state = AppState::new();
        state.start_agent();

        state.insert_char('d');
        state.insert_char('r');
        state.insert_char('a');
        state.insert_char('f');
        state.insert_char('t');
        state.insert_newline();
        state.insert_char('2');
        state.backspace();

        assert_eq!(state.input, "draft\n");
        assert!(!state.submit());

        state.finish_agent();
        assert!(state.submit());
        assert_eq!(state.messages[0].content, "draft\n");
    }

    #[test]
    fn scrolling_clamps_at_both_transcript_bounds() {
        let mut state = AppState::new();
        state.scroll_up(5);
        assert_eq!(state.scroll_offset, 0);

        state.scroll_down(5);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn tool_appends_structured_tool_messages_for_live_calls() {
        let mut state = AppState::new();
        state.tool(
            "list_directory".to_owned(),
            serde_json::json!({"path": "."}),
            serde_json::json!({"status": "ok"}),
        );

        assert_eq!(state.messages.len(), 1);
        let message = &state.messages[0];
        assert_eq!(message.role, Role::Activity);
        assert!(message.content.is_empty());
        assert!(matches!(
            message.kind,
            MessageKind::Tool {
                ref name,
                ref input,
                ref output,
            } if name == "list_directory"
                && input["path"] == "."
                && output["status"] == "ok"
        ));
    }
}
