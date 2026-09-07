pub mod cache;
pub mod config;
pub mod live;
pub mod model;
pub mod parser;
pub mod render;
pub mod shell;
pub mod structured;
pub mod terminal;

use std::io::{self, Read, Write};

use config::Config;
use model::View;
use parser::ParserRegistry;
use render::RenderOptions;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub source: String,
    pub width: Option<usize>,
    pub force: bool,
    pub color: ColorChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedOutput {
    content: PreparedContent,
    color: bool,
    breakpoints: config::Breakpoints,
    border_style: render::BorderStyle,
    palette: render::ColorPalette,
    truecolor: bool,
}

#[derive(Debug, Clone)]
enum PreparedContent {
    Parsed { view: View, original: Vec<u8> },
    Original(Vec<u8>),
}

impl PreparedOutput {
    pub(crate) fn view(&self) -> Option<&View> {
        match &self.content {
            PreparedContent::Parsed { view, .. } => Some(view),
            PreparedContent::Original(_) => None,
        }
    }

    pub(crate) fn with_view(&self, view: View) -> Self {
        let original = match &self.content {
            PreparedContent::Parsed { original, .. } | PreparedContent::Original(original) => {
                original.clone()
            }
        };
        Self {
            content: PreparedContent::Parsed { view, original },
            color: self.color,
            breakpoints: self.breakpoints,
            border_style: self.border_style,
            palette: self.palette,
            truecolor: self.truecolor,
        }
    }

    pub(crate) fn render_at(&self, width: usize) -> Vec<u8> {
        let (view, original) = match &self.content {
            PreparedContent::Parsed { view, original } => (view, original),
            PreparedContent::Original(input) => return input.clone(),
        };
        let rendered = render::render(
            view,
            &RenderOptions {
                width: width.max(1),
                color: self.color,
                breakpoints: self.breakpoints,
                border_style: self.border_style,
                palette: self.palette,
                truecolor: self.truecolor,
            },
        );
        if rendered.is_empty() {
            original.clone()
        } else {
            rendered.into_bytes()
        }
    }

    pub(crate) fn render_options_at(&self, width: usize) -> RenderOptions {
        RenderOptions {
            width,
            color: self.color,
            breakpoints: self.breakpoints,
            border_style: self.border_style,
            palette: self.palette,
            truecolor: self.truecolor,
        }
    }
}

pub fn structured_bytes_with_policy(
    input: &[u8],
    options: &RunOptions,
    config: &Config,
    format: structured::StructuredFormat,
    formatting_disabled: bool,
) -> Vec<u8> {
    let mut explicit = options.clone();
    explicit.force = true;
    let prepared = prepare_output(input, &explicit, config, true, formatting_disabled);
    prepared
        .view()
        .map(|view| structured::serialize(view, format).into_bytes())
        .unwrap_or_else(|| input.to_vec())
}

pub fn run_structured_stream<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    options: &RunOptions,
    config: &Config,
    format: structured::StructuredFormat,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut lines = 0_usize;
    loop {
        let read = input.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        lines += chunk[..read].iter().filter(|byte| **byte == b'\n').count();
        bytes.extend_from_slice(&chunk[..read]);
        if lines > config.max_lines || bytes.len() > config.max_bytes {
            output.write_all(&bytes)?;
            io::copy(&mut input, &mut output)?;
            return Ok(());
        }
    }
    output.write_all(&structured_bytes_with_policy(
        &bytes,
        options,
        config,
        format,
        terminal::formatting_disabled(),
    ))
}

pub fn render_bytes(
    input: &[u8],
    options: &RunOptions,
    config: &Config,
    stdout_is_tty: bool,
) -> Vec<u8> {
    render_bytes_with_policy(
        input,
        options,
        config,
        stdout_is_tty,
        terminal::formatting_disabled(),
    )
}

pub fn render_view(
    view: &View,
    options: &RunOptions,
    config: &Config,
    stdout_is_tty: bool,
) -> Vec<u8> {
    let width = options
        .width
        .or_else(|| terminal::terminal_width().map(|width| width.saturating_sub(1)))
        .unwrap_or(79)
        .max(1);
    let color = match options.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => stdout_is_tty,
    } && config.theme != "none";
    render::render(
        view,
        &RenderOptions {
            width,
            color,
            breakpoints: config.breakpoints,
            border_style: resolved_border_style(config),
            palette: render::ColorPalette::from_name(&config.theme),
            truecolor: terminal::supports_truecolor(&config.terminal_profile),
        },
    )
    .into_bytes()
}

