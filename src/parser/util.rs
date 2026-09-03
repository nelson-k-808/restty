use crate::model::{Alignment, Cell, Column, Style, Table};

pub fn is_permissions(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 10
        && matches!(bytes[0], b'-' | b'd' | b'l' | b'b' | b'c' | b'p' | b's')
        && bytes[1..10]
            .iter()
            .all(|byte| matches!(*byte, b'r' | b'w' | b'x' | b's' | b'S' | b't' | b'T' | b'-'))
}

pub fn is_month(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "jan"
            | "feb"
            | "mar"
            | "apr"
            | "may"
            | "jun"
            | "jul"
            | "aug"
            | "sep"
            | "oct"
            | "nov"
            | "dec"
    )
}

pub fn style_for_level(level: &str) -> Style {
    match level.to_ascii_uppercase().as_str() {
        "ERROR" | "ERR" | "FATAL" | "CRITICAL" => Style::Error,
        "WARN" | "WARNING" => Style::Warning,
        "INFO" | "NOTICE" => Style::Info,
        "DEBUG" | "TRACE" => Style::Debug,
        _ => Style::Plain,
    }
}

pub fn table(columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Option<Table> {
    (!rows.is_empty() && rows.iter().all(|row| row.len() == columns.len())).then_some(Table {
        columns,
        rows,
        prelude: Vec::new(),
    })
}

pub fn col(key: &str, label: &str, priority: u8) -> Column {
    Column::new(key, label, priority, Alignment::Left)
}

pub fn num_col(key: &str, label: &str, priority: u8) -> Column {
    Column::new(key, label, priority, Alignment::Right)
}

pub fn aligned_starts(header: &str) -> Vec<usize> {
    let bytes = header.as_bytes();
    let mut starts = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        starts.push(index);
        while index < bytes.len() {
            if bytes[index].is_ascii_whitespace() {
                let run = index;
                while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                if index.saturating_sub(run) >= 2 {
                    break;
                }
            } else {
                index += 1;
            }
        }
    }
    starts
}

pub fn split_at_starts(line: &str, starts: &[usize]) -> Vec<String> {
    starts
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = starts.get(position + 1).copied().unwrap_or(line.len());
            if *start >= line.len() {
                String::new()
            } else {
                line.get(*start..end.min(line.len()))
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            }
        })
        .collect()
}

pub fn split_whitespace_limit(input: &str, fields: usize) -> Vec<String> {
    if fields == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut rest = input.trim();
    while result.len() + 1 < fields {
        let Some(end) = rest.find(char::is_whitespace) else {
            break;
        };
        result.push(rest[..end].to_owned());
        rest = rest[end..].trim_start();
    }
    if !rest.is_empty() {
        result.push(rest.to_owned());
    }
    result
}

pub fn looks_numeric(value: &str) -> bool {
    let trimmed = value.trim_matches(|ch: char| matches!(ch, '%' | '+' | '-' | '.' | ','));
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit())
}
