use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style as TuiStyle};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell as TuiCell, Gauge, Paragraph, Row as TuiRow, Sparkline,
    Table as TuiTable, TableState,
};
use ratatui::Terminal;

use crate::config::Config;
use crate::model::{ValueType, View};
use crate::render::{BorderStyle, RenderOptions};
use crate::{prepare_output, terminal, PreparedOutput, RunOptions};

pub fn view(input: &[u8], options: &RunOptions, config: &Config) -> io::Result<()> {
    let mut responsive = options.clone();
    responsive.width = None;
    responsive.force = true;
    let prepared = prepare_output(input, &responsive, config, true, false);
    let mut document = LiveDocument::new(prepared);
    let mut session = LiveSession::enter()?;
    let mut scroll = 0_usize;
    let mut size = terminal::terminal_size().unwrap_or((80, 24));
    let frame_interval = Duration::from_millis(16);
    let settle_delay = Duration::from_millis(6);
    let mut last_draw = Instant::now() - frame_interval;
    let mut last_resize = Instant::now() - settle_delay;
    let mut pending_redraw = true;
    let mut max_scroll = 0_usize;

    loop {
        let observed_size = terminal::terminal_size().unwrap_or(size);
        if observed_size != size {
            size = observed_size;
            last_resize = Instant::now();
            pending_redraw = true;
        }

        let now = Instant::now();
        if pending_redraw
            && (now.duration_since(last_resize) >= settle_delay
                || now.duration_since(last_draw) >= frame_interval)
        {
            let frame = document.build_frame(size.0, size.1, scroll);
            scroll = scroll.min(frame.max_scroll);
            max_scroll = frame.max_scroll;
            session.draw(&frame, size.0, size.1, "live", "j/k scroll  q quit")?;
            last_draw = now;
            pending_redraw = false;
        }

        if let Some(key) = read_key(Duration::from_millis(8))? {
            let page = size.1.saturating_sub(2).max(1);
            match key.as_slice() {
                b"q" | b"Q" | b"\x03" | b"\x1a" | b"\x1b" => break,
                b"k" | b"\x1b[A" => scroll = scroll.saturating_sub(1),
                b"j" | b"\x1b[B" => scroll = (scroll + 1).min(max_scroll),
                b"\x1b[5~" => scroll = scroll.saturating_sub(page),
                b"\x1b[6~" => scroll = (scroll + page).min(max_scroll),
                b"g" | b"\x1b[H" => scroll = 0,
                b"G" | b"\x1b[F" => scroll = max_scroll,
                _ => continue,
            }
            pending_redraw = true;
        }
    }
    Ok(())
}

pub fn explore(input: &[u8], options: &RunOptions, config: &Config) -> io::Result<()> {
    let mut responsive = options.clone();
    responsive.width = None;
    responsive.force = true;
    let prepared = prepare_output(input, &responsive, config, true, false);
    let Some(view) = prepared.view().cloned() else {
        return view(input, options, config);
    };
    let mut explorer = Explorer::new(view);
    let mut document = LiveDocument::new(prepared.with_view(explorer.current_view()));
    let mut session = LiveSession::enter()?;
    let mut scroll = 0_usize;
    let mut size = terminal::terminal_size().unwrap_or((80, 24));
    let mut last_draw = Instant::now() - Duration::from_millis(16);
    let mut last_resize = Instant::now() - Duration::from_millis(6);
    let mut pending_redraw = true;
    let mut filtering = false;

    loop {
        let observed_size = terminal::terminal_size().unwrap_or(size);
        if observed_size != size {
            size = observed_size;
            last_resize = Instant::now();
            pending_redraw = true;
        }
        let now = Instant::now();
        if pending_redraw
            && (now.duration_since(last_resize) >= Duration::from_millis(6)
                || now.duration_since(last_draw) >= Duration::from_millis(16))
        {
            let frame = document.build_frame(size.0, size.1, scroll);
            scroll = scroll.min(frame.max_scroll);
            let status = explorer.status(filtering);
            session.draw(&frame, size.0, size.1, "explore", &status)?;
            last_draw = now;
            pending_redraw = false;
        }

        let Some(key) = read_key(Duration::from_millis(8))? else {
            continue;
        };
        let mut changed = false;
        if filtering {
            match key.as_slice() {
                b"\x1b" | b"\r" | b"\n" => filtering = false,
                b"\x7f" | b"\x08" => {
                    explorer.filter.pop();
                    changed = true;
                }
                bytes => {
                    if let Ok(text) = std::str::from_utf8(bytes) {
                        for ch in text.chars().filter(|ch| !ch.is_control()) {
                            explorer.filter.push(ch);
                            changed = true;
                        }
                    }
                }
            }
        } else {
            match key.as_slice() {
                b"q" | b"Q" | b"\x03" | b"\x1a" | b"\x1b" => break,
                b"/" => filtering = true,
                b"s" | b"\t" => {
                    explorer.next_sort();
                    changed = true;
                }
                b"r" => {
                    explorer.descending = !explorer.descending;
                    changed = true;
                }
                b"a" => {
                    if let Some(action) = crate::parser::actions_for(&options.source).first() {
                        if let Some(value) = explorer.selected_value(action.value_key) {
                            session.suspend()?;
                            let action_result = run_action(*action, &value);
                            let resume_result = session.resume();
                            action_result?;
                            resume_result?;
                        }
                    }
                }
                b"k" | b"\x1b[A" => {
                    explorer.move_selection(-1);
                    changed = true;
                }
                b"j" | b"\x1b[B" => {
                    explorer.move_selection(1);
                    changed = true;
                }
                b"\x1b[5~" => {
                    explorer.move_selection(-(size.1.saturating_sub(2).max(1) as isize));
                    changed = true;
                }
                b"\x1b[6~" => {
                    explorer.move_selection(size.1.saturating_sub(2).max(1) as isize);
                    changed = true;
                }
                b"g" | b"\x1b[H" => {
                    explorer.selected = 0;
                    changed = true;
                }
                b"G" | b"\x1b[F" => {
                    explorer.selected = explorer.row_count().saturating_sub(1);
                    changed = true;
                }
                _ => continue,
            }
        }
        if changed {
            document = LiveDocument::new(prepared.with_view(explorer.current_view()));
            let viewport = size.1.saturating_sub(4).max(1);
            scroll = explorer.selected.saturating_sub(viewport / 2);
        }
        pending_redraw = true;
    }
    Ok(())
}

