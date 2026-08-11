use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::state::{Message, MessageKind, Role};

const USER_STYLE: Style = Style::new().fg(Color::Cyan);
const ROVEN_STYLE: Style = Style::new().fg(Color::Green);
const MUTED_STYLE: Style = Style::new().fg(Color::DarkGray);
const CODE_STYLE: Style = Style::new().fg(Color::Yellow);
const HEADING_STYLE: Style = Style::new()
    .fg(Color::LightCyan)
    .add_modifier(Modifier::BOLD);
const TOOL_STYLE: Style = Style::new().fg(Color::DarkGray);

#[derive(Debug, Clone)]
struct InlineText {
    text: String,
    style: Style,
}

#[derive(Debug, Clone)]
struct TableState {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
}

pub(crate) fn render_message(message: &Message, width: usize) -> Vec<Line<'static>> {
    match &message.kind {
        MessageKind::Text => {
            render_text(message.role, &message.content, message.duration_ms, width)
        }
        MessageKind::Tool {
            name,
            input,
            output,
        } => render_tool(name, input, output, width),
    }
}

fn render_text(
    role: Role,
    raw: &str,
    duration_ms: Option<u64>,
    width: usize,
) -> Vec<Line<'static>> {
    let raw = normalize_line_endings(raw);
    if role == Role::Thought {
        return render_thought(&raw, duration_ms, width);
    }
    let (label, label_style, body_style) = match role {
        Role::User => ("You › ", USER_STYLE, USER_STYLE),
        Role::Roven => ("Roven › ", ROVEN_STYLE, ROVEN_STYLE),
        Role::Activity => ("Roven › ", MUTED_STYLE, MUTED_STYLE),
        Role::Thought => unreachable!("thought is handled above"),
    };
    render_markdown(&raw, label, label_style, body_style, width)
}

fn render_thought(raw: &str, duration_ms: Option<u64>, width: usize) -> Vec<Line<'static>> {
    let title = match duration_ms {
        Some(duration_ms) => format!("Thought: {duration_ms}ms"),
        None => "Thought".to_owned(),
    };
    let mut lines = vec![Line::from(Span::styled(title, MUTED_STYLE))];
    lines.extend(render_markdown(raw, "", MUTED_STYLE, MUTED_STYLE, width));
    lines.push(Line::default());
    lines
}

fn render_tool(name: &str, input: &Value, output: &Value, width: usize) -> Vec<Line<'static>> {
    let prefix = "  ";
    let mut lines = vec![Line::from(Span::styled(
        format!("Roven · {name} completed"),
        TOOL_STYLE,
    ))];
    lines.extend(render_json_block("input", input, prefix, width));
    lines.extend(render_json_block("output", output, prefix, width));
    lines.push(Line::default());
    lines
}

fn render_json_block(label: &str, value: &Value, prefix: &str, width: usize) -> Vec<Line<'static>> {
    let json = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_owned());
    let available = width.saturating_sub(prefix.chars().count()).max(1);
    let mut lines = vec![Line::from(Span::styled(
        format!("{prefix}{label}:"),
        TOOL_STYLE,
    ))];
    for raw_line in json.lines() {
        let content = if raw_line.is_empty() { " " } else { raw_line };
        lines.push(Line::from(vec![
            Span::raw(prefix.to_owned()),
            Span::styled(clip_to_width(content, available), CODE_STYLE),
        ]));
    }
    lines
}

