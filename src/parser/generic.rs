use crate::model::{Alignment, Cell, Column, LogRecord, Style, View};

use super::util::{looks_numeric, split_whitespace_limit, style_for_level, table};
use super::Parser;

pub fn all() -> Vec<Box<dyn Parser>> {
    vec![
        Box::new(GenericTableParser),
        Box::new(KeyValueParser),
        Box::new(LogParser),
    ]
}

struct GenericTableParser;

impl Parser for GenericTableParser {
    fn name(&self) -> &'static str {
        "generic-table"
    }

    fn detect(&self, _source: &str, first_lines: &str) -> bool {
        let lines = first_lines
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        if lines.len() < 2 {
            return false;
        }
        if looks_like_log(lines[0])
            || lines
                .iter()
                .filter(|line| looks_like_key_value(line))
                .count()
                * 4
                >= lines.len() * 3
        {
            return false;
        }
        let header_words = lines[0].split_whitespace().collect::<Vec<_>>();
        let uppercase_words = header_words
            .iter()
            .filter(|word| {
                word.chars().any(char::is_alphabetic)
                    && word.chars().all(|ch| !ch.is_ascii_lowercase())
            })
            .count();
        if uppercase_words * 2 < header_words.len() {
            return false;
        }
        if header_words.len() < 2 {
            return false;
        }
        let structured = lines[1..]
            .iter()
            .filter(|line| split_whitespace_limit(line, header_words.len()).len() >= 2)
            .count();
        structured > 0 && structured * 5 >= (lines.len() - 1) * 3
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut lines = input.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next()?;
        let labels = header
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if labels.len() < 2 {
            return None;
        }
        let mut rows = Vec::new();
        let mut structured_rows = 0;
        for line in lines {
            let mut fields = split_whitespace_limit(line, labels.len());
            let populated = fields.len();
            if populated == 0 || (populated == 1 && rows.is_empty()) {
                return None;
            }
            structured_rows += usize::from(populated >= 2);
            if populated == 1 {
                let continuation = fields.pop().unwrap_or_default();
                fields.resize(labels.len(), String::new());
                if let Some(last) = fields.last_mut() {
                    *last = continuation;
                }
            } else {
                fields.resize(labels.len(), String::new());
            }
            rows.push(fields.into_iter().map(Cell::plain).collect::<Vec<_>>());
        }
        if structured_rows * 5 < rows.len() * 3 {
            return None;
        }
        let columns = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let numeric = rows
                    .iter()
                    .filter_map(|row| row.get(index))
                    .filter(|cell| !cell.text.is_empty())
                    .all(|cell| looks_numeric(&cell.text));
                Column::new(
                    &normalize_key(label),
                    label,
                    if index == 0 { 0 } else { (index.min(3)) as u8 },
                    if numeric {
                        Alignment::Right
                    } else {
                        Alignment::Left
                    },
                )
            })
            .collect();
        Some(View::Table(table(columns, rows)?))
    }

    fn confidence(&self) -> f32 {
        0.80
    }
}

struct KeyValueParser;

impl Parser for KeyValueParser {
    fn name(&self) -> &'static str {
        "generic-key-value"
    }

    fn detect(&self, _source: &str, first_lines: &str) -> bool {
        let lines = first_lines
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        lines.len() >= 2
            && lines
                .iter()
                .filter(|line| looks_like_key_value(line))
                .count()
                * 4
                >= lines.len() * 3
    }

    fn parse(&self, input: &str) -> Option<View> {
        let rows = input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (key, value) = line.split_once(':')?;
                let key = key.trim();
                let value = value.trim();
                (!key.is_empty() && !value.is_empty())
                    .then(|| vec![Cell::styled(key, Style::Accent), Cell::plain(value)])
            })
            .collect::<Option<Vec<_>>>()?;
        Some(View::Table(table(
            vec![
                Column::new("property", "PROPERTY", 1, Alignment::Left),
                Column::new("value", "VALUE", 0, Alignment::Left),
            ],
            rows,
        )?))
    }

    fn confidence(&self) -> f32 {
        0.82
    }
}