pub fn watch_command(
    command: &str,
    args: &[String],
    interval: Duration,
    alert: Option<&str>,
    options: &RunOptions,
    config: &Config,
) -> io::Result<u8> {
    let (initial, mut last_code) = capture_command(command, args, config)?;
    let mut trends = HashMap::new();
    let mut previous_view = crate::parse_bytes(&initial, options, config);
    if previous_view.is_none() {
        io::stdout().write_all(&initial)?;
        return Ok(last_code);
    }
    if let Some(view) = &mut previous_view {
        update_trends(view, &mut trends, &options.source, config.watch_samples);
    }
    let prepared = prepare_output(&initial, options, config, true, false);
    let mut document = LiveDocument::new(prepared);
    let command_identity = std::iter::once(command.to_owned())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
    let _ = crate::cache::store_if_parsed(&initial, options, config, &command_identity);
    let mut session = LiveSession::enter()?;
    let mut size = terminal::terminal_size().unwrap_or((80, 24));
    let mut scroll = 0_usize;
    let mut max_scroll = 0_usize;
    let mut pending_redraw = true;
    let mut last_draw = Instant::now() - Duration::from_millis(16);
    let mut last_resize = Instant::now() - Duration::from_millis(6);
    let mut next_refresh = Instant::now() + interval;
    let mut alerted = false;

    loop {
        let observed_size = terminal::terminal_size().unwrap_or(size);
        if observed_size != size {
            size = observed_size;
            last_resize = Instant::now();
            pending_redraw = true;
        }
        let now = Instant::now();
        if now >= next_refresh {
            let (current, code) = capture_command(command, args, config)?;
            last_code = code;
            let current_prepared = prepare_output(&current, options, config, true, false);
            let mut current_view = current_prepared.view().cloned();
            if let Some(view) = &mut current_view {
                update_trends(view, &mut trends, &options.source, config.watch_samples);
            }
            let display = match (&previous_view, &current_view) {
                (Some(previous), Some(current)) => {
                    crate::cache::diff_views(previous, current, &options.source)
                }
                _ => current_view
                    .clone()
                    .unwrap_or_else(|| View::Logs(Vec::new())),
            };
            if !alerted && alert.is_some_and(|pattern| changed_rows_match(&display, pattern)) {
                session.bell()?;
                alerted = true;
            }
            document = if current_view.is_some() {
                LiveDocument::new(current_prepared.with_view(display))
            } else {
                LiveDocument::new(current_prepared)
            };
            previous_view = current_view;
            let _ = crate::cache::store_if_parsed(&current, options, config, &command_identity);
            next_refresh = Instant::now() + interval;
            scroll = 0;
            pending_redraw = true;
        }
        if pending_redraw
            && (now.duration_since(last_resize) >= Duration::from_millis(6)
                || now.duration_since(last_draw) >= Duration::from_millis(16))
        {
            let frame = document.build_frame(size.0, size.1, scroll);
            scroll = scroll.min(frame.max_scroll);
            max_scroll = frame.max_scroll;
            let remaining = next_refresh
                .saturating_duration_since(Instant::now())
                .as_secs_f32();
            let help = format!("refresh {remaining:.1}s  j/k scroll  q quit");
            session.draw(&frame, size.0, size.1, "watch", &help)?;
            last_draw = now;
            pending_redraw = false;
        }
        if let Some(key) = read_key(Duration::from_millis(8))? {
            match key.as_slice() {
                b"q" | b"Q" | b"\x03" | b"\x1a" | b"\x1b" => break,
                b"k" | b"\x1b[A" => scroll = scroll.saturating_sub(1),
                b"j" | b"\x1b[B" => scroll = (scroll + 1).min(max_scroll),
                b"\x1b[5~" => scroll = scroll.saturating_sub(size.1.saturating_sub(2).max(1)),
                b"\x1b[6~" => scroll = (scroll + size.1.saturating_sub(2).max(1)).min(max_scroll),
                b"g" | b"\x1b[H" => scroll = 0,
                b"G" | b"\x1b[F" => scroll = max_scroll,
                b" " => next_refresh = Instant::now(),
                _ => continue,
            }
            pending_redraw = true;
        }
    }
    Ok(last_code)
}

