use restty::config::Config;
use restty::{render_bytes_with_policy, ColorChoice, RunOptions};
use unicode_width::UnicodeWidthStr;

const CASES: &[(&str, &str, &str)] = &[
    ("lsblk", include_str!("fixtures/lsblk.txt"), "MOUNTPOINTS"),
    ("free", include_str!("fixtures/free.txt"), "AVAILABLE"),
    ("ip", include_str!("fixtures/ip-route.txt"), "DESTINATION"),
];

#[test]
fn common_linux_commands_render_responsively() {
    let config = Config {
        terminal_profile: "unicode".into(),
        ..Config::default()
    };
    for (source, input, wide_heading) in CASES {
        for width in [32, 72, 120] {
            let output = render_bytes_with_policy(
                input.as_bytes(),
                &RunOptions {
                    source: (*source).into(),
                    width: Some(width),
                    force: true,
                    color: ColorChoice::Never,
                },
                &config,
                false,
                false,
            );
            let output = std::str::from_utf8(&output).unwrap();
            assert!(
                output.starts_with('╭'),
                "{source} was not formatted at {width}"
            );
            assert!(
                output
                    .lines()
                    .all(|line| UnicodeWidthStr::width(line) <= width),
                "{source} exceeded {width} columns:\n{output}"
            );
            if width == 120 {
                assert!(output.contains(wide_heading));
            }
        }
    }
}
