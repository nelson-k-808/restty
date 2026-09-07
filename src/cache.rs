use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::model::{Cell, ChangeKind, Table, View};
use crate::{prepare_output, RunOptions};

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub source: String,
    pub command: String,
    pub argv: Vec<String>,
    pub timestamp: u64,
    pub raw: Vec<u8>,
    pub rows_json: String,
}

pub fn store_if_parsed(
    input: &[u8],
    options: &RunOptions,
    config: &Config,
    command: &[String],
) -> io::Result<bool> {
    let prepared = prepare_output(input, options, config, true, false);
    let Some(view) = prepared.view() else {
        return Ok(false);
    };
    let Some(root) = cache_root() else {
        return Ok(false);
    };
    fs::create_dir_all(&root)?;
    let command_text = command.join(" ");
    let key = cache_key(&options.source, command);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = CacheEntry {
        source: options.source.clone(),
        command: command_text,
        argv: command.to_vec(),
        timestamp,
        raw: input.to_vec(),
        rows_json: crate::structured::serialize(view, crate::structured::StructuredFormat::Json)
            .trim()
            .to_owned(),
    };
    atomic_write(
        &root.join(format!("{key}.json")),
        encode_entry(&entry).as_bytes(),
    )?;
    atomic_write(&root.join("latest"), key.as_bytes())?;
    Ok(true)
}

pub fn latest() -> io::Result<Option<CacheEntry>> {
    let Some(root) = cache_root() else {
        return Ok(None);
    };
    let Ok(key) = fs::read_to_string(root.join("latest")) else {
        return Ok(None);
    };
    let key = key.trim();
    if key.len() != 16 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(None);
    }
    read_entry(&root.join(format!("{key}.json")))
}

pub fn snapshot(name: &str) -> io::Result<bool> {
    validate_name(name)?;
    let Some(entry) = latest()? else {
        return Ok(false);
    };
    let Some(root) = cache_root() else {
        return Ok(false);
    };
    let snapshots = root.join("snapshots");
    fs::create_dir_all(&snapshots)?;
    let path = snapshots.join(format!("{name}.json"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(encode_entry(&entry).as_bytes())?;
    Ok(true)
}

pub fn named_snapshot(name: &str) -> io::Result<Option<CacheEntry>> {
    validate_name(name)?;
    let Some(root) = cache_root() else {
        return Ok(None);
    };
    read_entry(&root.join("snapshots").join(format!("{name}.json")))
}

pub fn parsed_view(entry: &CacheEntry, config: &Config) -> Option<View> {
    let options = RunOptions {
        source: entry.source.clone(),
        width: None,
        force: true,
        color: crate::ColorChoice::Never,
    };
    prepare_output(&entry.raw, &options, config, true, false)
        .view()
        .cloned()
}

pub fn diff_views(previous: &View, current: &View, source: &str) -> View {
    let (View::Table(previous), View::Table(current)) = (previous, current) else {
        return current.clone();
    };
    if previous.columns.is_empty() || current.columns.is_empty() {
        return View::Table(current.clone());
    }
    let key = key_index(source, current);
    let previous_key = previous
        .columns
        .iter()
        .position(|column| column.key == current.columns[key].key)
        .unwrap_or_else(|| key.min(previous.columns.len() - 1));
    let old_rows = previous
        .rows
        .iter()
        .filter_map(|row| row.get(previous_key).map(|cell| (cell.text.clone(), row)))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    let mut added = 0;
    let mut changed = 0;
    for row in &current.rows {
        let row_key = row.get(key).map(|cell| cell.text.as_str()).unwrap_or("");
        seen.insert(row_key.to_owned());
        if let Some(old) = old_rows.get(row_key) {
            let mut row = row.clone();
            let mut row_changed = false;
            for (index, cell) in row.iter_mut().enumerate() {
                let old_index = current.columns.get(index).and_then(|column| {
                    previous
                        .columns
                        .iter()
                        .position(|old_column| old_column.key == column.key)
                });
                if old_index
                    .and_then(|old_index| old.get(old_index))
                    .map(|old| old.text.as_str())
                    != Some(cell.text.as_str())
                {
                    cell.change = Some(ChangeKind::Changed);
                    row_changed = true;
                }
            }
            changed += usize::from(row_changed);
            rows.push(row);
        } else {
            added += 1;
            rows.push(mark_row(row, ChangeKind::Added));
        }
    }
    let mut removed = 0;
    for old in &previous.rows {
        let row_key = old
            .get(previous_key)
            .map(|cell| cell.text.as_str())
            .unwrap_or("");
        if !seen.contains(row_key) {
            removed += 1;
            let aligned = current
                .columns
                .iter()
                .map(|column| {
                    previous
                        .columns
                        .iter()
                        .position(|old_column| old_column.key == column.key)
                        .and_then(|index| old.get(index))
                        .cloned()
                        .unwrap_or_else(|| Cell::plain(""))
                        .with_change(ChangeKind::Removed)
                })
                .collect();
            rows.push(aligned);
        }
    }
    let mut table = current.clone();
    table.rows = rows;
    table
        .prelude
        .push(format!("changes: +{added} ~{changed} -{removed}"));
    View::Table(table)
}

fn mark_row(row: &[Cell], change: ChangeKind) -> Vec<Cell> {
    row.iter()
        .cloned()
        .map(|cell| cell.with_change(change))
        .collect()
}

pub(crate) fn key_index(source: &str, table: &Table) -> usize {
    crate::parser::key_columns(source)
        .iter()
        .find_map(|key| table.columns.iter().position(|column| column.key == *key))
        .unwrap_or(0)
}

fn cache_root() -> Option<PathBuf> {
    if let Some(root) = env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(root).join("restty"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache/restty"))
}

fn cache_key(source: &str, command: &[String]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in source.bytes().chain(std::iter::once(0)).chain(
        command
            .iter()
            .flat_map(|arg| arg.bytes().chain(std::iter::once(0))),
    ) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn encode_entry(entry: &CacheEntry) -> String {
    format!(
        "{{\n  \"version\": 1,\n  \"source\": {},\n  \"command\": {},\n  \"argv\": {},\n  \"timestamp\": {},\n  \"rows\": {},\n  \"raw_hex\": \"{}\"\n}}\n",
        json_string(&entry.source),
        json_string(&entry.command),
        json_string_array(&entry.argv),
        entry.timestamp,
        entry.rows_json,
        hex_encode(&entry.raw)
    )
}

fn read_entry(path: &Path) -> io::Result<Option<CacheEntry>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(source) = json_string_field(&contents, "source") else {
        return Ok(None);
    };
    let Some(command) = json_string_field(&contents, "command") else {
        return Ok(None);
    };
    let argv = json_string_array_field(&contents, "argv").unwrap_or_default();
    let Some(raw) = json_string_field(&contents, "raw_hex").and_then(|value| hex_decode(&value))
    else {
        return Ok(None);
    };
    let timestamp = json_number_field(&contents, "timestamp").unwrap_or(0);
    let rows_json = json_array_field(&contents, "rows").unwrap_or_else(|| "[]".into());
    Ok(Some(CacheEntry {
        source,
        command,
        argv,
        timestamp,
        raw,
        rows_json,
    }))
}

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::rename(temporary, path)
}

fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "snapshot names may contain only letters, digits, '.', '-' and '_'",
        ));
    }
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch < '\u{20}' => output.push(' '),
            ch => output.push(ch),
        }
    }
    output.push('"');
    output
}