fn capture_command(command: &str, args: &[String], config: &Config) -> io::Result<(Vec<u8>, u8)> {
    let mut child = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("could not capture command output"))?;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut lines = 0_usize;
    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        lines += buffer[..read].iter().filter(|byte| **byte == b'\n').count();
        output.extend_from_slice(&buffer[..read]);
        if lines > config.max_lines || output.len() > config.max_bytes {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "watch output exceeded configured safety limits",
            ));
        }
    }
    let status = child.wait()?;
    let code = status.code().unwrap_or(1).clamp(0, 255) as u8;
    Ok((output, code))
}

fn changed_rows_match(view: &View, pattern: &str) -> bool {
    let needle = pattern.to_lowercase();
    match view {
        View::Table(table) => table.rows.iter().any(|row| {
            row.iter().any(|cell| cell.change.is_some())
                && row
                    .iter()
                    .any(|cell| cell.text.to_lowercase().contains(&needle))
        }),
        View::Logs(_) => false,
    }
}

fn update_trends(
    view: &mut View,
    trends: &mut HashMap<(String, String), VecDeque<u64>>,
    source: &str,
    limit: usize,
) {
    let View::Table(table) = view else {
        return;
    };
    if table.columns.is_empty() {
        return;
    }
    let key_index = crate::cache::key_index(source, table);
    for row in &mut table.rows {
        let row_key = row
            .get(key_index)
            .map(|cell| cell.text.clone())
            .unwrap_or_default();
        for (column_index, column) in table.columns.iter().enumerate() {
            if column.value_type != ValueType::Percentage {
                continue;
            }
            let Some(cell) = row.get_mut(column_index) else {
                continue;
            };
            let Ok(value) = cell.text.trim().trim_end_matches('%').parse::<f64>() else {
                continue;
            };
            let history = trends
                .entry((row_key.clone(), column.key.clone()))
                .or_default();
            history.push_back(value.clamp(0.0, 100.0).round() as u64);
            while history.len() > limit {
                history.pop_front();
            }
            cell.trend = history.iter().copied().collect();
        }
    }
}

struct Explorer {
    original: View,
    filter: String,
    sort_column: usize,
    descending: bool,
    selected: usize,
}

impl Explorer {
    fn new(original: View) -> Self {
        Self {
            original,
            filter: String::new(),
            sort_column: 0,
            descending: false,
            selected: 0,
        }
    }

    fn next_sort(&mut self) {
        if let View::Table(table) = &self.original {
            self.sort_column = (self.sort_column + 1) % table.columns.len().max(1);
        }
    }

    fn move_selection(&mut self, amount: isize) {
        let last = self.row_count().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(amount).min(last);
    }

    fn row_count(&self) -> usize {
        match self.current_view_unselected() {
            View::Table(table) => table.rows.len(),
            View::Logs(records) => records.len(),
        }
    }

    fn current_view(&self) -> View {
        let mut view = self.current_view_unselected();
        if let View::Table(table) = &mut view {
            let selected = self.selected.min(table.rows.len().saturating_sub(1));
            if let Some(row) = table.rows.get_mut(selected) {
                for cell in row {
                    cell.style = crate::model::Style::Accent;
                }
            }
        }
        view
    }

    fn current_view_unselected(&self) -> View {
        match &self.original {
            View::Table(table) => {
                let mut table = table.clone();
                if !self.filter.is_empty() {
                    let needle = self.filter.to_lowercase();
                    table.rows.retain(|row| {
                        row.iter()
                            .any(|cell| cell.text.to_lowercase().contains(&needle))
                    });
                }
                table.rows.sort_by(|left, right| {
                    let left = left
                        .get(self.sort_column)
                        .map(|cell| cell.text.as_str())
                        .unwrap_or("");
                    let right = right
                        .get(self.sort_column)
                        .map(|cell| cell.text.as_str())
                        .unwrap_or("");
                    natural_compare(left, right)
                });
                if self.descending {
                    table.rows.reverse();
                }
                View::Table(table)
            }
            View::Logs(records) => {
                let needle = self.filter.to_lowercase();
                View::Logs(
                    records
                        .iter()
                        .filter(|record| {
                            needle.is_empty()
                                || record.message.to_lowercase().contains(&needle)
                                || record
                                    .level
                                    .as_deref()
                                    .is_some_and(|level| level.to_lowercase().contains(&needle))
                        })
                        .cloned()
                        .collect(),
                )
            }
        }
    }

