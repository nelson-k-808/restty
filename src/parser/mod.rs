mod builtins;
mod generic;
mod jc;
mod util;

use std::collections::HashSet;

use crate::model::{ParsedView, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    OpenPath,
    TerminateProcess,
}

#[derive(Debug, Clone, Copy)]
pub struct ActionSpec {
    pub label: &'static str,
    pub value_key: &'static str,
    pub kind: ActionKind,
}

const LS_ACTIONS: &[ActionSpec] = &[ActionSpec {
    label: "open",
    value_key: "name",
    kind: ActionKind::OpenPath,
}];

const PS_ACTIONS: &[ActionSpec] = &[ActionSpec {
    label: "terminate",
    value_key: "pid",
    kind: ActionKind::TerminateProcess,
}];

pub fn actions_for(source: &str) -> &'static [ActionSpec] {
    match source {
        "ls" => LS_ACTIONS,
        "ps" => PS_ACTIONS,
        _ => &[],
    }
}

pub fn key_columns(source: &str) -> &'static [&'static str] {
    match source {
        "ls" => &["name"],
        "ps" => &["pid"],
        "df" => &["mounted", "filesystem"],
        "du" => &["path"],
        "git" => &["path", "commit"],
        "docker" => &["container_id", "names"],
        "lsblk" => &["name"],
        "free" => &["type"],
        "ip" => &["destination"],
        _ => &[],
    }
}

pub trait Parser: Send + Sync {
    fn name(&self) -> &'static str;
    fn detect(&self, source: &str, first_lines: &str) -> bool;
    fn parse(&self, input: &str) -> Option<View>;
    fn confidence(&self) -> f32 {
        0.96
    }
}

pub struct ParserRegistry {
    parsers: Vec<Box<dyn Parser>>,
}

impl ParserRegistry {
    pub fn new(disabled: &[String]) -> Self {
        let disabled = disabled.iter().map(String::as_str).collect::<HashSet<_>>();
        let parsers = builtins::all()
            .into_iter()
            .chain(jc::all())
            .chain(generic::all())
            .filter(|parser| !disabled.contains(parser.name()))
            .collect();
        Self { parsers }
    }

    pub fn parse(&self, source: &str, input: &str) -> Option<ParsedView> {
        let first_lines = input.lines().take(8).collect::<Vec<_>>().join("\n");
        self.parsers.iter().find_map(|parser| {
            if !parser.detect(source, &first_lines) {
                return None;
            }
            parser.parse(input).map(|view| ParsedView {
                view,
                confidence: parser.confidence(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_utf8_never_panics() {
        let registry = ParserRegistry::new(&[]);
        let mut state = 0x1234_5678_u64;
        for length in 0..512 {
            let text = (0..length)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    char::from_u32((state % 0x11_0000) as u32).unwrap_or('\u{fffd}')
                })
                .collect::<String>();
            for source in ["unknown", "ls", "ps", "df", "du", "git", "docker"] {
                let _ = registry.parse(source, &text);
            }
        }
        let _ = registry.parse("git", "界 x\n");
        let _ = registry.parse("generic", "NAME  VALUE\n界界  ok\n");
    }
}
