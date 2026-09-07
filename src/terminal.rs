use std::env;
use std::io::IsTerminal;

pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

pub fn formatting_disabled() -> bool {
    env::var_os("NO_COLOR").is_some() || env::var("RESTTY_DISABLE").ok().as_deref() == Some("1")
}

pub fn supports_unicode(profile: &str) -> bool {
    match profile {
        "ascii" => false,
        "unicode" | "truecolor" | "ansi16" => true,
        _ => {
            if env::var("TERM").ok().as_deref() == Some("dumb") {
                return false;
            }
            env::var("LC_ALL")
                .ok()
                .or_else(|| env::var("LC_CTYPE").ok())
                .or_else(|| env::var("LANG").ok())
                .map(|locale| {
                    let locale = locale.to_ascii_lowercase();
                    locale.contains("utf-8") || locale.contains("utf8")
                })
                .unwrap_or(true)
        }
    }
}

pub fn supports_truecolor(profile: &str) -> bool {
    match profile {
        "truecolor" => true,
        "ascii" | "ansi16" | "unicode" => false,
        _ => {
            let colorterm = env::var("COLORTERM")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
            matches!(colorterm.as_str(), "truecolor" | "24bit")
                || term.contains("truecolor")
                || term.contains("direct")
        }
    }
}

#[repr(C)]
#[derive(Default)]
struct WinSize {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const TIOCGWINSZ: usize = 0x5413;

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
const TIOCGWINSZ: usize = 0x4008_7468;

#[cfg(unix)]
unsafe extern "C" {
    fn ioctl(fd: i32, request: usize, ...) -> i32;
}

#[cfg(unix)]
pub fn terminal_size() -> Option<(usize, usize)> {
    let mut size = WinSize::default();
    // SAFETY: stdout is fd 1 and `size` is a writable winsize-compatible struct.
    let result = unsafe { ioctl(1, TIOCGWINSZ, &mut size) };
    (result == 0 && size.cols > 0 && size.rows > 0)
        .then_some((size.cols as usize, size.rows as usize))
}

#[cfg(unix)]
pub fn terminal_width() -> Option<usize> {
    terminal_size().map(|(width, _)| width)
}

#[cfg(not(unix))]
pub fn terminal_width() -> Option<usize> {
    None
}

#[cfg(not(unix))]
pub fn terminal_size() -> Option<(usize, usize)> {
    None
}