    fn status(&self, filtering: bool) -> String {
        let sort = match &self.original {
            View::Table(table) => table
                .columns
                .get(self.sort_column)
                .map(|column| column.key.as_str())
                .unwrap_or("none"),
            View::Logs(_) => "none",
        };
        if filtering {
            format!("filter: {}_  Enter accept  Esc close", self.filter)
        } else {
            format!(
                "row:{}/{}  sort:{sort} {}  filter:{}  / filter  s next  r reverse  a action  q quit",
                self.selected.saturating_add(1).min(self.row_count()),
                self.row_count(),
                if self.descending { "desc" } else { "asc" },
                if self.filter.is_empty() {
                    "—"
                } else {
                    &self.filter
                }
            )
        }
    }

    fn selected_value(&self, key: &str) -> Option<String> {
        let View::Table(table) = self.current_view_unselected() else {
            return None;
        };
        let column = table.columns.iter().position(|column| column.key == key)?;
        table
            .rows
            .get(self.selected.min(table.rows.len().saturating_sub(1)))?
            .get(column)
            .map(|cell| cell.text.clone())
    }
}

fn run_action(action: crate::parser::ActionSpec, value: &str) -> io::Result<()> {
    let status = match action.kind {
        crate::parser::ActionKind::TerminateProcess => {
            Command::new("kill").args(["-TERM", value]).status()?
        }
        crate::parser::ActionKind::OpenPath => {
            #[cfg(target_os = "macos")]
            let opener = "open";
            #[cfg(not(target_os = "macos"))]
            let opener = "xdg-open";
            Command::new(opener).arg(value).status()?
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} action failed with {status}",
            action.label
        )))
    }
}

fn natural_compare(left: &str, right: &str) -> std::cmp::Ordering {
    match (numeric_value(left), numeric_value(right)) {
        (Some(left), Some(right)) => left
            .partial_cmp(&right)
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => left.to_lowercase().cmp(&right.to_lowercase()),
    }
}

fn numeric_value(value: &str) -> Option<f64> {
    value
        .trim()
        .trim_end_matches('%')
        .replace(',', "")
        .parse()
        .ok()
}

struct Frame {
    lines: Vec<String>,
    first_line: usize,
    max_scroll: usize,
    total_lines: usize,
    view: Option<View>,
    render_options: RenderOptions,
    table_layout: Option<crate::render::TableLayout>,
}

struct LiveDocument {
    prepared: PreparedOutput,
    cached_width: Option<usize>,
    cached_lines: Vec<String>,
    cached_table_layout: Option<crate::render::TableLayout>,
}

impl LiveDocument {
    fn new(prepared: PreparedOutput) -> Self {
        Self {
            prepared,
            cached_width: None,
            cached_lines: Vec::new(),
            cached_table_layout: None,
        }
    }

    fn build_frame(&mut self, columns: usize, rows: usize, scroll: usize) -> Frame {
        let content_width = columns.saturating_sub(1).max(1);
        if self.cached_width != Some(content_width) {
            let render_options = self.prepared.render_options_at(content_width);
            let rendered = self.prepared.render_at(content_width);
            let rendered = String::from_utf8_lossy(&rendered);
            self.cached_lines = rendered
                .lines()
                .map(|line| truncate_ansi_aware(line, content_width))
                .collect();
            self.cached_width = Some(content_width);
            self.cached_table_layout = match self.prepared.view() {
                Some(View::Table(table)) => {
                    Some(crate::render::table_layout(table, &render_options))
                }
                _ => None,
            };
        }

        let viewport_height = rows.saturating_sub(1);
        let max_scroll = self.cached_lines.len().saturating_sub(viewport_height);
        let first_line = scroll.min(max_scroll);
        let lines = self
            .cached_lines
            .iter()
            .skip(first_line)
            .take(viewport_height)
            .cloned()
            .collect();
        Frame {
            lines,
            first_line,
            max_scroll,
            total_lines: self.cached_lines.len(),
            view: self.prepared.view().cloned(),
            render_options: self.prepared.render_options_at(content_width),
            table_layout: self.cached_table_layout.clone(),
        }
    }
}

struct LiveSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    active: bool,
}

