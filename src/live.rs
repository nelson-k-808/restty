use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Config;
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
            session.draw(&frame, size.0, size.1)?;
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

struct Frame {
    lines: Vec<String>,
    first_line: usize,
    max_scroll: usize,
    total_lines: usize,
}

struct LiveDocument {
    prepared: PreparedOutput,
    cached_width: Option<usize>,
    cached_lines: Vec<String>,
}

impl LiveDocument {
    fn new(prepared: PreparedOutput) -> Self {
        Self {
            prepared,
            cached_width: None,
            cached_lines: Vec::new(),
        }
    }

    fn build_frame(&mut self, columns: usize, rows: usize, scroll: usize) -> Frame {
        let content_width = columns.saturating_sub(1).max(1);
        if self.cached_width != Some(content_width) {
            let rendered = self.prepared.render_at(content_width);
            let rendered = String::from_utf8_lossy(&rendered);
            self.cached_lines = rendered
                .lines()
                .map(|line| truncate_ansi_aware(line, content_width))
                .collect();
            self.cached_width = Some(content_width);
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
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct TermiosStorage([u8; 256]);

#[cfg(unix)]
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[cfg(unix)]
unsafe extern "C" {
    fn tcgetattr(fd: i32, termios: *mut TermiosStorage) -> i32;
    fn tcsetattr(fd: i32, action: i32, termios: *const TermiosStorage) -> i32;
    fn cfmakeraw(termios: *mut TermiosStorage);
    fn poll(fds: *mut PollFd, count: usize, timeout_ms: i32) -> i32;
    fn read(fd: i32, buffer: *mut u8, count: usize) -> isize;
}

struct LiveSession {
    stdout: io::Stdout,
    previous_screen: Vec<String>,
    #[cfg(unix)]
    original_termios: TermiosStorage,
}

impl LiveSession {
    #[cfg(unix)]
    fn enter() -> io::Result<Self> {
        let mut original_termios = TermiosStorage([0; 256]);
        // SAFETY: fd 0 is stdin and storage is deliberately larger and more aligned than
        // termios on the supported Unix targets.
        if unsafe { tcgetattr(0, &mut original_termios) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original_termios;
        // SAFETY: `raw` contains a termios value initialized by tcgetattr.
        unsafe { cfmakeraw(&mut raw) };
        // TCSANOW is zero on the supported Unix targets.
        if unsafe { tcsetattr(0, 0, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut stdout = io::stdout();
        if let Err(error) =
            write!(stdout, "\x1b[?1049h\x1b[?7l\x1b[?25l").and_then(|_| stdout.flush())
        {
            // SAFETY: restore the value captured from fd 0 above.
            unsafe { tcsetattr(0, 0, &original_termios) };
            return Err(error);
        }
        Ok(Self {
            stdout,
            previous_screen: Vec::new(),
            original_termios,
        })
    }

    #[cfg(not(unix))]
    fn enter() -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "live mode currently requires a Unix terminal",
        ))
    }

    fn draw(&mut self, frame: &Frame, columns: usize, rows: usize) -> io::Result<()> {
        let next_screen = screen_lines(frame, columns, rows);
        let update = compose_update(&self.previous_screen, &next_screen);
        if !update.is_empty() {
            self.stdout.write_all(update.as_bytes())?;
            self.stdout.flush()?;
        }
        self.previous_screen = next_screen;
        Ok(())
    }
}

fn screen_lines(frame: &Frame, columns: usize, rows: usize) -> Vec<String> {
    let mut screen = vec![String::new(); rows];
    for (row, line) in frame.lines.iter().enumerate().take(rows.saturating_sub(1)) {
        screen[row] = line.clone();
    }
    if rows > 0 {
        let last_visible = (frame.first_line + frame.lines.len()).min(frame.total_lines);
        let status = format!(
            " restty live  {}×{}  lines {}–{} of {}  j/k scroll  q quit ",
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

fn compose_update(previous: &[String], next: &[String]) -> String {
    let mut rows = String::new();
    for (row, line) in next.iter().enumerate() {
        if previous.get(row) != Some(line) {
            let _ = write!(rows, "\x1b[{};1H\x1b[2K{}", row + 1, line);
        }
    }
    if rows.is_empty() {
        String::new()
    } else {
        format!("\x1b[?2026h{rows}\x1b[?2026l")
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = write!(self.stdout, "\x1b[0m\x1b[?25h\x1b[?7h\x1b[?1049l")
            .and_then(|_| self.stdout.flush());
        #[cfg(unix)]
        {
            // SAFETY: restore the termios value captured from stdin at session entry.
            unsafe { tcsetattr(0, 0, &self.original_termios) };
        }
    }
}

#[cfg(unix)]
fn read_key(timeout: Duration) -> io::Result<Option<Vec<u8>>> {
    let mut descriptor = PollFd {
        fd: 0,
        events: 1,
        revents: 0,
    };
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: descriptor points to one initialized pollfd value.
    let result = unsafe { poll(&mut descriptor, 1, timeout_ms) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if result == 0 || descriptor.revents & 1 == 0 {
        return Ok(None);
    }
    let mut buffer = [0_u8; 32];
    // SAFETY: buffer is writable for its declared length.
    let count = unsafe { read(0, buffer.as_mut_ptr(), buffer.len()) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((count > 0).then(|| buffer[..count as usize].to_vec()))
}

#[cfg(not(unix))]
fn read_key(_timeout: Duration) -> io::Result<Option<Vec<u8>>> {
    Ok(None)
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
    fn differential_updates_only_touch_changed_rows() {
        let previous = vec!["header".into(), "old".into(), "footer".into()];
        let next = vec!["header".into(), "new".into(), "footer".into()];
        let differential = compose_update(&previous, &next);
        let initial = compose_update(&[], &next);
        assert_eq!(differential.matches("\x1b[2K").count(), 1);
        assert_eq!(initial.matches("\x1b[2K").count(), 3);
        assert!(differential.contains("\x1b[2;1H"));
        assert!(!differential.contains("\x1b[1;1H"));
        assert!(compose_update(&next, &next).is_empty());
    }
}
