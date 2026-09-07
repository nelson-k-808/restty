use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use restty::config::Config;
use restty::structured::StructuredFormat;
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
        "live" => live_external(args.collect(), false),
        "explore" => live_external(args.collect(), true),
        "last" => last_cached(args.collect()),
        "snapshot" => snapshot_cached(args.collect()),
        "diff" => diff_cached(args.collect()),
        "watch" => watch_external(args.collect()),
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

fn watch_external(args: Vec<String>) -> Result<ExitCode, String> {
    let mut source = None;
    let mut color = ColorChoice::Auto;
    let mut border = None;
    let mut interval = Duration::from_secs(2);
    let mut alert = None;
    let mut command_start = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--" {
            command_start = Some(index + 1);
            break;
        } else if let Some(value) = argument.strip_prefix("--interval=") {
            interval = parse_interval(value)?;
        } else if argument == "--interval" {
            index += 1;
            interval = parse_interval(args.get(index).ok_or("missing value after `--interval`")?)?;
        } else if let Some(value) = argument.strip_prefix("--alert=") {
            alert = Some(value.to_owned());
        } else if argument == "--alert" {
            index += 1;
            alert = Some(
                args.get(index)
                    .ok_or("missing value after `--alert`")?
                    .to_owned(),
            );
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
        } else if let Some(value) = argument.strip_prefix("--border=") {
            border = Some(parse_border(value)?);
        } else if argument == "--border" {
            index += 1;
            border = Some(parse_border(
                args.get(index).ok_or("missing value after `--border`")?,
            )?);
        } else if argument == "--help" || argument == "-h" {
            print_watch_help();
            return Ok(ExitCode::SUCCESS);
        } else if argument.starts_with('-') {
            return Err(format!("unknown watch option `{argument}`"));
        } else {
            command_start = Some(index);
            break;
        }
        index += 1;
    }
    let command_start = command_start.ok_or("expected a command after `restty watch --`")?;
    let command = args
        .get(command_start)
        .ok_or("expected a command after `--`")?;
    let command_args = &args[command_start + 1..];
    let detected_source = source.unwrap_or_else(|| {
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("generic")
            .to_owned()
    });
    if should_passthrough_command(&detected_source, command_args) {
        return Err(format!(
            "`{command}` is interactive or unbounded and cannot be watched"
        ));
    }
    if restty::terminal::formatting_disabled()
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
    {
        let status = Command::new(command)
            .args(command_args)
            .status()
            .map_err(|error| format!("could not run `{command}`: {error}"))?;
        return Ok(exit_code(status));
    }
    let mut config = Config::load();
    if let Some(border) = border {
        config.border_style = border;
    }
    if !config
        .enabled_commands
        .iter()
        .any(|enabled| enabled == &detected_source)
    {
        config.enabled_commands.push(detected_source.clone());
    }
    let options = RunOptions {
        source: detected_source,
        width: None,
        force: true,
        color,
    };
    let code = restty::live::watch_command(
        command,
        command_args,
        interval,
        alert.as_deref(),
        &options,
        &config,
    )
    .map_err(|error| format!("watch failed: {error}"))?;
    Ok(ExitCode::from(code))
}

fn last_cached(args: Vec<String>) -> Result<ExitCode, String> {
    if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!(
            "USAGE:\n  restty last\n\nRe-emits the most recently cached parsed command result."
        );
        return Ok(ExitCode::SUCCESS);
    }
    if !args.is_empty() {
        return Err("`restty last` does not accept arguments".into());
    }
    let Some(entry) = restty::cache::latest().map_err(|error| error.to_string())? else {
        return Ok(ExitCode::SUCCESS);
    };
    let mut config = Config::load();
    if !config
        .enabled_commands
        .iter()
        .any(|enabled| enabled == &entry.source)
    {
        config.enabled_commands.push(entry.source.clone());
    }
    let options = RunOptions {
        source: entry.source,
        width: None,
        force: true,
        color: ColorChoice::Auto,
    };
    let output = restty::render_bytes(
        &entry.raw,
        &options,
        &config,
        restty::terminal::stdout_is_terminal(),
    );
    io::stdout()
        .write_all(&output)
        .map_err(|error| error.to_string())?;
    Ok(ExitCode::SUCCESS)
}

fn snapshot_cached(args: Vec<String>) -> Result<ExitCode, String> {
    if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!(
            "USAGE:\n  restty snapshot <name>\n\nCopies the most recent cache entry to a named, non-overwritten snapshot."
        );
        return Ok(ExitCode::SUCCESS);
    }
    let [name] = args.as_slice() else {
        return Err("expected exactly one snapshot name".into());
    };
    match restty::cache::snapshot(name) {
        Ok(true) => {
            println!("saved snapshot `{name}`");
            Ok(ExitCode::SUCCESS)
        }
        Ok(false) => Ok(ExitCode::SUCCESS),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(format!("snapshot `{name}` already exists"))
        }
        Err(error) => Err(error.to_string()),
    }
}