impl LiveSession {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, DisableLineWrap, Hide) {
            let _ = execute!(stdout, Show, EnableLineWrap, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self {
                terminal,
                active: true,
            }),
            Err(error) => {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, Show, EnableLineWrap, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                Err(error)
            }
        }
    }

    fn draw(
        &mut self,
        frame: &Frame,
        columns: usize,
        rows: usize,
        mode: &str,
        help: &str,
    ) -> io::Result<()> {
        let screen = screen_lines(frame, columns, rows, mode, help);
        let content = screen
            .iter()
            .take(screen.len().saturating_sub(1))
            .map(|line| ansi_line(line))
            .collect::<Vec<_>>();
        let status = screen.last().cloned().unwrap_or_default();
        self.terminal.draw(|terminal_frame| {
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(terminal_frame.size());
            if let Some(View::Table(table)) = &frame.view {
                render_tui_table(terminal_frame, areas[0], table, frame);
            } else {
                terminal_frame.render_widget(Paragraph::new(content), areas[0]);
            }
            terminal_frame.render_widget(Paragraph::new(ansi_line(&status)), areas[1]);
        })?;
        Ok(())
    }

    fn bell(&mut self) -> io::Result<()> {
        self.terminal.backend_mut().write_all(b"\x07")?;
        self.terminal.backend_mut().flush()
    }

    fn suspend(&mut self) -> io::Result<()> {
        if self.active {
            disable_raw_mode()?;
            execute!(
                self.terminal.backend_mut(),
                Show,
                EnableLineWrap,
                LeaveAlternateScreen
            )?;
            self.active = false;
        }
        Ok(())
    }

    fn resume(&mut self) -> io::Result<()> {
        if !self.active {
            enable_raw_mode()?;
            if let Err(error) = execute!(
                self.terminal.backend_mut(),
                EnterAlternateScreen,
                DisableLineWrap,
                Hide
            ) {
                let _ = execute!(
                    self.terminal.backend_mut(),
                    Show,
                    EnableLineWrap,
                    LeaveAlternateScreen
                );
                let _ = disable_raw_mode();
                return Err(error);
            }
            self.active = true;
            self.terminal.clear()?;
        }
        Ok(())
    }
}

fn render_tui_table(
    frame: &mut ratatui::Frame<'_>,
    mut area: ratatui::layout::Rect,
    table: &crate::model::Table,
    live_frame: &Frame,
) {
    if !table.prelude.is_empty() && area.height > 1 {
        let prelude_height = table.prelude.len().min(usize::from(area.height - 1)) as u16;
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(prelude_height), Constraint::Min(1)])
            .split(area);
        let prelude = table
            .prelude
            .iter()
            .map(|line| {
                Line::styled(
                    line.clone(),
                    tui_style(crate::model::Style::Muted, None, &live_frame.render_options),
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(prelude), areas[0]);
        area = areas[1];
    }
    let mut options = live_frame.render_options;
    options.width = usize::from(area.width.saturating_sub(1)).max(1);
    let layout = live_frame
        .table_layout
        .clone()
        .unwrap_or_else(|| crate::render::table_layout(table, &options));
    if layout.selected.is_empty() {
        return;
    }
    let header = TuiRow::new(layout.selected.iter().map(|index| {
        TuiCell::from(table.columns[*index].label.clone()).style(tui_style(
            crate::model::Style::Accent,
            None,
            &options,
        ))
    }));
    let rows = table.rows.iter().map(|row| {
        TuiRow::new(layout.selected.iter().map(|index| {
            let cell = row
                .get(*index)
                .cloned()
                .unwrap_or_else(|| crate::model::Cell::plain(""));
            TuiCell::from(crate::render::format_cell_value(
                &table.columns[*index],
                &cell,
                &options,
            ))
            .style(tui_style(cell.style, cell.change, &options))
        }))
    });
    let widths = layout
        .widths
        .iter()
        .map(|width| Constraint::Length((*width).min(u16::MAX as usize) as u16))
        .collect::<Vec<_>>();
    let block = table_block(options.border_style);
    let inner = block.inner(area);
    let widget = TuiTable::new(rows, widths)
        .header(header)
        .column_spacing(layout.padding.min(u16::MAX as usize) as u16)
        .block(block);
    let mut state = TableState::default();
    let offset = live_frame
        .first_line
        .min(table.rows.len().saturating_sub(1));
    *state.offset_mut() = offset;
    frame.render_stateful_widget(widget, area, &mut state);
    render_tui_gauges(frame, inner, table, &layout, &options, offset);
}