struct LogParser;

impl Parser for LogParser {
    fn name(&self) -> &'static str {
        "generic-log"
    }

    fn detect(&self, _source: &str, first_lines: &str) -> bool {
        let lines = first_lines
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        !lines.is_empty()
            && lines.iter().filter(|line| looks_like_log(line)).count() * 4 >= lines.len() * 3
    }

    fn parse(&self, input: &str) -> Option<View> {
        let records = input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(parse_log_line)
            .collect::<Vec<_>>();
        (!records.is_empty()).then_some(View::Logs(records))
    }

    fn confidence(&self) -> f32 {
        0.84
    }
}

fn normalize_key(label: &str) -> String {
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn looks_like_log(line: &str) -> bool {
    let trimmed = line.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}') && trimmed.contains(':'))
        || extract_timestamp(trimmed).is_some()
        || extract_level(trimmed).is_some()
}

fn looks_like_key_value(line: &str) -> bool {
    line.split_once(':').is_some_and(|(key, value)| {
        let key = key.trim();
        !key.is_empty()
            && !value.trim().is_empty()
            && key.len() <= 40
            && key.chars().next().is_some_and(char::is_alphabetic)
    })
}

fn parse_log_line(line: &str) -> LogRecord {
    let trimmed = line.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') && trimmed.contains(':') {
        return LogRecord {
            timestamp: None,
            level: Some("JSON".into()),
            message: trimmed.into(),
            style: Style::Info,
        };
    }

    let timestamp = extract_timestamp(trimmed);
    let level = extract_level(trimmed);
    let style = level
        .as_deref()
        .map(style_for_level)
        .unwrap_or(Style::Plain);
    let mut message = trimmed;
    if let Some(timestamp) = &timestamp {
        message = message
            .strip_prefix(timestamp)
            .unwrap_or(message)
            .trim_start();
    }
    if let Some(level) = &level {
        if let Some(position) = message.to_ascii_uppercase().find(level) {
            let after = position + level.len();
            message = message
                .get(after..)
                .unwrap_or_default()
                .trim_start_matches([' ', '\t', ':', '-', ']', '[']);
        }
    }
    LogRecord {
        timestamp,
        level,
        message: message.to_owned(),
        style,
    }
}

fn extract_timestamp(line: &str) -> Option<String> {
    let candidate = line.split_whitespace().next()?.trim_start_matches('[');
    let candidate = candidate.trim_end_matches(']');
    let bytes = candidate.as_bytes();
    let iso_date = bytes.len() >= 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes[..4].iter().all(u8::is_ascii_digit);
    let clock = bytes.len() >= 8
        && bytes.get(2) == Some(&b':')
        && bytes.get(5) == Some(&b':')
        && bytes[..2].iter().all(u8::is_ascii_digit);
    (iso_date || clock).then(|| candidate.to_owned())
}

fn extract_level(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let cleaned = word.trim_matches(|ch: char| !ch.is_ascii_alphabetic());
        matches!(
            cleaned.to_ascii_uppercase().as_str(),
            "ERROR"
                | "ERR"
                | "FATAL"
                | "CRITICAL"
                | "WARN"
                | "WARNING"
                | "INFO"
                | "NOTICE"
                | "DEBUG"
                | "TRACE"
        )
        .then(|| cleaned.to_ascii_uppercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_log_shapes() {
        assert!(looks_like_log("2026-09-03T12:01:02Z INFO ready"));
        assert!(looks_like_log("[ERROR] request failed"));
        assert!(looks_like_log(r#"{"level":"info","message":"ready"}"#));
        assert!(!looks_like_log("ordinary prose"));
    }

    #[test]
    fn recognizes_key_value_reports_without_stealing_timestamped_logs() {
        let parser = KeyValueParser;
        assert!(parser.detect("lscpu", "Architecture: x86_64\nCPU(s): 16\n"));
        assert!(!parser.detect(
            "generic",
            "2026-09-03T12:00:01Z INFO ready\n2026-09-03T12:00:02Z WARN slow\n"
        ));
    }
}