fn diff_cached(args: Vec<String>) -> Result<ExitCode, String> {
    if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!(
            "USAGE:\n  restty diff <name> [-- <command> [args...]]\n\nCompares a named snapshot with the latest cached result, or with a newly executed bounded command."
        );
        return Ok(ExitCode::SUCCESS);
    }
    let Some(name) = args.first() else {
        return Err("expected a snapshot name".into());
    };
    let Some(snapshot) = restty::cache::named_snapshot(name).map_err(|error| error.to_string())?
    else {
        return Ok(ExitCode::SUCCESS);
    };
    let config = Config::load();
    let (current, source, status) = if args.len() == 1 {
        if snapshot.argv.is_empty() {
            let Some(current) = restty::cache::latest().map_err(|error| error.to_string())? else {
                return Ok(ExitCode::SUCCESS);
            };
            (current.raw, current.source, None)
        } else {
            let command = &snapshot.argv[0];
            let command_args = &snapshot.argv[1..];
            let (captured, source, status) = capture_diff_command(command, command_args, &config)?;
            let Some(captured) = captured else {
                return Ok(exit_code(status));
            };
            (captured, source, Some(status))
        }
    } else {
        if args.get(1).map(String::as_str) != Some("--") {
            return Err("expected `--` before the command".into());
        }
        let command = args.get(2).ok_or("expected a command after `--`")?;
        let command_args = &args[3..];
        let (captured, source, status) = capture_diff_command(command, command_args, &config)?;
        let Some(captured) = captured else {
            return Ok(exit_code(status));
        };
        (captured, source, Some(status))
    };
    let mut active_config = config;
    if !active_config
        .enabled_commands
        .iter()
        .any(|item| item == &source)
    {
        active_config.enabled_commands.push(source.clone());
    }
    let options = RunOptions {
        source: source.clone(),
        width: None,
        force: true,
        color: ColorChoice::Auto,
    };
    let Some(previous_view) = restty::cache::parsed_view(&snapshot, &active_config) else {
        return Ok(ExitCode::SUCCESS);
    };
    let Some(current_view) = restty::parse_bytes(&current, &options, &active_config) else {
        io::stdout()
            .write_all(&current)
            .map_err(|error| error.to_string())?;
        return Ok(status.map(exit_code).unwrap_or(ExitCode::SUCCESS));
    };
    let diff = restty::cache::diff_views(&previous_view, &current_view, &source);
    let output = restty::render_view(
        &diff,
        &options,
        &active_config,
        restty::terminal::stdout_is_terminal(),
    );
    io::stdout()
        .write_all(&output)
        .map_err(|error| error.to_string())?;
    Ok(status.map(exit_code).unwrap_or(ExitCode::SUCCESS))
}

fn capture_diff_command(
    command: &str,
    command_args: &[String],
    config: &Config,
) -> Result<(Option<Vec<u8>>, String, ExitStatus), String> {
    let source = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("generic")
        .to_owned();
    if should_passthrough_command(&source, command_args) {
        return Err(format!(
            "`{command}` is interactive or unbounded and cannot be diffed"
        ));
    }
    let mut child = Command::new(command)
        .args(command_args)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run `{command}`: {error}"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or("could not capture command output")?;
    let captured = capture_for_live(child_stdout, io::stdout().lock(), config)
        .map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    Ok((captured, source, status))
}