fn render_markdown(
    raw: &str,
    label: &str,
    label_style: Style,
    body_style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current = Vec::<InlineText>::new();
    let mut current_prefix = String::new();
    let mut current_style = body_style;
    let mut style_stack = Vec::new();
    let mut code_block: Option<(Option<String>, String)> = None;
    let mut list_stack: Vec<(bool, usize)> = Vec::new();
    let mut quote_depth = 0usize;
    let mut table: Option<TableState> = None;

    let flush_inline =
        |lines: &mut Vec<Line<'static>>, current: &mut Vec<InlineText>, prefix: &str| {
            if current.is_empty() {
                return;
            }
            let first_prefix = format!("{label}{prefix}");
            let continuation_prefix = " ".repeat(first_prefix.chars().count());
            lines.extend(wrap_inline(
                current,
                width,
                &first_prefix,
                &continuation_prefix,
                label_style,
            ));
            current.clear();
        };

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    for event in Parser::new_ext(raw, options) {
        if let Some(table_state) = &mut table {
            match event {
                Event::Start(Tag::TableRow) | Event::Start(Tag::TableHead) => {
                    table_state.current_row.clear();
                }
                Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                    if !table_state.current_row.is_empty() {
                        table_state
                            .rows
                            .push(std::mem::take(&mut table_state.current_row));
                    }
                }
                Event::Start(Tag::TableCell) => table_state.current_cell.clear(),
                Event::End(TagEnd::TableCell) => {
                    table_state
                        .current_row
                        .push(std::mem::take(&mut table_state.current_cell));
                }
                Event::Text(text) | Event::Code(text) => table_state.current_cell.push_str(&text),
                Event::SoftBreak | Event::HardBreak => table_state.current_cell.push(' '),
                Event::End(TagEnd::Table) => {
                    let finished = table.take().expect("table state exists");
                    lines.extend(render_table(finished.rows, label, label_style, width));
                }
                _ => {}
            }
            continue;
        }

        if let Some((_, content)) = &mut code_block {
            match event {
                Event::End(TagEnd::CodeBlock) => {
                    let (_, content) = code_block.take().expect("code block state exists");
                    flush_inline(&mut lines, &mut current, "");
                    let code_prefix = format!("{}  ", " ".repeat(label.width()));
                    for line in content.lines() {
                        lines.push(Line::from(vec![
                            Span::styled(code_prefix.clone(), body_style),
                            Span::styled(line.to_owned(), CODE_STYLE),
                        ]));
                    }
                    lines.push(Line::default());
                }
                Event::Text(text) => content.push_str(&text),
                Event::SoftBreak | Event::HardBreak => content.push('\n'),
                _ => {}
            }
            continue;
        }

        match event {
            Event::Start(Tag::Paragraph) => {
                current_prefix.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                let prefix = current_prefix.clone();
                flush_inline(&mut lines, &mut current, &prefix);
                lines.push(Line::default());
            }
            Event::Start(Tag::Heading { level, .. }) => {
                current_style = HEADING_STYLE;
                current_prefix = format!("{} ", "#".repeat(heading_number(level)));
            }
            Event::End(TagEnd::Heading(_)) => {
                let prefix = current_prefix.clone();
                flush_inline(&mut lines, &mut current, &prefix);
                current_style = body_style;
                lines.push(Line::default());
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_inline(&mut lines, &mut current, "");
                let language = match kind {
                    CodeBlockKind::Fenced(language) => Some(language.to_string()),
                    CodeBlockKind::Indented => None,
                };
                code_block = Some((language, String::new()));
            }
            Event::Start(Tag::List(start)) => {
                list_stack.push((start.is_some(), start.unwrap_or(1) as usize));
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                lines.push(Line::default());
            }
            Event::Start(Tag::Item) => {
                let (ordered, number) = list_stack.last_mut().copied().unwrap_or((false, 0));
                let marker = if ordered {
                    if let Some((_, next)) = list_stack.last_mut() {
                        *next = next.saturating_add(1);
                    }
                    format!("{number}. ")
                } else {
                    "• ".to_owned()
                };
                current_prefix = format!(
                    "{}{}",
                    "  ".repeat(list_stack.len().saturating_sub(1)),
                    marker
                );
            }
            Event::End(TagEnd::Item) => {
                let prefix = current_prefix.clone();
                flush_inline(&mut lines, &mut current, &prefix);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                quote_depth += 1;
                current_prefix = format!("{}│ ", "  ".repeat(quote_depth.saturating_sub(1)));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                quote_depth = quote_depth.saturating_sub(1);
                lines.push(Line::default());
            }
            Event::Start(Tag::Table(_)) => {
                flush_inline(&mut lines, &mut current, "");
                table = Some(TableState {
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    current_cell: String::new(),
                });
            }
            Event::Rule => {
                flush_inline(&mut lines, &mut current, "");
                lines.push(Line::from(Span::styled("  ─────────────", MUTED_STYLE)));
                lines.push(Line::default());
            }
            Event::Text(text) => current.push(InlineText {
                text: text.to_string(),
                style: current_style,
            }),
            Event::Code(text) => current.push(InlineText {
                text: text.to_string(),
                style: CODE_STYLE,
            }),
            Event::SoftBreak | Event::HardBreak => current.push(InlineText {
                text: "\n".to_owned(),
                style: current_style,
            }),
            Event::Start(Tag::Strong) => {
                style_stack.push(current_style);
                current_style = current_style.add_modifier(Modifier::BOLD);
            }
            Event::End(TagEnd::Strong) => current_style = style_stack.pop().unwrap_or(body_style),
            Event::Start(Tag::Emphasis) => {
                style_stack.push(current_style);
                current_style = current_style.add_modifier(Modifier::ITALIC);
            }
            Event::End(TagEnd::Emphasis) => current_style = style_stack.pop().unwrap_or(body_style),
            Event::Start(Tag::Strikethrough) => {
                style_stack.push(current_style);
                current_style = current_style.add_modifier(Modifier::CROSSED_OUT);
            }
            Event::End(TagEnd::Strikethrough) => {
                current_style = style_stack.pop().unwrap_or(body_style)
            }
            Event::TaskListMarker(checked) => current.push(InlineText {
                text: if checked { "[x] " } else { "[ ] " }.to_owned(),
                style: current_style,
            }),
            Event::Html(html) | Event::InlineHtml(html) => current.push(InlineText {
                text: html.to_string(),
                style: current_style,
            }),
            Event::FootnoteReference(reference) => current.push(InlineText {
                text: format!("[^{reference}]"),
                style: current_style,
            }),
            Event::Start(Tag::Link { .. }) => {
                style_stack.push(current_style);
                current_style = current_style.add_modifier(Modifier::UNDERLINED);
            }
            Event::End(TagEnd::Link) => current_style = style_stack.pop().unwrap_or(body_style),
            Event::Start(Tag::Image { .. }) => current.push(InlineText {
                text: "![".to_owned(),
                style: current_style,
            }),
            Event::End(TagEnd::Image) => current.push(InlineText {
                text: "]".to_owned(),
                style: current_style,
            }),
            _ => {}
        }
    }

    flush_inline(&mut lines, &mut current, &current_prefix);
    while lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }
    if label.is_empty() && lines.is_empty() {
        lines.push(Line::default());
    }
    lines.push(Line::default());
    lines
}