fn render_tui_gauges(
    frame: &mut ratatui::Frame<'_>,
    inner: ratatui::layout::Rect,
    table: &crate::model::Table,
    layout: &crate::render::TableLayout,
    options: &RenderOptions,
    offset: usize,
) {
    if options.width < options.breakpoints.core || inner.height < 2 {
        return;
    }
    let visible_rows = usize::from(inner.height.saturating_sub(1));
    for (visible_index, row) in table
        .rows
        .iter()
        .skip(offset)
        .take(visible_rows)
        .enumerate()
    {
        let mut x = inner.x;
        for (position, column_index) in layout.selected.iter().enumerate() {
            let width = layout.widths[position].min(u16::MAX as usize) as u16;
            let column = &table.columns[*column_index];
            let Some(cell) = row.get(*column_index) else {
                x = x
                    .saturating_add(width)
                    .saturating_add(layout.padding as u16);
                continue;
            };
            if column.value_type == ValueType::Percentage && width > 0 {
                let value = cell.text.trim().trim_end_matches('%').parse::<f64>().ok();
                if let Some(value) = value {
                    let cell_area = ratatui::layout::Rect::new(
                        x,
                        inner.y.saturating_add(1 + visible_index as u16),
                        width.min(inner.right().saturating_sub(x)),
                        1,
                    );
                    if cell_area.width > 0 {
                        let areas = if options.width >= options.breakpoints.wide
                            && cell.trend.len() > 1
                            && cell_area.width >= 8
                        {
                            Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([
                                    Constraint::Percentage(60),
                                    Constraint::Percentage(40),
                                ])
                                .split(cell_area)
                        } else {
                            Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([Constraint::Percentage(100), Constraint::Length(0)])
                                .split(cell_area)
                        };
                        frame.render_widget(
                            Gauge::default()
                                .ratio((value / 100.0).clamp(0.0, 1.0))
                                .label(cell.text.clone())
                                .use_unicode(options.border_style != BorderStyle::Ascii)
                                .gauge_style(tui_style(cell.style, cell.change, options)),
                            areas[0],
                        );
                        if areas[1].width > 0 {
                            frame.render_widget(
                                Sparkline::default()
                                    .data(&cell.trend)
                                    .max(100)
                                    .style(tui_style(cell.style, cell.change, options)),
                                areas[1],
                            );
                        }
                    }
                }
            }
            x = x
                .saturating_add(width)
                .saturating_add(layout.padding as u16);
        }
    }
}

fn table_block(style: BorderStyle) -> Block<'static> {
    let block = Block::default();
    match style {
        BorderStyle::None => block,
        BorderStyle::Rounded => block.borders(Borders::ALL).border_type(BorderType::Rounded),
        BorderStyle::Light => block.borders(Borders::ALL).border_type(BorderType::Plain),
        BorderStyle::Heavy => block.borders(Borders::ALL).border_type(BorderType::Thick),
        BorderStyle::Double => block.borders(Borders::ALL).border_type(BorderType::Double),
        BorderStyle::Ascii => block.borders(Borders::ALL).border_set(border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        }),
    }
}

