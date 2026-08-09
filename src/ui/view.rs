use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::state::{AppState, Role};

pub(crate) const MINIMUM_WIDTH: u16 = 40;
pub(crate) const MINIMUM_HEIGHT: u16 = 8;

const USER_STYLE: Style = Style::new().fg(Color::Cyan);
const ROVEN_STYLE: Style = Style::new().fg(Color::Green);
const MUTED_STYLE: Style = Style::new().fg(Color::DarkGray);
const STATUS_STYLE: Style = Style::new().fg(Color::LightCyan);
const TRUST_TITLE_STYLE: Style = Style::new().fg(Color::LightBlue);
const TRUST_PATH_STYLE: Style = Style::new().fg(Color::Yellow);
const TRUST_BODY_STYLE: Style = Style::new().fg(Color::Gray);
const TRUST_SELECTED_STYLE: Style = Style::new().fg(Color::LightCyan);
const TRUST_UNSELECTED_STYLE: Style = Style::new().fg(Color::DarkGray);

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

    if !state.trusted {
        let content_area = area.inner(Margin {
            horizontal: 2,
            vertical: 0,
        });
        let yes_marker = if state.trust_yes_selected { ">" } else { " " };
        let no_marker = if state.trust_yes_selected { " " } else { ">" };
        let path = if state.project_path.is_empty() {
            "Current folder"
        } else {
            display_workspace_path(&state.project_path)
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("ROVEN · WORKSPACE", TRUST_TITLE_STYLE)),
                Line::from(""),
                Line::from(Span::styled(path, TRUST_PATH_STYLE)),
                Line::from(""),
                Line::from(Span::styled(
                    "Trust this workspace? Roven accesses it only for this open session.",
                    TRUST_BODY_STYLE,
                )),
                Line::from(""),
                Line::from(Span::styled("READ-ONLY ACCESS", MUTED_STYLE)),
                Line::from(Span::styled(
                    "Read files · search content · inspect Git and CodeGraph",
                    TRUST_BODY_STYLE,
                )),
                Line::from(Span::styled(
                    "No file edits · no command execution · no external tools",
                    TRUST_BODY_STYLE,
                )),
                Line::from(""),
                Line::from(Span::styled("CHOOSE", MUTED_STYLE)),
                Line::from(""),
                Line::from(Span::styled(
                    format!("{yes_marker} Trust and start"),
                    if state.trust_yes_selected {
                        TRUST_SELECTED_STYLE
                    } else {
                        TRUST_UNSELECTED_STYLE
                    },
                )),
                Line::from(Span::styled(
                    format!("{no_marker} Exit Roven"),
                    if state.trust_yes_selected {
                        TRUST_UNSELECTED_STYLE
                    } else {
                        TRUST_SELECTED_STYLE
                    },
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "↑ ↓ select  ·  Enter confirm  ·  Esc cancel",
                    MUTED_STYLE,
                )),
            ])
            .wrap(Wrap { trim: false }),
            content_area,
        );
        return;
    }

    if let Some(entries) = &state.resume_entries {
        let items = if entries.is_empty() {
            "No previous sessions for this project".to_owned()
        } else {
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let marker = if index == state.resume_index {
                        ">"
                    } else {
                        " "
                    };
                    format!("{marker} {}", entry.title)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        frame.render_widget(
            Paragraph::new(format!(
                "Resume conversation\n\n{items}\n\nEnter resume   Esc back"
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(MUTED_STYLE),
            ),
            area,
        );
        return;
    }

    let composer_height = composer_height(&state.input);
    let status_height = u16::from(state.status.is_some());
    let [transcript_area, status_area, composer_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(status_height),
        Constraint::Length(composer_height),
    ])
    .areas(area);

    draw_transcript(frame, transcript_area, state);
    draw_status_bar(frame, status_area, state);
    draw_composer(frame, composer_area, state);
}

fn display_workspace_path(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

fn composer_height(input: &str) -> u16 {
    input.split('\n').count().clamp(1, 4) as u16 + 2
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
        .flat_map(|message| match message.role {
            Role::Thought => vec![
                Line::from(Span::styled(
                    match message.duration_ms {
                        Some(duration_ms) => format!("Thought: {duration_ms}ms"),
                        None => "Thought".to_owned(),
                    },
                    MUTED_STYLE,
                )),
                Line::from(Span::styled(message.content.clone(), MUTED_STYLE)),
                Line::default(),
            ],
            role => {
                let (label, style) = match role {
                    Role::User => ("You", USER_STYLE),
                    Role::Roven => ("Roven", ROVEN_STYLE),
                    Role::Activity => ("Roven", MUTED_STYLE),
                    Role::Thought => unreachable!("thought is rendered above"),
                };
                vec![
                    Line::from(vec![
                        Span::styled(format!("{label} › "), style),
                        Span::raw(message.content.clone()),
                    ]),
                    Line::default(),
                ]
            }
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

fn draw_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some(status) = &state.status else {
        return;
    };
    frame.render_widget(Paragraph::new(Span::styled(status, STATUS_STYLE)), area);
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
    if !state.running {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
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
        state.project_path = r"\\?\C:\Users\visha\roven".to_owned();
        let rendered = render(&mut state, 80, 24);

        assert!(rendered.contains("Trust this workspace?"));
        assert!(rendered.contains(r"C:\Users\visha\roven"));
        assert!(!rendered.contains(r"\\?\C:\Users\visha\roven"));
        assert!(rendered.contains("READ-ONLY ACCESS"));
        assert!(rendered.contains("Trust and start"));
        assert!(rendered.contains("Exit Roven"));
    }

    #[test]
    fn populated_screen_renders_left_aligned_turns() {
        let mut state = AppState::new();
        state.trusted = true;
        for character in "Hello".chars() {
            state.insert_char(character);
        }
        assert!(state.submit());

        let rendered = render(&mut state, 80, 24);

        assert!(rendered.contains("You › Hello"));
        assert!(!rendered.contains("Roven"));
        assert!(!rendered.contains("Project agent"));
        assert!(!rendered.contains("The chat UI is ready"));
    }

    #[test]
    fn provider_thought_renders_as_a_muted_transcript_block() {
        let mut state = AppState::new();
        state.trusted = true;
        state.start_agent();
        state.append_thought("Check the request.".to_owned());
        state.append_agent_text("Here is the answer.".to_owned());
        let rendered = render(&mut state, 80, 24);

        assert!(rendered.contains("Thought:"));
        assert!(rendered.contains("Check the request."));
        assert!(rendered.contains("Roven › Here is the answer."));
    }

    #[test]
    fn active_agent_renders_a_dedicated_status_bar_until_completion() {
        let mut state = AppState::new();
        state.trusted = true;
        state.start_agent();
        assert!(render(&mut state, 80, 24).contains("Agent working..."));

        state.append_thought("Inspecting the request.".to_owned());
        assert!(render(&mut state, 80, 24).contains("Thinking..."));

        state.finish_agent();
        assert!(!render(&mut state, 80, 24).contains("Thinking..."));
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
        state.trusted = true;
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
        state.trusted = true;

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
