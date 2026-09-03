use std::io::Write;
use std::process::{Command, Stdio};

fn run_with_env(input: &[u8], key: &str, value: &str) -> Vec<u8> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_restty"))
        .args([
            "render",
            "--source=generic",
            "--force",
            "--width=80",
            "--color=never",
        ])
        .env(key, value)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    output.stdout
}

#[test]
fn no_color_and_disable_are_exact_passthroughs_even_with_force() {
    let input = b"NAME  VALUE\nalpha  one\nbeta   two\n";
    assert_eq!(run_with_env(input, "NO_COLOR", "1"), input);
    assert_eq!(run_with_env(input, "RESTTY_DISABLE", "1"), input);
}