fn json_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn json_string_array_field(input: &str, field: &str) -> Option<Vec<String>> {
    let array = json_array_field(input, field)?;
    let mut values = Vec::new();
    let mut index = 1_usize;
    let bytes = array.as_bytes();
    while index + 1 < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b',') {
            index += 1;
        }
        if index + 1 >= bytes.len() || bytes[index] == b']' {
            break;
        }
        if bytes[index] != b'"' {
            return None;
        }
        let tail = &array[index..];
        let wrapped = format!("{{\"value\":{tail}}}");
        let value = json_string_field(&wrapped, "value")?;
        let mut escaped = false;
        index += 1;
        while index < bytes.len() {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == b'"' {
                index += 1;
                break;
            }
            index += 1;
        }
        values.push(value);
    }
    Some(values)
}

fn json_string_field(input: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
    let tail = input.get(input.find(&marker)? + marker.len()..)?;
    let mut chars = tail.get(tail.find(':')? + 1..)?.trim_start().chars();
    (chars.next()? == '"').then_some(())?;
    let mut output = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            output.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(output);
        } else {
            output.push(ch);
        }
    }
    None
}

fn json_number_field(input: &str, field: &str) -> Option<u64> {
    let marker = format!("\"{field}\"");
    let tail = input.get(input.find(&marker)? + marker.len()..)?;
    tail.get(tail.find(':')? + 1..)?
        .trim_start()
        .split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn json_array_field(input: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
    let tail = input.get(input.find(&marker)? + marker.len()..)?;
    let value = tail.get(tail.find(':')? + 1..)?.trim_start();
    if !value.starts_with('[') {
        return None;
    }
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(value[..=index].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

fn hex_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn hex_decode(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None;
    }
    (0..input.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&input[index..index + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Alignment, Column};

    #[test]
    fn cache_entry_round_trips() {
        let entry = CacheEntry {
            source: "ps".into(),
            command: "ps aux".into(),
            argv: vec!["ps".into(), "aux".into()],
            timestamp: 42,
            raw: b"a\0b\n".to_vec(),
            rows_json: "[{\"a\": \"b\"}]".into(),
        };
        let encoded = encode_entry(&entry);
        assert_eq!(json_string_field(&encoded, "source").as_deref(), Some("ps"));
        assert_eq!(json_string_array_field(&encoded, "argv"), Some(entry.argv));
        assert_eq!(hex_decode(&hex_encode(&entry.raw)), Some(entry.raw));
        assert_eq!(
            json_array_field(&encoded, "rows").as_deref(),
            Some("[{\"a\": \"b\"}]")
        );
    }

    #[test]
    fn marks_added_changed_and_removed_rows() {
        let table = |rows: &[(&str, &str)]| {
            View::Table(Table {
                columns: vec![
                    Column::new("id", "ID", 0, Alignment::Left),
                    Column::new("value", "VALUE", 1, Alignment::Left),
                ],
                rows: rows
                    .iter()
                    .map(|(id, value)| vec![Cell::plain(*id), Cell::plain(*value)])
                    .collect(),
                prelude: Vec::new(),
            })
        };
        let View::Table(diff) = diff_views(
            &table(&[("a", "old"), ("gone", "x")]),
            &table(&[("a", "new"), ("added", "y")]),
            "generic",
        ) else {
            panic!("expected table");
        };
        assert_eq!(diff.rows[0][1].change, Some(ChangeKind::Changed));
        assert_eq!(diff.rows[1][0].change, Some(ChangeKind::Added));
        assert_eq!(diff.rows[2][0].change, Some(ChangeKind::Removed));
    }
}
