use std::{
    env,
    fmt::Display,
    io::{self, IsTerminal},
};

use crate::git::{RepositoryValidationBlockers, RepositoryValidationError};

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

pub(crate) fn validation_error_message(error: &RepositoryValidationError) -> Option<String> {
    match error {
        RepositoryValidationError::Dirty { root, blockers } => Some(render_validation_error(
            "PMEMC inspection is blocked",
            root,
            blockers,
            "Commit, stash, remove, or ignore the listed files as appropriate, then retry.",
            colors_enabled(
                io::stderr().is_terminal(),
                env::var_os("NO_COLOR").is_some(),
            ),
        )),
        RepositoryValidationError::MissingCommit { root } => Some(format!(
            "{} PMEMC cannot inspect this repository\n  Repository: {}\n  {} Create at least one commit before inspection.",
            styled(
                "✗",
                Style::Failure,
                colors_enabled(
                    io::stderr().is_terminal(),
                    env::var_os("NO_COLOR").is_some()
                )
            ),
            root.display(),
            styled(
                "•",
                Style::Info,
                colors_enabled(
                    io::stderr().is_terminal(),
                    env::var_os("NO_COLOR").is_some()
                )
            ),
        )),
        RepositoryValidationError::Git(_) => None,
    }
}

fn render_validation_error(
    title: &str,
    root: &std::path::Path,
    blockers: &RepositoryValidationBlockers,
    next_step: &str,
    colors: bool,
) -> String {
    let mut message = format!(
        "{} {title}\n  Repository: {}\n  Blocking conditions:",
        styled("✗", Style::Failure, colors),
        root.display(),
    );
    append_operations(&mut message, &blockers.unfinished_operations, colors);
    append_paths(
        &mut message,
        "Merge conflicts",
        &blockers.conflicted_paths,
        colors,
    );
    append_paths(
        &mut message,
        "Staged changes",
        &blockers.staged_paths,
        colors,
    );
    append_paths(
        &mut message,
        "Unstaged changes",
        &blockers.unstaged_paths,
        colors,
    );
    append_paths(
        &mut message,
        "Untracked files",
        &blockers.untracked_paths,
        colors,
    );
    message.push_str(&format!(
        "\n  {} {next_step}",
        styled("•", Style::Info, colors)
    ));
    message
}

pub(crate) fn print_startup_repository_validation(root: &std::path::Path, head_commit: &str) {
    let colors = colors_enabled(
        io::stdout().is_terminal(),
        env::var_os("NO_COLOR").is_some(),
    );
    println!(
        "{}",
        startup_repository_validation_message(root, head_commit, colors)
    );
}

pub(crate) fn print_startup_registration(project: &crate::storage::Project, outcome: &str) {
    let colors = colors_enabled(
        io::stdout().is_terminal(),
        env::var_os("NO_COLOR").is_some(),
    );
    println!(
        "\n{}",
        startup_registration_message(project, outcome, colors)
    );
}

fn startup_registration_message(
    project: &crate::storage::Project,
    outcome: &str,
    colors: bool,
) -> String {
    format!(
        "[2/2] {}\n  {}\n  Project: {}\n  Repository: {}",
        styled("Project registration", Style::Info, colors),
        styled(&format!("✓ {outcome}"), Style::Success, colors),
        project.name,
        project.canonical_path.display(),
    )
}

fn startup_repository_validation_message(
    root: &std::path::Path,
    head_commit: &str,
    colors: bool,
) -> String {
    format!(
        "[1/2] {}\n  {}\n  {}\n  Repository: {}\n  HEAD:       {head_commit}",
        styled("Repository validation", Style::Info, colors),
        styled("✓ Git repository verified", Style::Success, colors),
        styled("✓ Clean committed state", Style::Success, colors),
        root.display(),
    )
}

fn append_operations(
    message: &mut String,
    operations: &[crate::git::UnfinishedGitOperation],
    colors: bool,
) {
    if operations.is_empty() {
        return;
    }
    message.push_str(&format!(
        "\n    {} {} ({})",
        styled("!", Style::Warning, colors),
        styled("Unfinished Git operations", Style::Warning, colors),
        operations.len(),
    ));
    for operation in operations {
        message.push_str(&format!("\n      - {operation}"));
    }
}

fn append_paths(message: &mut String, label: &str, paths: &[String], colors: bool) {
    if paths.is_empty() {
        return;
    }
    let mut paths = paths.to_vec();
    paths.sort_unstable();
    message.push_str(&format!(
        "\n    {} {} ({})",
        styled("!", Style::Warning, colors),
        styled(label, Style::Warning, colors),
        paths.len(),
    ));
    for path in paths {
        message.push_str(&format!("\n      - {path}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{RepositoryValidationBlockers, UnfinishedGitOperation};

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

    #[test]
    fn startup_repository_validation_uses_terminal_colors() {
        assert_eq!(
            startup_repository_validation_message(std::path::Path::new("C:/repo"), "abc123", true,),
            "[1/2] \x1b[36mRepository validation\x1b[0m\n  \x1b[32m✓ Git repository verified\x1b[0m\n  \x1b[32m✓ Clean committed state\x1b[0m\n  Repository: C:/repo\n  HEAD:       abc123"
        );
    }

    #[test]
    fn validation_error_groups_blockers_and_sorts_paths() {
        let message = render_validation_error(
            "PMEMC inspection is blocked",
            std::path::Path::new("C:/repo"),
            &RepositoryValidationBlockers {
                staged_paths: vec!["zeta.rs".into(), "alpha.rs".into()],
                unstaged_paths: vec!["tracked.rs".into()],
                untracked_paths: vec!["new.rs".into()],
                conflicted_paths: vec!["conflict.rs".into()],
                unfinished_operations: vec![UnfinishedGitOperation::Merge],
            },
            "Fix the repository, then retry.",
            false,
        );

        assert_eq!(
            message,
            "✗ PMEMC inspection is blocked\n  Repository: C:/repo\n  Blocking conditions:\n    ! Unfinished Git operations (1)\n      - merge\n    ! Merge conflicts (1)\n      - conflict.rs\n    ! Staged changes (2)\n      - alpha.rs\n      - zeta.rs\n    ! Unstaged changes (1)\n      - tracked.rs\n    ! Untracked files (1)\n      - new.rs\n  • Fix the repository, then retry."
        );
    }
}
