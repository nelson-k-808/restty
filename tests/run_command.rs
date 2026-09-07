use std::io::Write;
use std::process::{Command, Stdio};

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
    assert!(stdout.starts_with('╭') || stdout.starts_with('+'));
    assert!(stdout.contains("api-server"));
    assert!(stdout.ends_with("╯\n") || stdout.ends_with("+\n"));
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with('╭') || stdout.starts_with('+'));
}

#[test]
fn border_override_selects_the_requested_style() {
    let output = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args([
            "run",
            "--force",
            "--border=double",
            "--color=never",
            "--",
            "printf",
            "NAME  VALUE\nalpha  one\n",
        ])
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C.UTF-8")
        .env_remove("NO_COLOR")
        .env_remove("RESTTY_DISABLE")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().starts_with('╔'));
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

#[test]
fn structured_render_and_run_work_without_a_tty() {
    let input = b"NAME  VALUE\nalpha  one\n";
    let mut render = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args(["render", "--source=generic", "--format=json"])
        .env_remove("NO_COLOR")
        .env_remove("RESTTY_DISABLE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    render.stdin.take().unwrap().write_all(input).unwrap();
    let output = render.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[\n  {\"name\": \"alpha\", \"value\": \"one\"}\n]\n"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args([
            "run",
            "--source=generic",
            "--format=csv",
            "--",
            "printf",
            "NAME  VALUE\nalpha  one\n",
        ])
        .env("XDG_CACHE_HOME", std::env::temp_dir())
        .env_remove("NO_COLOR")
        .env_remove("RESTTY_DISABLE")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"name,value\nalpha,one\n");
}

#[test]
fn cache_snapshot_last_and_diff_form_a_workflow() {
    let cache = std::env::temp_dir().join(format!(
        "restty-cache-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("workflow")
    ));
    let run = |value: &str| {
        Command::new(env!("CARGO_BIN_EXE_restty"))
            .args([
                "run",
                "--source=generic",
                "--force",
                "--color=never",
                "--",
                "printf",
                &format!("NAME  VALUE\nalpha  {value}\n"),
            ])
            .env("XDG_CACHE_HOME", &cache)
            .env_remove("NO_COLOR")
            .env_remove("RESTTY_DISABLE")
            .output()
            .unwrap()
    };
    assert!(run("one").status.success());
    let snapshot = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args(["snapshot", "baseline"])
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert!(snapshot.status.success());
    assert!(run("two").status.success());
    let last = Command::new(env!("CARGO_BIN_EXE_restty"))
        .arg("last")
        .env("XDG_CACHE_HOME", &cache)
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert!(String::from_utf8(last.stdout).unwrap().contains("two"));
    let diff = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args([
            "diff",
            "baseline",
            "--",
            "printf",
            "NAME  VALUE\nalpha  two\n",
        ])
        .env("XDG_CACHE_HOME", &cache)
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert!(String::from_utf8(diff.stdout)
        .unwrap()
        .contains("changes: +0 ~1 -0"));
    let _ = std::fs::remove_dir_all(cache);
}

#[test]
fn interactive_modes_execute_once_when_redirected() {
    for mode in ["explore", "watch"] {
        let output = Command::new(env!("CARGO_BIN_EXE_restty"))
            .args([mode, "--", "printf", "plain output\n"])
            .env_remove("NO_COLOR")
            .env_remove("RESTTY_DISABLE")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"plain output\n");
    }
}
