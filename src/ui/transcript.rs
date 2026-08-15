use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::state::{Message, MessageKind, Role};

const USER_STYLE: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
const MODEL_STYLE: Style = Style::new().fg(Color::Gray);
const MUTED_STYLE: Style = Style::new().fg(Color::DarkGray);
const CODE_STYLE: Style = Style::new().fg(Color::Gray);
const HEADING_STYLE: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
const TOOL_STYLE: Style = Style::new().fg(Color::DarkGray);
const USER_CONTINUATION_PREFIX: &str = "│     ";
const MODEL_LABEL: &str = "│ ";
const MODEL_CONTINUATION_PREFIX: &str = "┆       ";
const ACTIVITY_LABEL: &str = "Activity ";

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
    let (label, continuation_prefix, label_style, body_style) = match role {
        Role::User => (
            "You › ",
            USER_CONTINUATION_PREFIX.to_owned(),
            USER_STYLE,
            MODEL_STYLE,
        ),
        Role::Roven => (
            MODEL_LABEL,
            MODEL_CONTINUATION_PREFIX.to_owned(),
            MODEL_STYLE,
            MODEL_STYLE,
        ),
        Role::Activity => (
            ACTIVITY_LABEL,
            " ".repeat(ACTIVITY_LABEL.width()),
            MUTED_STYLE,
            MUTED_STYLE,
        ),
        Role::Thought => unreachable!("thought is handled above"),
    };
    render_markdown(
        &raw,
        label,
        &continuation_prefix,
        label_style,
        body_style,
        width,
    )
}

fn render_thought(raw: &str, duration_ms: Option<u64>, width: usize) -> Vec<Line<'static>> {
    let title = match duration_ms {
        Some(duration_ms) => format!("Thought: {duration_ms}ms"),
        None => "Thought".to_owned(),
    };
    let mut lines = vec![Line::from(Span::styled(title, MUTED_STYLE))];
    lines.extend(render_markdown(
        raw,
        "",
        "",
        MUTED_STYLE,
        MUTED_STYLE,
        width,
    ));
    lines.push(Line::default());
    lines
}

fn render_tool(name: &str, input: &Value, output: &Value, width: usize) -> Vec<Line<'static>> {
    let card = describe_tool(name, input, output);
    let mut lines = Vec::with_capacity(card.len() + 3);
    lines.push(Line::from(Span::styled(tool_box_top(width), TOOL_STYLE)));
    for content in card {
        lines.push(Line::from(Span::styled(
            tool_box_line(&content, width),
            TOOL_STYLE,
        )));
    }
    lines.push(Line::from(Span::styled(tool_box_bottom(width), TOOL_STYLE)));
    lines.push(Line::default());
    lines
}

fn tool_title(name: &str) -> String {
    let title = title_case_words(&humanize_identifier(name));
    if title.is_empty() {
        "Completed requested action".to_owned()
    } else {
        title
    }
}

fn tool_box_top(width: usize) -> String {
    match width {
        0 => String::new(),
        1 => "┌".to_owned(),
        2 => "┌┐".to_owned(),
        _ => format!("┌{}┐", "─".repeat(width - 2)),
    }
}

fn tool_box_line(content: &str, width: usize) -> String {
    match width {
        0 => String::new(),
        1 => "│".to_owned(),
        2 => "││".to_owned(),
        _ => {
            let inner_width = width - 2;
            let content = clip_to_width(content, inner_width.saturating_sub(2));
            let right_padding = inner_width.saturating_sub(content.width() + 1);
            format!("│ {content}{}│", " ".repeat(right_padding))
        }
    }
}

fn tool_box_bottom(width: usize) -> String {
    match width {
        0 => String::new(),
        1 => "└".to_owned(),
        2 => "└┘".to_owned(),
        _ => format!("└{}┘", "─".repeat(width - 2)),
    }
}

