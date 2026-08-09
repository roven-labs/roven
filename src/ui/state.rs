pub(crate) const PREVIEW_REPLY: &str =
    "The chat UI is ready; agent replies will appear here later.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Pmemc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) role: Role,
    pub(crate) content: String,
}

#[derive(Debug, Default)]
pub(crate) struct AppState {
    pub(crate) messages: Vec<Message>,
    pub(crate) input: String,
    pub(crate) scroll_offset: u16,
    scroll_limit: u16,
}

impl AppState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert_char(&mut self, character: char) {
        self.input.push(character);
    }

    pub(crate) fn backspace(&mut self) {
        self.input.pop();
    }

    pub(crate) fn insert_newline(&mut self) {
        self.input.push('\n');
    }

    pub(crate) fn submit(&mut self) -> bool {
        if self.input.trim().is_empty() {
            return false;
        }

        let content = std::mem::take(&mut self.input);
        self.messages.push(Message {
            role: Role::User,
            content,
        });
        self.messages.push(Message {
            role: Role::Pmemc,
            content: PREVIEW_REPLY.to_owned(),
        });
        self.scroll_offset = 0;
        true
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
}

#[cfg(test)]
mod tests {
    use super::{AppState, PREVIEW_REPLY, Role};

    #[test]
    fn submit_appends_a_user_turn_and_preview_reply() {
        let mut state = AppState::new();
        for character in "Hello".chars() {
            state.insert_char(character);
        }

        assert!(state.submit());
        assert_eq!(state.messages.len(), 2);
        assert!(matches!(state.messages[0].role, Role::User));
        assert_eq!(state.messages[0].content, "Hello");
        assert_eq!(state.messages[1].content, PREVIEW_REPLY);
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
