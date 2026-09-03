use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus, Stdio};

use restty::config::Config;
use restty::{ColorChoice, RunOptions};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(io::stderr(), "restty: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return Ok(ExitCode::SUCCESS);
    };
    match command.as_str() {
        "render" => render(args.collect()),
        "run" => run_external(args.collect()),
        "live" => live_external(args.collect()),
        "init" => {
            let shell = args.next().ok_or("expected `bash` or `zsh` after `init`")?;
            if args.next().is_some() {
                return Err("too many arguments for `init`".into());
            }
            let config = Config::load();
            let script = restty::shell::init(&shell, &config)
                .ok_or_else(|| format!("unsupported shell `{shell}` (expected bash or zsh)"))?;
            print!("{script}");
            Ok(ExitCode::SUCCESS)
        }
        "--help" | "-h" | "help" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        "--version" | "-V" => {
            println!("restty {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command `{other}`; try `restty --help`")),
    }
}

fn render(args: Vec<String>) -> Result<ExitCode, String> {
    let mut source = None;
    let mut width = None;
    let mut force = false;
    let mut color = ColorChoice::Auto;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if let Some(value) = argument.strip_prefix("--source=") {
            source = Some(value.to_owned());
        } else if argument == "--source" {
            index += 1;
            source = Some(
                args.get(index)
                    .ok_or("missing value after `--source`")?
                    .to_owned(),
            );
        } else if let Some(value) = argument.strip_prefix("--width=") {
            width = Some(parse_width(value)?);
        } else if argument == "--width" {
            index += 1;
            width = Some(parse_width(
                args.get(index).ok_or("missing value after `--width`")?,
            )?);
        } else if argument == "--force" {
            force = true;
        } else if let Some(value) = argument.strip_prefix("--color=") {
            color = parse_color(value)?;
        } else if argument == "--help" || argument == "-h" {
            print_render_help();
            return Ok(ExitCode::SUCCESS);
        } else {
            return Err(format!("unknown render option `{argument}`"));
        }
        index += 1;
    }

    let options = RunOptions {
        source: source.unwrap_or_else(|| "generic".into()),
        width,
        force,
        color,
    };
    let config = Config::load();
    let is_tty = restty::terminal::stdout_is_terminal();
    let stdin = io::stdin();
    let stdout = io::stdout();
    match restty::run_stream(stdin.lock(), stdout.lock(), &options, &config, is_tty) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
        Err(error) => Err(error.to_string()),
    }
}

fn run_external(args: Vec<String>) -> Result<ExitCode, String> {
    let mut source = None;
    let mut width = None;
    let mut force = false;
    let mut color = ColorChoice::Auto;
    let mut command_start = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--" {
            command_start = Some(index + 1);
            break;
        } else if let Some(value) = argument.strip_prefix("--source=") {
            source = Some(value.to_owned());
        } else if argument == "--source" {
            index += 1;
            source = Some(
                args.get(index)
                    .ok_or("missing value after `--source`")?
                    .to_owned(),
            );
        } else if let Some(value) = argument.strip_prefix("--width=") {
            width = Some(parse_width(value)?);
        } else if argument == "--width" {
            index += 1;
            width = Some(parse_width(
                args.get(index).ok_or("missing value after `--width`")?,
            )?);
        } else if argument == "--force" {
            force = true;
        } else if let Some(value) = argument.strip_prefix("--color=") {
            color = parse_color(value)?;
        } else if argument == "--help" || argument == "-h" {
            print_run_help();
            return Ok(ExitCode::SUCCESS);
        } else if argument.starts_with('-') {
            return Err(format!("unknown run option `{argument}`"));
        } else {
            command_start = Some(index);
            break;
        }
        index += 1;
    }

    let command_start = command_start.ok_or("expected a command after `restty run --`")?;
    let command = args
        .get(command_start)
        .ok_or("expected a command after `restty run --`")?;
    let command_args = &args[command_start + 1..];
    let detected_source = source.unwrap_or_else(|| {
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("generic")
            .to_owned()
    });
    let stdout_is_tty = restty::terminal::stdout_is_terminal();

    if should_passthrough_command(&detected_source, command_args)
        || restty::terminal::formatting_disabled()
        || (!force && !stdout_is_tty)
    {
        let status = Command::new(command)
            .args(command_args)
            .status()
            .map_err(|error| format!("could not run `{command}`: {error}"))?;
        return Ok(exit_code(status));
    }

    let mut child = Command::new(command)
        .args(command_args)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run `{command}`: {error}"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("could not capture output from `{command}`"))?;
    let mut config = Config::load();
    if detected_source != "generic"
        && !config
            .enabled_commands
            .iter()
            .any(|enabled| enabled == &detected_source)
    {
        config.enabled_commands.push(detected_source.clone());
    }
    let options = RunOptions {
        source: detected_source,
        width,
        force: true,
        color,
    };
    let stdout = io::stdout();
    let render_result = restty::run_stream(
        child_stdout,
        stdout.lock(),
        &options,
        &config,
        stdout_is_tty,
    );
    let status = child
        .wait()
        .map_err(|error| format!("could not wait for `{command}`: {error}"))?;
    match render_result {
        Ok(()) => Ok(exit_code(status)),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(exit_code(status)),
        Err(error) => Err(error.to_string()),
    }
}