fn render(args: Vec<String>) -> Result<ExitCode, String> {
    let mut source = None;
    let mut width = None;
    let mut force = false;
    let mut color = ColorChoice::Auto;
    let mut border = None;
    let mut format = None;
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
        } else if let Some(value) = argument.strip_prefix("--format=") {
            format = Some(parse_format(value)?);
        } else if argument == "--format" {
            index += 1;
            format = Some(parse_format(
                args.get(index).ok_or("missing value after `--format`")?,
            )?);
        } else if let Some(value) = argument.strip_prefix("--border=") {
            border = Some(parse_border(value)?);
        } else if argument == "--border" {
            index += 1;
            border = Some(parse_border(
                args.get(index).ok_or("missing value after `--border`")?,
            )?);
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
    let mut config = Config::load();
    if let Some(border) = border {
        config.border_style = border;
    }
    let is_tty = restty::terminal::stdout_is_terminal();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let result = if let Some(format) = format {
        restty::run_structured_stream(stdin.lock(), stdout.lock(), &options, &config, format)
    } else {
        restty::run_stream(stdin.lock(), stdout.lock(), &options, &config, is_tty)
    };
    match result {
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
    let mut border = None;
    let mut format = None;
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
        } else if let Some(value) = argument.strip_prefix("--format=") {
            format = Some(parse_format(value)?);
        } else if argument == "--format" {
            index += 1;
            format = Some(parse_format(
                args.get(index).ok_or("missing value after `--format`")?,
            )?);
        } else if let Some(value) = argument.strip_prefix("--border=") {
            border = Some(parse_border(value)?);
        } else if argument == "--border" {
            index += 1;
            border = Some(parse_border(
                args.get(index).ok_or("missing value after `--border`")?,
            )?);
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
        || (format.is_none() && !force && !stdout_is_tty)
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
    if let Some(border) = border {
        config.border_style = border;
    }
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
    let command_identity = args[command_start..].to_vec();
    let _ = restty::cache::store_if_parsed(&captured, &options, &config, &command_identity);
    let output = if let Some(format) = format {
        restty::structured_bytes_with_policy(&captured, &options, &config, format, false)
    } else {
        restty::render_bytes(&captured, &options, &config, stdout_is_tty)
    };
    match io::stdout().write_all(&output) {
        Ok(()) => Ok(exit_code(status)),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(exit_code(status)),
        Err(error) => Err(error.to_string()),
    }
}

fn live_external(args: Vec<String>, explore: bool) -> Result<ExitCode, String> {
    let mut source = None;
    let mut color = ColorChoice::Auto;
    let mut border = None;
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
        } else if let Some(value) = argument.strip_prefix("--border=") {
            border = Some(parse_border(value)?);
        } else if argument == "--border" {
            index += 1;
            border = Some(parse_border(
                args.get(index).ok_or("missing value after `--border`")?,
            )?);
        } else if argument == "--help" || argument == "-h" {
            if explore {
                print_explore_help();
            } else {
                print_live_help();
            }
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
    if let Some(border) = border {
        config.border_style = border;
    }
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
    if restty::parse_bytes(&captured, &options, &config).is_none() {
        io::stdout()
            .write_all(&captured)
            .map_err(|error| error.to_string())?;
        return Ok(exit_code(status));
    }
    let command_identity = args[command_start..].to_vec();
    let _ = restty::cache::store_if_parsed(&captured, &options, &config, &command_identity);
    let viewer = if explore {
        restty::live::explore
    } else {
        restty::live::view
    };
    viewer(&captured, &options, &config).map_err(|error| format!("live viewer failed: {error}"))?;
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

fn parse_format(value: &str) -> Result<StructuredFormat, String> {
    StructuredFormat::from_name(value)
        .ok_or_else(|| format!("invalid output format `{value}` (expected json, csv, or yaml)"))
}

fn parse_border(value: &str) -> Result<String, String> {
    match value {
        "heavy" | "light" | "rounded" | "double" | "ascii" | "none" | "sharp" => {
            Ok(value.to_owned())
        }
        _ => Err(format!(
            "invalid border style `{value}` (expected heavy, light, rounded, double, ascii, or none)"
        )),
    }
}

fn parse_interval(value: &str) -> Result<Duration, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.1 && *seconds <= 86_400.0)
        .map(Duration::from_secs_f64)
        .ok_or_else(|| format!("invalid interval `{value}` (expected 0.1–86400 seconds)"))
}

fn print_help() {
    println!(
        "restty — responsive terminal output\n\nUSAGE:\n  restty live [OPTIONS] -- <command> [args...]\n  restty explore [OPTIONS] -- <command> [args...]\n  restty watch [OPTIONS] -- <command> [args...]\n  restty run [OPTIONS] -- <command> [args...]\n  restty render [OPTIONS] < input\n  restty last\n  restty snapshot <name>\n  restty diff <name> [-- <command> [args...]]\n  restty init <bash|zsh>\n\nTry `restty live -- ps aux` or run a subcommand with --help."
    );
}

fn print_run_help() {
    println!(
        "USAGE:\n  restty run [--source NAME] [--width COLS] [--border STYLE] [--color auto|always|never] [--format json|csv|yaml] [--force] -- <command> [args...]\n\nRuns any non-interactive command through the built-in or generic formatter. Structured formats are emitted on TTY and redirected stdout. Interactive and follow-mode commands are passed through untouched."
    );
}

fn print_live_help() {
    println!(
        "USAGE:\n  restty live [--source NAME] [--border STYLE] [--color auto|always|never] -- <command> [args...]\n\nKeeps completed output in an alternate-screen viewer and redraws it whenever the terminal is resized. Use arrows or j/k to scroll, Page Up/Down for pages, Home/End to jump, and q, Esc, or Ctrl-C to quit."
    );
}

fn print_explore_help() {
    println!(
        "USAGE:\n  restty explore [--source NAME] [--border STYLE] [--color auto|always|never] -- <command> [args...]\n\nOpens retained parsed output in an interactive viewer. Press / to filter, s or Tab to select the next sort column, r to reverse sorting, j/k to scroll, a to invoke a parser-declared row action, and q to quit."
    );
}

fn print_watch_help() {
    println!(
        "USAGE:\n  restty watch [--interval SECONDS] [--alert PATTERN] [--source NAME] [--border STYLE] [--color auto|always|never] -- <command> [args...]\n\nRepeatedly runs a bounded non-interactive command and highlights row changes. Space refreshes immediately; q quits. Alerts ring once per session when a new or changed row matches."
    );
}

fn print_render_help() {
    println!(
        "USAGE:\n  restty render [--source NAME] [--width COLS] [--border STYLE] [--color auto|always|never] [--format json|csv|yaml] [--force]\n\nStructured formats are emitted on TTY and redirected stdout. Without --force or --format, redirected stdout and disable environment variables are exact passthroughs."
    );
}