fn tui_style(
    style: crate::model::Style,
    change: Option<crate::model::ChangeKind>,
    options: &RenderOptions,
) -> TuiStyle {
    if !options.color {
        return TuiStyle::default();
    }
    if let Some(change) = change {
        let background = match (options.palette, options.truecolor, change) {
            (crate::render::ColorPalette::Default, false, crate::model::ChangeKind::Added) => {
                Color::Green
            }
            (crate::render::ColorPalette::Default, false, crate::model::ChangeKind::Removed) => {
                Color::Red
            }
            (_, false, crate::model::ChangeKind::Changed) => Color::Yellow,
            (crate::render::ColorPalette::Colorblind, false, crate::model::ChangeKind::Added) => {
                Color::Blue
            }
            (crate::render::ColorPalette::Colorblind, false, crate::model::ChangeKind::Removed) => {
                Color::Magenta
            }
            (crate::render::ColorPalette::Default, true, crate::model::ChangeKind::Added) => {
                Color::Rgb(34, 197, 94)
            }
            (crate::render::ColorPalette::Default, true, crate::model::ChangeKind::Removed) => {
                Color::Rgb(248, 113, 113)
            }
            (crate::render::ColorPalette::Default, true, crate::model::ChangeKind::Changed) => {
                Color::Rgb(250, 204, 21)
            }
            (crate::render::ColorPalette::Colorblind, true, crate::model::ChangeKind::Added) => {
                Color::Rgb(86, 180, 233)
            }
            (crate::render::ColorPalette::Colorblind, true, crate::model::ChangeKind::Removed) => {
                Color::Rgb(204, 121, 167)
            }
            (crate::render::ColorPalette::Colorblind, true, crate::model::ChangeKind::Changed) => {
                Color::Rgb(240, 228, 66)
            }
        };
        return TuiStyle::default().fg(Color::Black).bg(background);
    }
    let (color, modifier) = if options.truecolor {
        match (options.palette, style) {
            (_, crate::model::Style::Plain) => return TuiStyle::default(),
            (_, crate::model::Style::Muted) => (Color::Rgb(148, 163, 184), Modifier::DIM),
            (_, crate::model::Style::Accent) => (Color::Rgb(34, 211, 238), Modifier::BOLD),
            (crate::render::ColorPalette::Default, crate::model::Style::Good) => {
                (Color::Rgb(34, 197, 94), Modifier::empty())
            }
            (crate::render::ColorPalette::Default, crate::model::Style::Warning) => {
                (Color::Rgb(250, 204, 21), Modifier::empty())
            }
            (crate::render::ColorPalette::Default, crate::model::Style::Error) => {
                (Color::Rgb(248, 113, 113), Modifier::BOLD)
            }
            (crate::render::ColorPalette::Default, crate::model::Style::Info) => {
                (Color::Rgb(96, 165, 250), Modifier::empty())
            }
            (crate::render::ColorPalette::Default, crate::model::Style::Debug) => {
                (Color::Rgb(192, 132, 252), Modifier::DIM)
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Good) => {
                (Color::Rgb(86, 180, 233), Modifier::empty())
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Warning) => {
                (Color::Rgb(240, 228, 66), Modifier::empty())
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Error) => {
                (Color::Rgb(204, 121, 167), Modifier::BOLD)
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Info) => {
                (Color::Rgb(0, 158, 115), Modifier::empty())
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Debug) => {
                (Color::Rgb(213, 94, 0), Modifier::DIM)
            }
        }
    } else {
        match (options.palette, style) {
            (_, crate::model::Style::Plain) => return TuiStyle::default(),
            (_, crate::model::Style::Muted) => (Color::Gray, Modifier::DIM),
            (_, crate::model::Style::Accent) => (Color::Cyan, Modifier::BOLD),
            (crate::render::ColorPalette::Default, crate::model::Style::Good) => {
                (Color::Green, Modifier::empty())
            }
            (_, crate::model::Style::Warning) => (Color::Yellow, Modifier::empty()),
            (crate::render::ColorPalette::Default, crate::model::Style::Error) => {
                (Color::Red, Modifier::BOLD)
            }
            (crate::render::ColorPalette::Default, crate::model::Style::Info) => {
                (Color::Blue, Modifier::empty())
            }
            (crate::render::ColorPalette::Default, crate::model::Style::Debug) => {
                (Color::Magenta, Modifier::DIM)
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Good) => {
                (Color::Blue, Modifier::empty())
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Error) => {
                (Color::Magenta, Modifier::BOLD)
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Info) => {
                (Color::Cyan, Modifier::empty())
            }
            (crate::render::ColorPalette::Colorblind, crate::model::Style::Debug) => {
                (Color::Red, Modifier::DIM)
            }
        }
    };
    TuiStyle::default().fg(color).add_modifier(modifier)
}

fn screen_lines(frame: &Frame, columns: usize, rows: usize, mode: &str, help: &str) -> Vec<String> {
    let mut screen = vec![String::new(); rows];
    for (row, line) in frame.lines.iter().enumerate().take(rows.saturating_sub(1)) {
        screen[row] = line.clone();
    }
    if rows > 0 {
        let last_visible = (frame.first_line + frame.lines.len()).min(frame.total_lines);
        let status = format!(
            " restty {mode}  {}×{}  lines {}–{} of {}  {help} ",
            columns,
            rows,
            if frame.total_lines == 0 {
                0
            } else {
                frame.first_line + 1
            },
            last_visible,
            frame.total_lines
        );
        let status = truncate_plain(&status, columns);
        screen[rows - 1] = format!("\x1b[7m{status}\x1b[0m");
    }
    screen
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            let _ = execute!(
                self.terminal.backend_mut(),
                Show,
                EnableLineWrap,
                LeaveAlternateScreen
            );
        }
        let _ = self.terminal.show_cursor();
    }
}

fn read_key(timeout: Duration) -> io::Result<Option<Vec<u8>>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    let Event::Key(key) = event::read()? else {
        return Ok(None);
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Ok(None);
    }
    let bytes = match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => vec![3],
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => vec![26],
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Esc => vec![27],
        KeyCode::Enter => vec![b'\n'],
        KeyCode::Backspace | KeyCode::Delete => vec![127],
        KeyCode::Tab => vec![b'\t'],
        _ => return Ok(None),
    };
    Ok(Some(bytes))
}

fn ansi_line(value: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut remaining = value;
    let mut style = TuiStyle::default();
    while let Some(start) = remaining.find("\x1b[") {
        if start > 0 {
            spans.push(Span::styled(remaining[..start].to_owned(), style));
        }
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find('m') else {
            spans.push(Span::styled("\x1b[".to_owned(), style));
            break;
        };
        style = sgr_style(&remaining[..end], style);
        remaining = &remaining[end + 1..];
    }
    if !remaining.is_empty() {
        spans.push(Span::styled(remaining.to_owned(), style));
    }
    Line::from(spans)
}