pub fn parse_bytes(input: &[u8], options: &RunOptions, config: &Config) -> Option<View> {
    let mut explicit = options.clone();
    explicit.force = true;
    prepare_output(input, &explicit, config, true, false)
        .view()
        .cloned()
}

#[doc(hidden)]
pub fn render_bytes_with_policy(
    input: &[u8],
    options: &RunOptions,
    config: &Config,
    stdout_is_tty: bool,
    formatting_disabled: bool,
) -> Vec<u8> {
    let width = options
        .width
        .or_else(|| terminal::terminal_width().map(|width| width.saturating_sub(1)))
        .unwrap_or(79)
        .max(1);
    prepare_output(input, options, config, stdout_is_tty, formatting_disabled).render_at(width)
}

pub(crate) fn prepare_output(
    input: &[u8],
    options: &RunOptions,
    config: &Config,
    stdout_is_tty: bool,
    formatting_disabled: bool,
) -> PreparedOutput {
    let original = || PreparedOutput {
        content: PreparedContent::Original(input.to_vec()),
        color: false,
        breakpoints: config.breakpoints,
        border_style: resolved_border_style(config),
        palette: render::ColorPalette::from_name(&config.theme),
        truecolor: terminal::supports_truecolor(&config.terminal_profile),
    };
    if formatting_disabled || (!options.force && !stdout_is_tty) {
        return original();
    }
    if !source_enabled(&options.source, config) {
        return original();
    }
    if input.iter().filter(|byte| **byte == b'\n').count() > config.max_lines
        || input.len() > config.max_bytes
    {
        return original();
    }
    let Ok(text) = std::str::from_utf8(input) else {
        return original();
    };
    if text.trim().is_empty() {
        return original();
    }

    let registry = ParserRegistry::new(&config.disabled_parsers);
    let Some(mut parsed) = registry.parse(&options.source, text) else {
        return original();
    };
    config.apply_column_priorities(&options.source, &mut parsed.view);
    config.apply_thresholds(&options.source, &mut parsed.view);
    if parsed.confidence < config.min_confidence {
        return original();
    }
    let color = match options.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => stdout_is_tty,
    } && config.theme != "none";
    PreparedOutput {
        content: PreparedContent::Parsed {
            view: parsed.view,
            original: input.to_vec(),
        },
        color,
        breakpoints: config.breakpoints,
        border_style: resolved_border_style(config),
        palette: render::ColorPalette::from_name(&config.theme),
        truecolor: terminal::supports_truecolor(&config.terminal_profile),
    }
}

pub fn run_stream<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    options: &RunOptions,
    config: &Config,
    stdout_is_tty: bool,
) -> io::Result<()> {
    if terminal::formatting_disabled()
        || (!options.force && !stdout_is_tty)
        || !source_enabled(&options.source, config)
    {
        io::copy(&mut input, &mut output)?;
        return Ok(());
    }

    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut lines = 0_usize;
    loop {
        let read = input.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        lines += chunk[..read].iter().filter(|byte| **byte == b'\n').count();
        bytes.extend_from_slice(&chunk[..read]);
        if lines > config.max_lines || bytes.len() > config.max_bytes {
            output.write_all(&bytes)?;
            io::copy(&mut input, &mut output)?;
            return Ok(());
        }
    }

    output.write_all(&render_bytes_with_policy(
        &bytes,
        options,
        config,
        stdout_is_tty,
        false,
    ))
}

fn source_enabled(source: &str, config: &Config) -> bool {
    source == "generic"
        || config
            .enabled_commands
            .iter()
            .any(|command| command == source)
}

fn resolved_border_style(config: &Config) -> render::BorderStyle {
    let requested = render::BorderStyle::from_name(&config.border_style);
    if requested != render::BorderStyle::None
        && requested != render::BorderStyle::Ascii
        && !terminal::supports_unicode(&config.terminal_profile)
    {
        render::BorderStyle::Ascii
    } else {
        requested
    }
}
