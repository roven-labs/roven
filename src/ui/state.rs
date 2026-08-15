use std::time::Instant;

use serde_json::Value;

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
    generation_start_index: usize,
    generation_started_at: Option<Instant>,
    pub(crate) resume_entries: Option<Vec<ResumeEntry>>,
    pub(crate) resume_index: usize,
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
        if self.resume_entries.is_none() {
            self.input.push(character);
        }
    }

    pub(crate) fn backspace(&mut self) {
        if self.resume_entries.is_none() {
            self.input.pop();
        }
    }

    pub(crate) fn insert_newline(&mut self) {
        if self.resume_entries.is_none() {
            self.input.push('\n');
        }
    }

    pub(crate) fn submit(&mut self) -> bool {
        if self.running || self.resume_entries.is_some() || self.input.trim().is_empty() {
            return false;
        }

        let content = std::mem::take(&mut self.input);
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
    }

    pub(crate) fn context_usage_percent(&self) -> usize {
        const MAX_CONTEXT_TOKENS: usize = 262_144;
        let characters = self
            .messages
            .iter()
            .map(|message| {
                let tool_characters = match &message.kind {
                    MessageKind::Text => 0,
                    MessageKind::Tool {
                        name,
                        input,
                        output,
                    } => {
                        name.chars().count()
                            + serde_json::to_string(input).map_or(0, |value| value.chars().count())
                            + serde_json::to_string(output).map_or(0, |value| value.chars().count())
                    }
                };
                message.content.chars().count() + tool_characters
            })
            .sum::<usize>();
        let estimated_tokens = characters.saturating_add(3) / 4;
        estimated_tokens
            .saturating_mul(100)
            .checked_div(MAX_CONTEXT_TOKENS)
            .unwrap_or(0)
            .min(100)
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
}

#[cfg(test)]
mod tests {
    use super::{AppState, MessageKind, Role};

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
