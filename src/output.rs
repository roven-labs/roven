use std::{
    env,
    fmt::Display,
    io::{self, IsTerminal},
};

const STAGE_COUNT: u8 = 7;

#[derive(Clone, Copy)]
pub(crate) enum Style {
    Success,
    Info,
    Warning,
    Failure,
}

pub(crate) struct InspectionReporter {
    colors: bool,
}

impl InspectionReporter {
    pub(crate) fn new() -> Self {
        Self {
            colors: colors_enabled(
                io::stdout().is_terminal(),
                env::var_os("NO_COLOR").is_some(),
            ),
        }
    }

    pub(crate) fn stage(&self, number: u8, label: &str, style: Style, detail: impl Display) {
        println!(
            "{}",
            self.line(
                format_args!("[{number}/{STAGE_COUNT}] {label:<35}"),
                style,
                detail,
            )
        );
    }

    pub(crate) fn detail(&self, label: &str, value: impl Display) {
        println!("      {label:<35}{value}");
    }

    pub(crate) fn waiting(&self, message: &str) {
        self.detail("", message);
    }

    fn line(&self, prefix: impl Display, style: Style, detail: impl Display) -> String {
        let marker = styled(marker(style), style, self.colors);
        format!("{prefix} {marker} {detail}")
    }
}

fn marker(style: Style) -> &'static str {
    match style {
        Style::Success => "✓",
        Style::Info => "•",
        Style::Warning => "!",
        Style::Failure => "✗",
    }
}

fn style_code(style: Style) -> &'static str {
    match style {
        Style::Success => "32",
        Style::Info => "36",
        Style::Warning => "33",
        Style::Failure => "31",
    }
}

fn styled(text: &str, style: Style, colors: bool) -> String {
    if colors {
        format!("\x1b[{}m{text}\x1b[0m", style_code(style))
    } else {
        text.to_owned()
    }
}

fn colors_enabled(is_terminal: bool, no_color: bool) -> bool {
    is_terminal && !no_color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_status_has_no_ansi_escape_sequences() {
        assert!(!styled("done", Style::Success, false).contains('\x1b'));
    }

    #[test]
    fn styled_status_resets_terminal_style() {
        assert_eq!(styled("done", Style::Success, true), "\x1b[32mdone\x1b[0m");
    }

    #[test]
    fn no_color_disables_styling() {
        assert!(!colors_enabled(true, true));
        assert!(!colors_enabled(false, true));
    }
}