fn render_table(
    rows: Vec<Vec<String>>,
    label: &str,
    label_style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::default()];
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.width());
        }
    }
    let label_width = label.width();
    let total = widths.iter().sum::<usize>() + columns.saturating_mul(3).saturating_sub(1);
    if total > width.saturating_sub(label_width) {
        let continuation_prefix = " ".repeat(label_width);
        return rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let text = row.join(" | ");
                let prefix = if index == 0 {
                    label
                } else {
                    &continuation_prefix
                };
                Line::from(vec![
                    Span::styled(prefix.to_owned(), label_style),
                    Span::styled(
                        clip_to_width(&text, width.saturating_sub(prefix.width())),
                        CODE_STYLE,
                    ),
                ])
            })
            .collect();
    }
    let continuation_prefix = " ".repeat(label_width);
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let text = row
                .into_iter()
                .enumerate()
                .map(|(column, cell)| {
                    let padding = widths[column].saturating_sub(cell.width());
                    format!("{cell}{}", " ".repeat(padding))
                })
                .collect::<Vec<_>>()
                .join(" │ ");
            let prefix = if index == 0 {
                label
            } else {
                &continuation_prefix
            };
            Line::from(vec![
                Span::styled(prefix.to_owned(), label_style),
                Span::styled(text, CODE_STYLE),
            ])
        })
        .collect()
}

