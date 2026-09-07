use restty::config::Config;
use restty::{render_bytes_with_policy, ColorChoice, RunOptions};

const CASES: &[(&str, &str)] = &[
    ("ls", include_str!("fixtures/ls.txt")),
    ("ps", include_str!("fixtures/ps.txt")),
    ("df", include_str!("fixtures/df.txt")),
    ("du", include_str!("fixtures/du.txt")),
    ("git", include_str!("fixtures/git-status.txt")),
    ("git", include_str!("fixtures/git-log.txt")),
    ("docker", include_str!("fixtures/docker-ps.txt")),
    ("generic", include_str!("fixtures/generic-table.txt")),
    ("generic", include_str!("fixtures/logs.txt")),
    ("df", include_str!("fixtures/df-bsd.txt")),
];

#[test]
fn parser_and_width_goldens() {
    let config = Config {
        border_style: "none".into(),
        ..Config::default()
    };
    let mut actual = String::new();
    for (case_index, (source, input)) in CASES.iter().enumerate() {
        for width in [48, 80, 120] {
            let output = render_bytes_with_policy(
                input.as_bytes(),
                &RunOptions {
                    source: (*source).to_owned(),
                    width: Some(width),
                    force: true,
                    color: ColorChoice::Never,
                },
                &config,
                false,
                false,
            );
            actual.push_str(&format!(
                "=== case {case_index} source={source} width={width} ===\n"
            ));
            actual.push_str(std::str::from_utf8(&output).unwrap());
        }
    }
    assert_eq!(actual, include_str!("golden/all.txt"));
}

#[test]
fn polished_border_golden() {
    let config = Config {
        terminal_profile: "unicode".into(),
        ..Config::default()
    };
    let output = render_bytes_with_policy(
        include_str!("fixtures/ps.txt").as_bytes(),
        &RunOptions {
            source: "ps".into(),
            width: Some(80),
            force: true,
            color: ColorChoice::Never,
        },
        &config,
        false,
        false,
    );
    assert_eq!(
        std::str::from_utf8(&output).unwrap(),
        include_str!("golden/bordered.txt")
    );
}
