use std::process::Command;

fn pmemc(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args(arguments)
        .output()
        .expect("pmemc binary should run")
}

#[test]
fn help_describes_the_version_one_command_surface() {
    let output = pmemc(&["--help"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Project Memory CLI"));
    assert!(stdout.contains("auth"));
    for legacy_command in ["init", "project", "status", "inspect", "review", "history"] {
        assert!(
            !stdout.contains(legacy_command),
            "help must not expose legacy `{legacy_command}`"
        );
    }
}

#[test]
fn bare_invocation_displays_local_help_without_starting_a_session() {
    let output = pmemc(&[]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Usage: pmemc [COMMAND]"));
    assert!(stdout.contains("auth"));
    assert!(!stdout.contains("session"));
    assert!(!stdout.contains("pmemc>"));
    assert!(!stdout.contains("pmemc_prepare_project"));
}

#[test]
fn auth_is_the_only_retained_command_surface() {
    let output = pmemc(&["auth", "--help"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    for command in ["set", "status", "remove"] {
        assert!(stdout.contains(command), "auth must expose `{command}`");
    }
    for removed_command in ["project", "study", "model", "prepare", "codegraph"] {
        assert!(
            !stdout.contains(removed_command),
            "auth help must not expose `{removed_command}`"
        );
    }
}

#[test]
fn version_identifies_the_pmemc_binary() {
    let output = pmemc(&["--version"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("version should be UTF-8");
    assert!(stdout.starts_with("pmemc "));
}