fn wrap_inline(
    spans: &[InlineText],
    width: usize,
    first_prefix: &str,
    continuation_prefix: &str,
    prefix_style: Style,
) -> Vec<Line<'static>> {
    let prefix_width = first_prefix.width();
    let available = width.saturating_sub(prefix_width).max(1);
    let mut lines = Vec::new();
    let mut current = Vec::<InlineText>::new();
    let mut current_width = 0usize;

    for span in spans {
        for part in span.text.split_inclusive('\n') {
            let explicit_break = part.ends_with('\n');
            let text = part.trim_end_matches('\n');
            for word in text.split_inclusive(char::is_whitespace) {
                let word_width = word.width();
                if current_width > 0 && current_width + word_width > available {
                    let line_prefix = if lines.is_empty() {
                        first_prefix
                    } else {
                        continuation_prefix
                    };
                    let style = lines.is_empty().then_some(prefix_style);
                    lines.push(line_from_inline(
                        std::mem::take(&mut current),
                        line_prefix,
                        style,
                    ));
                    current_width = 0;
                }
                if !word.is_empty() {
                    current.push(InlineText {
                        text: word.to_owned(),
                        style: span.style,
                    });
                    current_width += word_width;
                }
            }
            if explicit_break {
                let line_prefix = if lines.is_empty() {
                    first_prefix
                } else {
                    continuation_prefix
                };
                let style = lines.is_empty().then_some(prefix_style);
                lines.push(line_from_inline(
                    std::mem::take(&mut current),
                    line_prefix,
                    style,
                ));
                current_width = 0;
            }
        }
    }
    if !current.is_empty() || lines.is_empty() {
        let line_prefix = if lines.is_empty() {
            first_prefix
        } else {
            continuation_prefix
        };
        let style = lines.is_empty().then_some(prefix_style);
        lines.push(line_from_inline(current, line_prefix, style));
    }
    lines
}

fn line_from_inline(
    spans: Vec<InlineText>,
    prefix: &str,
    prefix_style: Option<Style>,
) -> Line<'static> {
    let prefix = match prefix_style {
        Some(style) => Span::styled(prefix.to_owned(), style),
        None => Span::raw(prefix.to_owned()),
    };
    let mut line = Line::from(prefix);
    for span in spans {
        line.spans.push(Span::styled(span.text, span.style));
    }
    line
}

fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn heading_number(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn clip_to_width(value: &str, width: usize) -> String {
    let limit = width.max(1);
    let mut used = 0;
    let mut clipped = String::new();
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > limit {
            break;
        }
        clipped.push(character);
        used += character_width;
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::render_message;
    use crate::ui::state::{Message, MessageKind, Role};

    fn text(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_owned(),
            duration_ms: None,
            kind: MessageKind::Text,
        }
    }

    #[test]
    fn multiline_output_preserves_rows_and_aligns_continuations() {
        let lines = render_message(&text(Role::Roven, "first line\nsecond line"), 40);

        assert_eq!(lines[0].to_string(), "Roven › first line");
        assert_eq!(lines[1].to_string(), "        second line");
    }

    #[test]
    fn fenced_code_preserves_tree_spacing() {
        let lines = render_message(
            &text(Role::Roven, "```text\n├── src/\n│   └── main.rs\n```"),
            40,
        );

        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("├── src/"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("│   └── main.rs"))
        );
    }

    #[test]
    fn markdown_blocks_render_as_structured_terminal_lines() {
        let lines = render_message(
            &text(
                Role::Roven,
                "# Files\n\n- backend\n- frontend\n\n| Area | Status |\n| --- | --- |\n| UI | ready |",
            ),
            60,
        );

        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("# Files"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("• backend"))
        );
        assert!(lines.iter().any(|line| line.to_string().contains("ready")));
    }

    #[test]
    fn structured_tool_output_is_rendered_separately() {
        let message = Message::tool(
            "list_directory".to_owned(),
            serde_json::json!({"path": "."}),
            serde_json::json!({"status": "ok", "entries": ["src"]}),
        );
        let lines = render_message(&message, 60);

        assert!(lines[0].to_string().contains("list_directory completed"));
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("entries"))
        );
    }
}