fn sgr_style(sequence: &str, mut style: TuiStyle) -> TuiStyle {
    if sequence.is_empty() || sequence == "0" {
        return TuiStyle::default();
    }
    let codes = sequence
        .split(';')
        .filter_map(|code| code.parse::<u8>().ok())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < codes.len() {
        let code = codes[index];
        if matches!(code, 38 | 48) && codes.get(index + 1) == Some(&2) && index + 4 < codes.len() {
            let color = Color::Rgb(codes[index + 2], codes[index + 3], codes[index + 4]);
            style = if code == 38 {
                style.fg(color)
            } else {
                style.bg(color)
            };
            index += 5;
            continue;
        }
        style = match code {
            1 => style.add_modifier(Modifier::BOLD),
            2 => style.add_modifier(Modifier::DIM),
            7 => style.add_modifier(Modifier::REVERSED),
            30 => style.fg(Color::Black),
            31 => style.fg(Color::Red),
            32 => style.fg(Color::Green),
            33 => style.fg(Color::Yellow),
            34 => style.fg(Color::Blue),
            35 => style.fg(Color::Magenta),
            36 => style.fg(Color::Cyan),
            37 => style.fg(Color::Gray),
            41 => style.bg(Color::Red),
            42 => style.bg(Color::Green),
            43 => style.bg(Color::Yellow),
            _ => style,
        };
        index += 1;
    }
    style
}

fn truncate_plain(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_owned();
    }
    truncate_visible_chars(value, max_width)
}

fn truncate_visible_chars(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let target = max_width.saturating_sub(1);
    let mut result = String::new();
    let mut width = 0;
    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > target {
            break;
        }
        result.push(ch);
        width += char_width;
    }
    result.push('…');
    result
}

fn truncate_ansi_aware(value: &str, max_width: usize) -> String {
    if visible_width(value) <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    let target = max_width.saturating_sub(1);
    let mut result = String::new();
    let mut width = 0;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            result.push(ch);
            for sequence in chars.by_ref() {
                result.push(sequence);
                if sequence.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > target {
            break;
        }
        result.push(ch);
        width += char_width;
    }
    result.push('…');
    result.push_str("\x1b[0m");
    result
}

fn visible_width(value: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    for ch in value.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            width += ch.width().unwrap_or(0);
        }
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColorChoice;

    fn options() -> RunOptions {
        RunOptions {
            source: "generic".into(),
            width: None,
            force: true,
            color: ColorChoice::Never,
        }
    }

    #[test]
    fn frame_reflows_and_clamps_scroll_for_each_size() {
        let input = b"NAME  VALUE\nalpha  a-very-long-value\nbeta  second\ngamma  third\n";
        let config = Config::default();
        let prepared = prepare_output(input, &options(), &config, true, false);
        let mut document = LiveDocument::new(prepared);
        let narrow = document.build_frame(24, 4, usize::MAX);
        let wide = document.build_frame(80, 20, 0);
        assert!(narrow.first_line <= narrow.max_scroll);
        assert!(narrow.lines.iter().all(|line| visible_width(line) < 24));
        assert!(wide.lines.iter().all(|line| visible_width(line) < 80));
        assert_eq!(wide.max_scroll, 0);
    }

    #[test]
    fn ansi_truncation_preserves_width_and_resets_style() {
        let value = "\x1b[31mabcdef\x1b[0m";
        let truncated = truncate_ansi_aware(value, 4);
        assert_eq!(visible_width(&truncated), 4);
        assert!(truncated.ends_with("\x1b[0m"));
    }

    #[test]
    fn ansi_styles_are_converted_to_tui_spans() {
        let line = ansi_line("plain \x1b[1;31merror\x1b[0m done");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[1].content, "error");
        assert_eq!(line.spans[1].style.fg, Some(Color::Red));
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        let rgb = ansi_line("\x1b[38;2;1;2;3mrgb\x1b[0m");
        assert_eq!(rgb.spans[0].style.fg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn explorer_filters_and_naturally_sorts_retained_rows() {
        use crate::model::{Alignment, Cell, Column, Table};
        let view = View::Table(Table {
            columns: vec![Column::new("pid", "PID", 0, Alignment::Right)],
            rows: vec![
                vec![Cell::styled("10", crate::model::Style::Plain)],
                vec![Cell::styled("2", crate::model::Style::Plain)],
                vec![Cell::styled("30", crate::model::Style::Plain)],
            ],
            prelude: Vec::new(),
        });
        let mut explorer = Explorer::new(view);
        explorer.filter = "0".into();
        let View::Table(table) = explorer.current_view() else {
            panic!("expected table");
        };
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0].text, "10");
        explorer.descending = true;
        let View::Table(table) = explorer.current_view() else {
            panic!("expected table");
        };
        assert_eq!(table.rows[0][0].text, "30");
    }
}
