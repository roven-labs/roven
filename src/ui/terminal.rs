use std::io;

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

use super::{state::AppState, view};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopAction {
    Redraw,
    Exit,
}

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

    loop {
        terminal.draw(|frame| view::draw(frame, &mut state))?;
        if handle_event(&mut state, event::read()?) == LoopAction::Exit {
            return Ok(());
        }
    }
}

pub(crate) fn handle_event(state: &mut AppState, event: Event) -> LoopAction {
    match event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) => LoopAction::Exit,
        Event::Key(KeyEvent {
            code: KeyCode::Char(character),
            modifiers,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) if !modifiers.contains(KeyModifiers::CONTROL) => {
            state.insert_char(character);
            LoopAction::Redraw
        }
        Event::Key(KeyEvent {
            code: KeyCode::Backspace,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) => {
            state.backspace();
            LoopAction::Redraw
        }
        Event::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) if modifiers.contains(KeyModifiers::ALT) => {
            state.insert_newline();
            LoopAction::Redraw
        }
        Event::Key(KeyEvent {
            code: KeyCode::Enter,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) => {
            state.submit();
            LoopAction::Redraw
        }
        Event::Key(KeyEvent {
            code: KeyCode::PageUp,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        })
        | Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            ..
        }) => {
            state.scroll_up(3);
            LoopAction::Redraw
        }
        Event::Key(KeyEvent {
            code: KeyCode::PageDown,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        })
        | Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            ..
        }) => {
            state.scroll_down(3);
            LoopAction::Redraw
        }
        _ => LoopAction::Redraw,
    }
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
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

    use super::{LoopAction, handle_event};
    use crate::ui::state::AppState;

    #[test]
    fn character_input_requests_redraw_and_updates_composer() {
        let mut state = AppState::new();

        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            ),
            LoopAction::Redraw
        );
        assert_eq!(state.input, "a");
    }

    #[test]
    fn enter_submits_and_alt_enter_adds_a_newline() {
        let mut state = AppState::new();
        handle_event(
            &mut state,
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        );

        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            ),
            LoopAction::Redraw
        );
        assert_eq!(state.input, "a\n");
        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            ),
            LoopAction::Redraw
        );
        assert_eq!(state.messages.len(), 2);
    }

    #[test]
    fn ctrl_c_requests_immediate_exit() {
        let mut state = AppState::new();

        assert_eq!(
            handle_event(
                &mut state,
                Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            ),
            LoopAction::Exit
        );
    }

    #[test]
    fn scrolling_and_resize_request_redraw() {
        let mut state = AppState::new();
        let scroll_up = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(handle_event(&mut state, scroll_up), LoopAction::Redraw);
        assert_eq!(
            handle_event(&mut state, Event::Resize(80, 24)),
            LoopAction::Redraw
        );
    }
}
