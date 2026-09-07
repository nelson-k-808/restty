use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::model::View;

#[derive(Debug, Clone, Default)]
pub struct ThresholdRule {
    pub warning: Option<f64>,
    pub error: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub struct Breakpoints {
    pub compact: usize,
    pub core: usize,
    pub wide: usize,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub enabled_commands: Vec<String>,
    pub disabled_parsers: Vec<String>,
    pub max_lines: usize,
    pub max_bytes: usize,
    pub min_confidence: f32,
    pub theme: String,
    pub border_style: String,
    pub terminal_profile: String,
    pub watch_samples: usize,
    pub breakpoints: Breakpoints,
    pub column_priorities: HashMap<String, HashMap<String, u8>>,
    pub thresholds: HashMap<String, HashMap<String, ThresholdRule>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled_commands: [
                "ls", "ps", "df", "du", "git", "docker", "lsblk", "free", "ip",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            disabled_parsers: Vec::new(),
            max_lines: 10_000,
            max_bytes: 8 * 1024 * 1024,
            min_confidence: 0.72,
            theme: "default".into(),
            border_style: "rounded".into(),
            terminal_profile: "auto".into(),
            watch_samples: 20,
            breakpoints: Breakpoints {
                compact: 60,
                core: 100,
                wide: 160,
            },
            column_priorities: HashMap::new(),
            thresholds: HashMap::new(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let mut config = Self::default();
        if let Some(path) = config_path() {
            if let Ok(contents) = fs::read_to_string(path) {
                config.merge_toml_subset(&contents);
            }
        }
        if let Ok(commands) = env::var("RESTTY_ENABLED_CMDS") {
            let parsed = commands
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if !parsed.is_empty() {
                config.enabled_commands = parsed;
            }
        }
        config.normalize_enabled_commands();
        config
    }

    fn normalize_enabled_commands(&mut self) {
        let mut normalized = Vec::new();
        for command in &self.enabled_commands {
            let Some(executable) = command.split_whitespace().next() else {
                continue;
            };
            if !executable.is_empty() && !normalized.iter().any(|item| item == executable) {
                normalized.push(executable.to_owned());
            }
        }
        self.enabled_commands = normalized;
    }

    pub fn merge_toml_subset(&mut self, input: &str) {
        let mut section = String::new();
        for raw_line in input.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_owned();
                continue;
            }
            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let key = raw_key.trim();
            let value = raw_value.trim();
            match section.as_str() {
                "" => match key {
                    "enabled_commands" => set_list(value, &mut self.enabled_commands),
                    "disabled_parsers" => set_list(value, &mut self.disabled_parsers),
                    "max_lines" => set_usize(value, &mut self.max_lines),
                    "max_bytes" => set_usize(value, &mut self.max_bytes),
                    "min_confidence" => {
                        if let Ok(parsed) = value.parse::<f32>() {
                            if (0.0..=1.0).contains(&parsed) {
                                self.min_confidence = parsed;
                            }
                        }
                    }
                    "theme" => self.theme = unquote(value).to_owned(),
                    "border_style" => self.border_style = unquote(value).to_owned(),
                    "terminal_profile" => self.terminal_profile = unquote(value).to_owned(),
                    "watch_samples" => {
                        if let Ok(samples) = value.parse::<usize>() {
                            self.watch_samples = samples.clamp(2, 1_024);
                        }
                    }
                    _ => {}
                },
                "breakpoints" => match key {
                    "compact" => set_usize(value, &mut self.breakpoints.compact),
                    "core" => set_usize(value, &mut self.breakpoints.core),
                    "wide" => set_usize(value, &mut self.breakpoints.wide),
                    _ => {}
                },
                _ if section.starts_with("columns.") => {
                    if let Ok(priority) = value.parse::<u8>() {
                        let command = section.trim_start_matches("columns.").to_owned();
                        self.column_priorities
                            .entry(command)
                            .or_default()
                            .insert(key.to_owned(), priority);
                    }
                }
                _ if section.starts_with("thresholds.") => {
                    let path = section.trim_start_matches("thresholds.");
                    let Some((command, column)) = path.split_once('.') else {
                        continue;
                    };
                    let Ok(number) = value.parse::<f64>() else {
                        continue;
                    };
                    let rule = self
                        .thresholds
                        .entry(command.to_owned())
                        .or_default()
                        .entry(column.to_owned())
                        .or_default();
                    match key {
                        "warning" => rule.warning = Some(number),
                        "error" => rule.error = Some(number),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if !(self.breakpoints.compact < self.breakpoints.core
            && self.breakpoints.core < self.breakpoints.wide)
        {
            self.breakpoints = Self::default().breakpoints;
        }
    }

    pub fn apply_column_priorities(&self, source: &str, view: &mut View) {
        let View::Table(table) = view else {
            return;
        };
        let Some(overrides) = self.column_priorities.get(source) else {
            return;
        };
        for column in &mut table.columns {
            if let Some(priority) = overrides.get(&column.key) {
                column.priority = *priority;
            }
        }
    }

    pub fn apply_thresholds(&self, source: &str, view: &mut View) {
        let View::Table(table) = view else {
            return;
        };
        let Some(rules) = self.thresholds.get(source) else {
            return;
        };
        for (index, column) in table.columns.iter().enumerate() {
            let Some(rule) = rules.get(&column.key) else {
                continue;
            };
            for row in &mut table.rows {
                let Some(cell) = row.get_mut(index) else {
                    continue;
                };
                let Ok(value) = cell
                    .text
                    .trim()
                    .trim_end_matches('%')
                    .replace(',', "")
                    .parse::<f64>()
                else {
                    continue;
                };
                if rule.error.is_some_and(|threshold| value >= threshold) {
                    cell.style = crate::model::Style::Error;
                } else if rule.warning.is_some_and(|threshold| value >= threshold) {
                    cell.style = crate::model::Style::Warning;
                }
            }
        }
    }
}

fn config_path() -> Option<PathBuf> {
    if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(root).join("restty/config.toml"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/restty/config.toml"))
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (index, ch) in line.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == '#' && !quoted {
            return &line[..index];
        }
    }
    line
}

fn set_usize(value: &str, target: &mut usize) {
    if let Ok(parsed) = value.parse::<usize>() {
        if parsed > 0 {
            *target = parsed;
        }
    }
}

fn set_list(value: &str, target: &mut Vec<String>) {
    let trimmed = value.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return;
    }
    let parsed = trimmed[1..trimmed.len() - 1]
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    *target = parsed;
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_config_shape() {
        let mut config = Config::default();
        config.merge_toml_subset(
            r#"
            enabled_commands = ["ls", "git"]
            max_lines = 42
            theme = "none"
            border_style = "ascii"
            [breakpoints]
            compact = 50
            core = 90
            wide = 140
            [columns.ls]
            owner = 3
            "#,
        );
        assert_eq!(config.enabled_commands, ["ls", "git"]);
        assert_eq!(config.max_lines, 42);
        assert_eq!(config.breakpoints.core, 90);
        assert_eq!(config.border_style, "ascii");
        assert_eq!(config.column_priorities["ls"]["owner"], 3);
    }

    #[test]
    fn normalizes_command_specs_to_executable_names() {
        let mut config = Config {
            enabled_commands: vec!["ip route".into(), "".into(), "free".into(), "ip".into()],
            ..Config::default()
        };
        config.normalize_enabled_commands();
        assert_eq!(config.enabled_commands, ["ip", "free"]);
    }

    #[test]
    fn threshold_rules_override_numeric_cell_styles() {
        use crate::model::{Alignment, Cell, Column, Style, Table};
        let mut config = Config::default();
        config.merge_toml_subset(
            "terminal_profile = \"ascii\"\nwatch_samples = 32\n[thresholds.df.use]\nwarning = 75\nerror = 90\n",
        );
        assert_eq!(config.terminal_profile, "ascii");
        assert_eq!(config.watch_samples, 32);
        let mut view = View::Table(Table {
            columns: vec![Column::new("use", "USE", 0, Alignment::Right)],
            rows: vec![vec![Cell::plain("76%")], vec![Cell::plain("91%")]],
            prelude: Vec::new(),
        });
        config.apply_thresholds("df", &mut view);
        let View::Table(table) = view else {
            panic!("expected table");
        };
        assert_eq!(table.rows[0][0].style, Style::Warning);
        assert_eq!(table.rows[1][0].style, Style::Error);
    }
}
