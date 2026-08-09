use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Thought,
    Roven,
    Activity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) role: Role,
    pub(crate) content: String,
    pub(crate) duration_ms: Option<u64>,
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
        if !self.running && self.resume_entries.is_none() {
            self.input.push(character);
        }
    }

    pub(crate) fn backspace(&mut self) {
        if !self.running && self.resume_entries.is_none() {
            self.input.pop();
        }
    }

    pub(crate) fn insert_newline(&mut self) {
        if !self.running && self.resume_entries.is_none() {
            self.input.push('\n');
        }
    }

    pub(crate) fn submit(&mut self) -> bool {
        if self.running || self.resume_entries.is_some() || self.input.trim().is_empty() {
            return false;
        }

        let content = std::mem::take(&mut self.input);
        self.messages.push(Message {
            role: Role::User,
            content,
            duration_ms: None,
        });
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
        self.messages.push(Message {
            role: Role::Thought,
            content: text,
            duration_ms: None,
        });
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
        self.messages.push(Message {
            role: Role::Roven,
            content: text,
            duration_ms: None,
        });
    }

    pub(crate) fn finish_agent(&mut self) {
        self.finish_active_thought();
        self.running = false;
        self.status = None;
    }

    pub(crate) fn agent_error(&mut self, message: String) {
        self.finish_agent();
        self.messages.push(Message {
            role: Role::Roven,
            content: format!("Error: {message}"),
            duration_ms: None,
        });
    }

    pub(crate) fn activity(&mut self, message: impl Into<String>) {
        self.messages.push(Message {
            role: Role::Activity,
            content: message.into(),
            duration_ms: None,
        });
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
    use super::{AppState, Role};

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
    fn scrolling_clamps_at_both_transcript_bounds() {
        let mut state = AppState::new();
        state.scroll_up(5);
        assert_eq!(state.scroll_offset, 0);

        state.scroll_down(5);
        assert_eq!(state.scroll_offset, 0);
    }
}
