use std::process::Command;

fn roven(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_roven"))
        .args(arguments)
        .output()
        .expect("roven binary should run")
}

#[test]
fn help_describes_the_current_command_surface() {
    let output = roven(&["--help"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("PMEMC"));
    assert!(stdout.contains("pmemc"));
    assert!(stdout.contains("auth"));
    for legacy_command in ["init", "project", "status", "inspect", "review", "history"] {
        assert!(
            !stdout.contains(legacy_command),
            "help must not expose legacy `{legacy_command}`"
        );
    }
}

#[test]
fn auth_is_the_only_retained_command_surface() {
    let output = roven(&["auth", "--help"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    for command in ["set", "list", "use", "status", "remove"] {
        assert!(stdout.contains(command), "auth must expose `{command}`");
    }
    for removed_command in ["project", "study", "prepare"] {
        assert!(
            !stdout.contains(removed_command),
            "auth help must not expose `{removed_command}`"
        );
    }
}

#[test]
fn version_identifies_the_pmemc_binary() {
    let output = roven(&["--version"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("version should be UTF-8");
    assert!(stdout.starts_with("pmemc "));
}
