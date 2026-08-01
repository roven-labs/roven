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
    for command in [
        "init", "project", "status", "inspect", "review", "history", "auth",
    ] {
        assert!(stdout.contains(command), "help should list `{command}`");
    }
    let project_help = pmemc(&["project", "--help"]);
    assert!(project_help.status.success());
    assert!(String::from_utf8_lossy(&project_help.stdout).contains("forget"));
}

#[test]
fn version_identifies_the_pmemc_binary() {
    let output = pmemc(&["--version"]);

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("version should be UTF-8");
    assert!(stdout.starts_with("pmemc "));
}
