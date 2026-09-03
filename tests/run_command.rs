use std::process::Command;

#[test]
fn universal_runner_formats_generic_tables() {
    let output = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args([
            "run",
            "--force",
            "--color=never",
            "--width=80",
            "--",
            "printf",
            "NAME          READY   AGE\napi-server    1/1     12m\nworker        0/1     3d\n",
        ])
        .env_remove("NO_COLOR")
        .env_remove("RESTTY_DISABLE")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with('╭'));
    assert!(stdout.contains("api-server"));
    assert!(stdout.ends_with("╯\n"));
}

#[test]
fn universal_runner_preserves_child_exit_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args([
            "run",
            "--force",
            "--color=never",
            "--",
            "sh",
            "-c",
            "printf 'NAME  VALUE\\nalpha  one\\n'; exit 37",
        ])
        .env_remove("NO_COLOR")
        .env_remove("RESTTY_DISABLE")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(37));
    assert!(String::from_utf8(output.stdout).unwrap().starts_with('╭'));
}

#[test]
fn live_mode_degrades_to_direct_execution_without_a_tty() {
    let output = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args(["live", "--", "printf", "plain output\\n"])
        .env_remove("NO_COLOR")
        .env_remove("RESTTY_DISABLE")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"plain output\n");
}
