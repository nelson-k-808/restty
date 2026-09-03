use restty::config::Config;
use restty::{render_bytes_with_policy, ColorChoice, RunOptions};

fn options(source: &str) -> RunOptions {
    RunOptions {
        source: source.into(),
        width: Some(80),
        force: false,
        color: ColorChoice::Never,
    }
}

#[test]
fn redirected_stdout_is_exact_passthrough() {
    let input = b"NAME  VALUE\nalpha  one\n";
    assert_eq!(
        render_bytes_with_policy(input, &options("generic"), &Config::default(), false, false,),
        input
    );
}

#[test]
fn malformed_and_binary_inputs_are_exact_passthrough() {
    let malformed = b"this is not a confidently parseable table\nsecond prose line\n";
    let binary = b"NAME  VALUE\nalpha  \xff\n";
    let mut forced = options("generic");
    forced.force = true;
    assert_eq!(
        render_bytes_with_policy(malformed, &forced, &Config::default(), true, false),
        malformed
    );
    assert_eq!(
        render_bytes_with_policy(binary, &forced, &Config::default(), true, false),
        binary
    );
}

#[test]
fn line_and_byte_limits_are_exact_passthrough() {
    let input = b"NAME  VALUE\nalpha  one\nbeta   two\n";
    let config = Config {
        max_lines: 1,
        max_bytes: 8,
        ..Config::default()
    };
    let mut forced = options("generic");
    forced.force = true;
    assert_eq!(
        render_bytes_with_policy(input, &forced, &config, true, false),
        input
    );
}

#[test]
fn disabled_command_is_exact_passthrough() {
    let input = b"4.0K\t./src\n120K\t./target\n";
    let mut config = Config::default();
    config.enabled_commands.retain(|command| command != "du");
    let mut forced = options("du");
    forced.force = true;
    assert_eq!(
        render_bytes_with_policy(input, &forced, &config, true, false),
        input
    );
}