fn render_markdown(
    raw: &str,
    label: &str,
    continuation_prefix: &str,
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
    let mut emitted_label = false;

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
                    let first_prefix =
                        message_block_prefix(label, continuation_prefix, "", &mut emitted_label);
                    lines.extend(render_table(
                        finished.rows,
                        &first_prefix,
                        continuation_prefix,
                        label_style,
                        width,
                    ));
                }
                _ => {}
            }
            continue;
        }

        if let Some((_, content)) = &mut code_block {
            match event {
                Event::End(TagEnd::CodeBlock) => {
                    let (_, content) = code_block.take().expect("code block state exists");
                    flush_inline_block(
                        &mut lines,
                        &mut current,
                        "",
                        label,
                        continuation_prefix,
                        label_style,
                        width,
                        &mut emitted_label,
                    );
                    let block_prefix =
                        message_block_prefix(label, continuation_prefix, "", &mut emitted_label);
                    let code_prefix = format!("{block_prefix}  ");
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
                if quote_depth == 0 {
                    current_prefix.clear();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                let prefix = current_prefix.clone();
                flush_inline_block(
                    &mut lines,
                    &mut current,
                    &prefix,
                    label,
                    continuation_prefix,
                    label_style,
                    width,
                    &mut emitted_label,
                );
                lines.push(Line::default());
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let _ = level;
                current_style = HEADING_STYLE;
                current_prefix.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let prefix = current_prefix.clone();
                flush_inline_block(
                    &mut lines,
                    &mut current,
                    &prefix,
                    label,
                    continuation_prefix,
                    label_style,
                    width,
                    &mut emitted_label,
                );
                current_style = body_style;
                lines.push(Line::default());
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_inline_block(
                    &mut lines,
                    &mut current,
                    "",
                    label,
                    continuation_prefix,
                    label_style,
                    width,
                    &mut emitted_label,
                );
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
                flush_inline_block(
                    &mut lines,
                    &mut current,
                    &prefix,
                    label,
                    continuation_prefix,
                    label_style,
                    width,
                    &mut emitted_label,
                );
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
                flush_inline_block(
                    &mut lines,
                    &mut current,
                    "",
                    label,
                    continuation_prefix,
                    label_style,
                    width,
                    &mut emitted_label,
                );
                table = Some(TableState {
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    current_cell: String::new(),
                });
            }
            Event::Rule => {
                flush_inline_block(
                    &mut lines,
                    &mut current,
                    "",
                    label,
                    continuation_prefix,
                    label_style,
                    width,
                    &mut emitted_label,
                );
                let rule_prefix =
                    message_block_prefix(label, continuation_prefix, "", &mut emitted_label);
                lines.push(Line::from(vec![
                    Span::styled(rule_prefix.to_owned(), label_style),
                    Span::styled("  ─────────────".to_owned(), MUTED_STYLE),
                ]));
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
                text: if checked { "☒ " } else { "☐ " }.to_owned(),
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

    flush_inline_block(
        &mut lines,
        &mut current,
        &current_prefix,
        label,
        continuation_prefix,
        label_style,
        width,
        &mut emitted_label,
    );
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
    first_prefix: &str,
    continuation_prefix: &str,
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
    let first_available = width.saturating_sub(first_prefix.width());
    let continuation_available = width.saturating_sub(continuation_prefix.width());
    let minimum_available = first_available.min(continuation_available);
    let total = widths.iter().sum::<usize>() + columns.saturating_mul(3).saturating_sub(1);
    if total > minimum_available {
        return rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let text = row.join(" | ");
                let prefix = if index == 0 {
                    first_prefix
                } else {
                    continuation_prefix
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
                first_prefix
            } else {
                continuation_prefix
            };
            Line::from(vec![
                Span::styled(prefix.to_owned(), label_style),
                Span::styled(text, CODE_STYLE),
            ])
        })
        .collect()
}

fn block_continuation_prefix(continuation_prefix: &str, local_prefix: &str) -> String {
    format!("{continuation_prefix}{}", " ".repeat(local_prefix.width()))
}

fn message_block_prefix(
    label: &str,
    continuation_prefix: &str,
    local_prefix: &str,
    emitted_label: &mut bool,
) -> String {
    if !*emitted_label {
        *emitted_label = true;
        format!("{label}{local_prefix}")
    } else {
        format!("{continuation_prefix}{local_prefix}")
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_inline_block(
    lines: &mut Vec<Line<'static>>,
    current: &mut Vec<InlineText>,
    prefix: &str,
    label: &str,
    continuation_prefix: &str,
    label_style: Style,
    width: usize,
    emitted_label: &mut bool,
) {
    if current.is_empty() {
        return;
    }
    let first_prefix = message_block_prefix(label, continuation_prefix, prefix, emitted_label);
    let continuation_prefix = block_continuation_prefix(continuation_prefix, prefix);
    lines.extend(wrap_inline(
        current,
        width,
        &first_prefix,
        &continuation_prefix,
        label_style,
    ));
    current.clear();
}

fn wrap_inline(
    spans: &[InlineText],
    width: usize,
    first_prefix: &str,
    continuation_prefix: &str,
    prefix_style: Style,
) -> Vec<Line<'static>> {
    let first_available = available_width(width, first_prefix);
    let continuation_available = available_width(width, continuation_prefix);
    let mut lines = Vec::new();
    let mut current = Vec::<InlineText>::new();
    let mut current_width = 0usize;
    let mut current_available = first_available;

    for span in spans {
        for part in span.text.split_inclusive('\n') {
            let explicit_break = part.ends_with('\n');
            let text = part.trim_end_matches('\n');
            for word in text.split_inclusive(char::is_whitespace) {
                let word_width = word.width();
                if current_width > 0 && current_width + word_width > current_available {
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
                    current_available = continuation_available;
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
                current_available = continuation_available;
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

fn available_width(width: usize, prefix: &str) -> usize {
    width.saturating_sub(prefix.width()).max(1)
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

fn describe_tool(name: &str, input: &Value, output: &Value) -> Vec<String> {
    let mut lines = vec![tool_title(name), format!("Status: {}", tool_status(output))];
    lines.extend(tool_detail_lines(input, output));
    lines
}

fn tool_status(output: &Value) -> String {
    output
        .get("status")
        .and_then(Value::as_str)
        .map(title_case_words)
        .unwrap_or_else(|| "Completed".to_owned())
}

fn tool_detail_lines(input: &Value, output: &Value) -> Vec<String> {
    let mut details = Vec::new();

    if let Some(reason) = output.get("reason").and_then(Value::as_str) {
        details.push(format!("Reason: {}", humanize_identifier(reason)));
    }

    let mut saw_path = false;
    if let Some(path) = output
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| input.get("path").and_then(Value::as_str))
    {
        details.push(format!("Path: {path}"));
        saw_path = true;
    }

    if let Some(entries) = output.get("entries").and_then(Value::as_array) {
        let entry_count = entries.len();
        details.push(format!(
            "Entries: {}",
            count_label(entry_count, "item", "items")
        ));
        if output.get("truncated").and_then(Value::as_bool) == Some(true) {
            details.push(format!(
                "Details: Truncated after {}",
                count_label(entry_count, "item", "items")
            ));
        }
    }

    if let Some(content) = output.get("content").and_then(Value::as_str) {
        details.push(format!(
            "Content: {}, {}",
            count_label(line_count(content), "line", "lines"),
            count_label(content.chars().count(), "char", "chars")
        ));
    }

    if let Some(tools) = output.get("tools").and_then(Value::as_array) {
        details.push(format!(
            "Tools: {}",
            count_label(tools.len(), "tool", "tools")
        ));
    }

    if let Some(project) = output.get("project").and_then(Value::as_object) {
        if let Some(name) = project.get("name").and_then(Value::as_str) {
            details.push(format!("Project: {name}"));
        }
        if !saw_path {
            if let Some(path) = project.get("path").and_then(Value::as_str) {
                details.push(format!("Path: {path}"));
            }
        }
    }

    details
}

fn line_count(value: &str) -> usize {
    if value.is_empty() {
        0
    } else {
        value.lines().count()
    }
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

fn title_case_words(value: &str) -> String {
    humanize_identifier(value)
        .split_whitespace()
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanize_identifier(value: &str) -> String {
    value
        .trim()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_word(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => {
            let mut capitalized = first.to_uppercase().collect::<String>();
            capitalized.push_str(characters.as_str());
            capitalized
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::render_message;
    use crate::ui::state::{Message, MessageKind, Role};
    use unicode_width::UnicodeWidthStr;

    fn text(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_owned(),
            duration_ms: None,
            kind: MessageKind::Text,
        }
    }

    #[test]
    fn multiline_output_uses_role_specific_prefixes_and_continuations() {
        let user_lines = render_message(&text(Role::User, "first line\nsecond line"), 40);
        let roven_lines = render_message(&text(Role::Roven, "first line\nsecond line"), 40);
        let activity_lines =
            render_message(&text(Role::Activity, "agent working\nstill working"), 40);

        assert_eq!(user_lines[0].to_string(), "You › first line");
        assert_eq!(user_lines[1].to_string(), "│     second line");
        assert_eq!(roven_lines[0].to_string(), "│ first line");
        assert_eq!(roven_lines[1].to_string(), "┆       second line");
        assert_eq!(activity_lines[0].to_string(), "Activity agent working");
        assert!(activity_lines[1].to_string().ends_with("still working"));
    }

    #[test]
    fn wrapped_model_lines_stay_within_requested_width() {
        let width = 14;
        let lines = render_message(&text(Role::Roven, "one two three four five"), width);

        assert!(lines.len() > 2);
        assert!(lines.iter().all(|line| line.to_string().width() <= width));
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

        assert!(lines.iter().any(|line| line.to_string().contains("Files")));
        assert!(
            !lines
                .iter()
                .any(|line| line.to_string().contains("# Files"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("• backend"))
        );
        assert!(lines.iter().any(|line| line.to_string().contains("ready")));

        let task_lines = render_message(
            &text(Role::Roven, "- [x] read project\n- [ ] prepare context"),
            60,
        );
        assert!(
            task_lines
                .iter()
                .any(|line| line.to_string().contains("☒ read project"))
        );
        assert!(
            task_lines
                .iter()
                .any(|line| line.to_string().contains("☐ prepare context"))
        );
    }

    #[test]
    fn wrapped_markdown_keeps_role_gutter_and_local_prefix() {
        let user_lines = render_message(
            &text(
                Role::User,
                "- user item that wraps across multiple words nicely",
            ),
            28,
        );
        assert!(user_lines[0].to_string().starts_with("You › • "));
        assert!(
            user_lines
                .iter()
                .skip(1)
                .any(|line| line.to_string().starts_with("│       "))
        );

        let roven_task_lines = render_message(
            &text(
                Role::Roven,
                "- [x] roven task item that wraps across multiple words nicely",
            ),
            28,
        );
        assert!(roven_task_lines[0].to_string().starts_with("│ • ☒ "));
        assert!(
            roven_task_lines
                .iter()
                .skip(1)
                .any(|line| line.to_string().starts_with("┆         "))
        );

        let roven_quote_lines = render_message(
            &text(
                Role::Roven,
                "> roven quote block that wraps across multiple words nicely",
            ),
            28,
        );
        assert!(
            roven_quote_lines
                .iter()
                .any(|line| line.to_string().starts_with("│ │ "))
        );
        assert!(
            roven_quote_lines
                .iter()
                .skip(1)
                .any(|line| line.to_string().starts_with("┆         "))
        );
    }

    #[test]
    fn structured_tool_output_is_rendered_separately() {
        let message = Message::tool(
            "list_directory".to_owned(),
            serde_json::json!({"path": "."}),
            serde_json::json!({
                "status": "ok",
                "path": ".",
                "workspace_path": ".",
                "entries": [
                    {"name": "src", "path": "src", "kind": "directory"}
                ],
                "truncated": true
            }),
        );
        let lines = render_message(&message, 60);

        assert!(lines[0].to_string().starts_with("┌"));
        assert!(lines[1].to_string().starts_with("│ List Directory"));
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("Status: Ok"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("Path: ."))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("Entries: 1 item"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("Details: Truncated after 1 item"))
        );
        assert!(lines.iter().all(|line| line.to_string().width() <= 60));
        assert!(!lines.iter().any(|line| line.to_string().contains("Roven")));
        assert!(!lines.iter().any(|line| line.to_string().contains("◆")));
        assert!(!lines.iter().any(|line| line.to_string().contains("input")));
        assert!(!lines.iter().any(|line| line.to_string().contains("output")));
        assert!(
            !lines
                .iter()
                .any(|line| line.to_string().contains("list_directory"))
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.to_string().contains("\"name\""))
        );
        assert!(lines.iter().any(|line| line.to_string().starts_with("└")));
        assert!(lines.last().is_some_and(|line| line.spans.is_empty()));
    }

    #[test]
    fn model_table_lines_stay_within_requested_width() {
        let width = 14;
        let lines = render_message(
            &text(Role::Roven, "| A | B |\n| --- | --- |\n| wide | row |"),
            width,
        );

        assert!(lines.iter().all(|line| line.to_string().width() <= width));
    }

    #[test]
    fn structured_tool_output_clamps_to_narrow_width() {
        let message = Message::tool(
            "list_directory".to_owned(),
            serde_json::json!({"path": "."}),
            serde_json::json!({"status": "ok"}),
        );
        let width = 12;
        let lines = render_message(&message, width);

        assert!(lines.iter().any(|line| line.to_string().starts_with("┌")));
        assert!(lines.iter().any(|line| line.to_string().starts_with("└")));
        assert!(lines.iter().all(|line| line.to_string().width() <= width));
    }

    #[test]
    fn role_label_is_emitted_once_per_message() {
        let lines = render_message(
            &text(
                Role::Roven,
                "First paragraph.\n\n- later item\n\n| A | B |\n| --- | --- |\n| 1 | 2 |",
            ),
            48,
        );

        assert!(lines[0].to_string().starts_with("│ First paragraph."));
        assert!(
            !lines
                .iter()
                .skip(1)
                .any(|line| line.to_string().starts_with("│ "))
        );
        assert!(
            lines
                .iter()
                .skip(1)
                .any(|line| line.to_string().starts_with("┆       • later item"))
        );
        assert!(
            lines
                .iter()
                .skip(1)
                .any(|line| line.to_string().starts_with("┆") && line.to_string().contains("A"))
        );
    }

    #[test]
    fn model_and_tool_output_have_no_branding_or_icon_labels() {
        let model_lines = render_message(&text(Role::Roven, "Answer"), 40);
        let tool_lines = render_message(
            &Message::tool(
                "read_file".to_owned(),
                serde_json::json!({"path": "secret.txt"}),
                serde_json::json!({
                    "status": "ok",
                    "path": "secret.txt",
                    "content": "top secret\nsecond line"
                }),
            ),
            40,
        );

        for line in model_lines.iter().chain(tool_lines.iter()) {
            let rendered = line.to_string();
            assert!(!rendered.contains("Roven"));
            assert!(!rendered.contains("◆"));
        }
        assert!(
            tool_lines
                .iter()
                .any(|line| line.to_string().contains("Content: 2 lines, 22 chars"))
        );
        assert!(
            !tool_lines
                .iter()
                .any(|line| line.to_string().contains("top secret"))
        );
    }

    #[test]
    fn tool_labels_are_friendly_and_unknown_tools_use_a_safe_fallback() {
        for (name, label) in [
            ("list_directory", "List Directory"),
            ("read_file", "Read File"),
            ("prepare_project", "Prepare Project"),
            ("list_tools", "List Tools"),
            ("custom_tool", "Custom Tool"),
        ] {
            let lines = render_message(
                &Message::tool(
                    name.to_owned(),
                    serde_json::json!({"path": "workspace"}),
                    serde_json::json!({"status": "unknown_tool", "reason": "unknown_tool"}),
                ),
                48,
            );
            assert!(lines[1].to_string().contains(label));
            assert!(!lines[1].to_string().contains(name));
        }

        let fallback_lines = render_message(
            &Message::tool(
                "".to_owned(),
                serde_json::Value::Null,
                serde_json::json!({"status": "error"}),
            ),
            48,
        );
        assert!(
            fallback_lines[1]
                .to_string()
                .contains("Completed requested action")
        );
    }

    #[test]
    fn tool_cards_render_structured_status_and_details_for_known_and_unknown_tools() {
        let prepared_lines = render_message(
            &Message::tool(
                "prepare_project".to_owned(),
                serde_json::json!({"path": "workspace"}),
                serde_json::json!({
                    "status": "prepared",
                    "project": {
                        "name": "pmemc",
                        "path": "workspace",
                        "github_remote": "origin",
                        "baseline_commit": "abc123"
                    }
                }),
            ),
            64,
        );
        assert!(
            prepared_lines
                .iter()
                .any(|line| line.to_string().contains("Status: Prepared"))
        );
        assert!(
            prepared_lines
                .iter()
                .any(|line| line.to_string().contains("Project: pmemc"))
        );
        assert!(
            prepared_lines
                .iter()
                .any(|line| line.to_string().contains("Path: workspace"))
        );

        let unknown_lines = render_message(
            &Message::tool(
                "custom_tool".to_owned(),
                serde_json::json!({"path": "notes.md"}),
                serde_json::json!({"status": "error", "reason": "unknown_tool"}),
            ),
            64,
        );
        assert!(
            unknown_lines
                .iter()
                .any(|line| line.to_string().contains("Status: Error"))
        );
        assert!(
            unknown_lines
                .iter()
                .any(|line| line.to_string().contains("Reason: unknown tool"))
        );
        assert!(
            unknown_lines
                .iter()
                .any(|line| line.to_string().contains("Custom Tool"))
        );
    }

    #[test]
    fn tool_cards_keep_a_blank_separator_before_the_next_turn() {
        let tool_lines = render_message(
            &Message::tool(
                "read_file".to_owned(),
                serde_json::json!({"path": "notes.md"}),
                serde_json::json!({
                    "status": "ok",
                    "path": "notes.md",
                    "content": "alpha\nbeta"
                }),
            ),
            48,
        );
        let separator_index = tool_lines.len() - 1;
        let mut combined = tool_lines.clone();
        combined.extend(render_message(&text(Role::Roven, "next turn"), 48));

        assert!(combined[separator_index].spans.is_empty());
        assert_eq!(combined[separator_index + 1].to_string(), "│ next turn");
    }
}