fn live_external(args: Vec<String>) -> Result<ExitCode, String> {
    let mut source = None;
    let mut color = ColorChoice::Auto;
    let mut command_start = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--" {
            command_start = Some(index + 1);
            break;
        } else if let Some(value) = argument.strip_prefix("--source=") {
            source = Some(value.to_owned());
        } else if argument == "--source" {
            index += 1;
            source = Some(
                args.get(index)
                    .ok_or("missing value after `--source`")?
                    .to_owned(),
            );
        } else if let Some(value) = argument.strip_prefix("--color=") {
            color = parse_color(value)?;
        } else if argument == "--help" || argument == "-h" {
            print_live_help();
            return Ok(ExitCode::SUCCESS);
        } else if argument.starts_with('-') {
            return Err(format!("unknown live option `{argument}`"));
        } else {
            command_start = Some(index);
            break;
        }
        index += 1;
    }

    let command_start = command_start.ok_or("expected a command after `restty live --`")?;
    let command = args
        .get(command_start)
        .ok_or("expected a command after `restty live --`")?;
    let command_args = &args[command_start + 1..];
    let detected_source = source.unwrap_or_else(|| {
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("generic")
            .to_owned()
    });

    if should_passthrough_command(&detected_source, command_args)
        || restty::terminal::formatting_disabled()
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
    {
        let status = Command::new(command)
            .args(command_args)
            .status()
            .map_err(|error| format!("could not run `{command}`: {error}"))?;
        return Ok(exit_code(status));
    }

    let mut child = Command::new(command)
        .args(command_args)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run `{command}`: {error}"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("could not capture output from `{command}`"))?;
    let mut config = Config::load();
    if detected_source != "generic"
        && !config
            .enabled_commands
            .iter()
            .any(|enabled| enabled == &detected_source)
    {
        config.enabled_commands.push(detected_source.clone());
    }
    let captured = {
        let stdout = io::stdout();
        capture_for_live(child_stdout, stdout.lock(), &config)
            .map_err(|error| format!("could not capture `{command}`: {error}"))?
    };
    let status = child
        .wait()
        .map_err(|error| format!("could not wait for `{command}`: {error}"))?;
    let Some(captured) = captured else {
        return Ok(exit_code(status));
    };
    if captured.is_empty() || std::str::from_utf8(&captured).is_err() {
        io::stdout()
            .write_all(&captured)
            .map_err(|error| error.to_string())?;
        return Ok(exit_code(status));
    }

    let options = RunOptions {
        source: detected_source,
        width: None,
        force: true,
        color,
    };
    restty::live::view(&captured, &options, &config)
        .map_err(|error| format!("live viewer failed: {error}"))?;
    Ok(exit_code(status))
}

fn capture_for_live<R: Read, W: Write>(
    mut input: R,
    mut passthrough: W,
    config: &Config,
) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut lines = 0_usize;
    loop {
        let read = input.read(&mut chunk)?;
        if read == 0 {
            return Ok(Some(bytes));
        }
        lines += chunk[..read].iter().filter(|byte| **byte == b'\n').count();
        bytes.extend_from_slice(&chunk[..read]);
        if lines > config.max_lines || bytes.len() > config.max_bytes {
            passthrough.write_all(&bytes)?;
            io::copy(&mut input, &mut passthrough)?;
            return Ok(None);
        }
    }
}

fn should_passthrough_command(command: &str, args: &[String]) -> bool {
    if matches!(
        command,
        "vi" | "vim"
            | "nvim"
            | "nano"
            | "emacs"
            | "top"
            | "htop"
            | "btop"
            | "less"
            | "more"
            | "most"
            | "man"
            | "ssh"
            | "mosh"
            | "tmux"
            | "screen"
            | "fzf"
            | "watch"
    ) {
        return true;
    }
    let follows = args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-f" | "--follow"));
    follows
        && matches!(
            command,
            "tail" | "journalctl" | "dmesg" | "docker" | "podman" | "kubectl"
        )
}

fn exit_code(status: ExitStatus) -> ExitCode {
    if let Some(code) = status.code() {
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return ExitCode::from((128 + signal).clamp(1, 255) as u8);
        }
    }
    ExitCode::FAILURE
}

fn parse_width(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|width| *width > 0)
        .ok_or_else(|| format!("invalid terminal width `{value}`"))
}

fn parse_color(value: &str) -> Result<ColorChoice, String> {
    match value {
        "auto" => Ok(ColorChoice::Auto),
        "always" => Ok(ColorChoice::Always),
        "never" => Ok(ColorChoice::Never),
        _ => Err(format!(
            "invalid color mode `{value}` (expected auto, always, or never)"
        )),
    }
}

fn print_help() {
    println!(
        "restty — responsive terminal output\n\nUSAGE:\n  restty live [OPTIONS] -- <command> [args...]\n  restty run [OPTIONS] -- <command> [args...]\n  restty render [OPTIONS] < input\n  restty init <bash|zsh>\n\nTry `restty live -- ps aux` or run a subcommand with --help."
    );
}

fn print_run_help() {
    println!(
        "USAGE:\n  restty run [--source NAME] [--width COLS] [--color auto|always|never] [--force] -- <command> [args...]\n\nRuns any non-interactive command through the built-in or generic formatter. Interactive and follow-mode commands are passed through untouched."
    );
}

fn print_live_help() {
    println!(
        "USAGE:\n  restty live [--source NAME] [--color auto|always|never] -- <command> [args...]\n\nKeeps completed output in an alternate-screen viewer and redraws it whenever the terminal is resized. Use arrows or j/k to scroll, Page Up/Down for pages, Home/End to jump, and q, Esc, or Ctrl-C to quit."
    );
}

fn print_render_help() {
    println!(
        "USAGE:\n  restty render [--source NAME] [--width COLS] [--color auto|always|never] [--force]\n\nWithout --force, redirected stdout and disable environment variables are exact passthroughs."
    );
}
