use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::state::{AppState, Role};

pub(crate) const MINIMUM_WIDTH: u16 = 40;
pub(crate) const MINIMUM_HEIGHT: u16 = 8;

const HEADER_STYLE: Style = Style::new().fg(Color::Blue);
const USER_STYLE: Style = Style::new().fg(Color::Cyan);
const PMEMC_STYLE: Style = Style::new().fg(Color::Green);
const MUTED_STYLE: Style = Style::new().fg(Color::DarkGray);

pub(crate) fn draw(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();
    if area.width < MINIMUM_WIDTH || area.height < MINIMUM_HEIGHT {
        frame.render_widget(
            Paragraph::new("Resize terminal to continue")
                .style(MUTED_STYLE)
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let composer_height = composer_height(&state.input);
    let [header_area, transcript_area, composer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(composer_height),
    ])
    .areas(area);

    draw_header(frame, header_area);
    draw_transcript(frame, transcript_area, state);
    draw_composer(frame, composer_area, state);
}

fn composer_height(input: &str) -> u16 {
    input.split('\n').count().clamp(1, 4) as u16 + 2
}

fn draw_header(frame: &mut Frame, area: Rect) {
    let [name_area, _, preview_area] = Layout::horizontal([
        Constraint::Length(5),
        Constraint::Min(1),
        Constraint::Length(10),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new("PMEMC").style(HEADER_STYLE), name_area);
    frame.render_widget(
        Paragraph::new("UI Preview")
            .style(MUTED_STYLE)
            .alignment(Alignment::Right),
        preview_area,
    );
}

fn draw_transcript(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if state.messages.is_empty() {
        frame.render_widget(
            Paragraph::new("Start a conversation")
                .style(MUTED_STYLE)
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let lines = state
        .messages
        .iter()
        .flat_map(|message| {
            let (label, style) = match message.role {
                Role::User => ("You", USER_STYLE),
                Role::Pmemc => ("PMEMC", PMEMC_STYLE),
            };
            [
                Line::from(vec![
                    Span::styled(format!("{label} › "), style),
                    Span::raw(message.content.clone()),
                ]),
                Line::default(),
            ]
        })
        .collect::<Vec<_>>();
    let line_count = lines.len().min(u16::MAX as usize) as u16;
    let maximum_scroll = line_count.saturating_sub(area.height);
    state.set_scroll_limit(maximum_scroll);
    let scroll_from_top = maximum_scroll.saturating_sub(state.scroll_offset);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_from_top, 0))
            .wrap(Wrap { trim: false }),
        area,
    );

    if state.is_scrolled_away_from_latest() && area.width > 1 {
        let indicator_area = Rect::new(area.x + area.width - 1, area.y, 1, area.height);
        frame.render_widget(Paragraph::new("│").style(MUTED_STYLE), indicator_area);
    }
}

fn draw_composer(frame: &mut Frame, area: Rect, state: &AppState) {
    let visible_input = visible_composer_text(&state.input);
    let composer = Paragraph::new(visible_input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(MUTED_STYLE),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(composer, area);

    let visible_lines = visible_input.split('\n').collect::<Vec<_>>();
    let last_line_width = visible_lines
        .last()
        .map_or(0, |line| line.chars().count().min(u16::MAX as usize) as u16);
    let cursor_x = area
        .x
        .saturating_add(1)
        .saturating_add(last_line_width)
        .min(area.x.saturating_add(area.width).saturating_sub(2));
    let cursor_y = area
        .y
        .saturating_add(visible_lines.len().saturating_sub(1) as u16)
        .saturating_add(1);
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn visible_composer_text(input: &str) -> String {
    let lines = input.split('\n').collect::<Vec<_>>();
    let first_visible_line = lines.len().saturating_sub(4);
    lines[first_visible_line..].join("\n")
}

#[cfg(test)]
mod tests {
    use ratatui::{
        Terminal,
        backend::{Backend, TestBackend},
        layout::Position,
    };

    use super::{MINIMUM_HEIGHT, MINIMUM_WIDTH, draw};
    use crate::ui::state::AppState;

    fn render(state: &mut AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| draw(frame, state))
            .expect("frame should render");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn empty_screen_has_header_and_centered_prompt() {
        let mut state = AppState::new();
        let rendered = render(&mut state, 80, 24);

        assert!(rendered.contains("PMEMC"));
        assert!(rendered.contains("UI Preview"));
        assert!(rendered.contains("Start a conversation"));
    }

    #[test]
    fn populated_screen_renders_left_aligned_turns() {
        let mut state = AppState::new();
        for character in "Hello".chars() {
            state.insert_char(character);
        }
        assert!(state.submit());

        let rendered = render(&mut state, 80, 24);

        assert!(rendered.contains("You › Hello"));
        assert!(rendered.contains("PMEMC › The chat UI is ready"));
    }

    #[test]
    fn undersized_screen_requests_resize() {
        let mut state = AppState::new();
        let rendered = render(&mut state, MINIMUM_WIDTH - 1, MINIMUM_HEIGHT - 1);

        assert!(rendered.contains("Resize terminal to continue"));
    }

    #[test]
    fn composer_keeps_the_newest_four_explicit_lines_visible() {
        let mut state = AppState::new();
        state.input = [
            "first-composer-line",
            "second-composer-line",
            "third-composer-line",
            "fourth-composer-line",
            "fifth-composer-line",
        ]
        .join("\n");

        let rendered = render(&mut state, 80, 24);

        assert!(!rendered.contains("first-composer-line"));
        assert!(rendered.contains("fifth-composer-line"));
    }

    #[test]
    fn composer_places_the_terminal_cursor_inside_its_border() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let mut state = AppState::new();

        terminal
            .draw(|frame| draw(frame, &mut state))
            .expect("frame should render");

        assert_eq!(
            terminal
                .backend_mut()
                .get_cursor_position()
                .expect("test backend should report cursor position"),
            Position::new(1, 22)
        );
    }
}
