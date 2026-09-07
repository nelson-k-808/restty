use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::model::{Alignment, Cell, Column, Style, Table, View};

use super::Parser;

static JC_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

const ADAPTERS: &[(&str, &str)] = &[
    ("arp", "arp"),
    ("blkid", "blkid"),
    ("crontab", "crontab"),
    ("date", "date"),
    ("df", "df"),
    ("dig", "dig"),
    ("du", "du"),
    ("env", "env"),
    ("find", "find"),
    ("free", "free"),
    ("fstab", "fstab"),
    ("getent", "getent"),
    ("history", "history"),
    ("hosts", "hosts"),
    ("ifconfig", "ifconfig"),
    ("ip", "ip-route"),
    ("iptables", "iptables"),
    ("last", "last"),
    ("ls", "ls"),
    ("lsblk", "lsblk"),
    ("lsof", "lsof"),
    ("mount", "mount"),
    ("netstat", "netstat"),
    ("passwd", "passwd"),
    ("ping", "ping"),
    ("ps", "ps"),
    ("route", "route"),
    ("sfdisk", "sfdisk"),
    ("ss", "ss"),
    ("stat", "stat"),
    ("sysctl", "sysctl"),
    ("systemctl", "systemctl"),
    ("timedatectl", "timedatectl"),
    ("traceroute", "traceroute"),
    ("ulimit", "ulimit"),
    ("uname", "uname"),
    ("uptime", "uptime"),
    ("w", "w"),
    ("wc", "wc"),
    ("who", "who"),
];

pub fn all() -> Vec<Box<dyn Parser>> {
    if jc_path().is_none() {
        return Vec::new();
    }
    ADAPTERS
        .iter()
        .map(|(source, parser)| Box::new(JcParser { source, parser }) as Box<dyn Parser>)
        .collect()
}

struct JcParser {
    source: &'static str,
    parser: &'static str,
}

impl Parser for JcParser {
    fn name(&self) -> &'static str {
        self.parser
    }

    fn detect(&self, source: &str, _first_lines: &str) -> bool {
        source == self.source
    }

    fn parse(&self, input: &str) -> Option<View> {
        let path = jc_path()?;
        let mut child = Command::new(path)
            .arg(format!("--{}", self.parser))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(input.as_bytes()).ok()?;
        let output = child.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        let json = std::str::from_utf8(&output.stdout).ok()?;
        json_to_view(json)
    }

    fn confidence(&self) -> f32 {
        0.94
    }
}

fn jc_path() -> Option<&'static Path> {
    JC_PATH
        .get_or_init(|| {
            let path = env::var_os("PATH")?;
            env::split_paths(&path)
                .map(|directory| directory.join("jc"))
                .find(|candidate| candidate.is_file())
        })
        .as_deref()
}

#[derive(Debug, Clone)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

fn json_to_view(input: &str) -> Option<View> {
    let mut parser = JsonParser::new(input);
    let value = parser.value()?;
    parser.whitespace();
    if parser.position != parser.input.len() {
        return None;
    }
    let objects = match value {
        JsonValue::Array(values) => values
            .into_iter()
            .map(|value| match value {
                JsonValue::Object(object) => Some(object),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?,
        JsonValue::Object(object) => vec![object],
        _ => return None,
    };
    if objects.is_empty() {
        return None;
    }
    let mut keys = Vec::new();
    for object in &objects {
        for (key, _) in object {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    let columns = keys
        .iter()
        .map(|key| {
            let numeric = objects
                .iter()
                .filter_map(|object| field(object, key))
                .all(|value| matches!(value, JsonValue::Number(_) | JsonValue::Null));
            Column::new(
                key,
                &key.replace('_', " ").to_ascii_uppercase(),
                priority_for(key),
                if numeric {
                    Alignment::Right
                } else {
                    Alignment::Left
                },
            )
        })
        .collect();
    let rows = objects
        .iter()
        .map(|object| {
            keys.iter()
                .map(|key| {
                    let value = field(object, key).map(json_display).unwrap_or_default();
                    Cell::styled(value, style_for(key))
                })
                .collect()
        })
        .collect();
    Some(View::Table(Table {
        columns,
        rows,
        prelude: Vec::new(),
    }))
}

fn field<'a>(object: &'a [(String, JsonValue)], key: &str) -> Option<&'a JsonValue> {
    object
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn priority_for(key: &str) -> u8 {
    let key = key.to_ascii_lowercase();
    if matches!(
        key.as_str(),
        "name" | "path" | "command" | "cmd" | "mountpoint" | "filesystem" | "destination"
    ) {
        0
    } else if key == "pid"
        || key == "id"
        || key == "user"
        || key.contains("percent")
        || key.ends_with("_pct")
    {
        1
    } else {
        2
    }
}

fn style_for(key: &str) -> Style {
    let key = key.to_ascii_lowercase();
    if matches!(key.as_str(), "name" | "path" | "mountpoint" | "destination") {
        Style::Accent
    } else if key.contains("percent") || key.ends_with("_pct") {
        Style::Warning
    } else {
        Style::Plain
    }
}

fn json_display(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) | JsonValue::String(value) => value.clone(),
        JsonValue::Array(_) | JsonValue::Object(_) => json_compact(value),
    }
}

fn json_compact(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "null".into(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.clone(),
        JsonValue::String(value) => format!("\"{}\"", escape_json(value)),
        JsonValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(json_compact)
                .collect::<Vec<_>>()
                .join(",")
        ),
        JsonValue::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!("\"{}\":{}", escape_json(key), json_compact(value)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

struct JsonParser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn value(&mut self) -> Option<JsonValue> {
        self.whitespace();
        match self.peek()? {
            b'n' => {
                self.literal(b"null")?;
                Some(JsonValue::Null)
            }
            b't' => {
                self.literal(b"true")?;
                Some(JsonValue::Bool(true))
            }
            b'f' => {
                self.literal(b"false")?;
                Some(JsonValue::Bool(false))
            }
            b'"' => self.string().map(JsonValue::String),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number().map(JsonValue::Number),
            _ => None,
        }
    }

    fn array(&mut self) -> Option<JsonValue> {
        self.consume(b'[')?;
        let mut values = Vec::new();
        self.whitespace();
        if self.take(b']') {
            return Some(JsonValue::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.whitespace();
            if self.take(b']') {
                break;
            }
            self.consume(b',')?;
        }
        Some(JsonValue::Array(values))
    }

    fn object(&mut self) -> Option<JsonValue> {
        self.consume(b'{')?;
        let mut fields = Vec::new();
        self.whitespace();
        if self.take(b'}') {
            return Some(JsonValue::Object(fields));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.consume(b':')?;
            fields.push((key, self.value()?));
            self.whitespace();
            if self.take(b'}') {
                break;
            }
            self.consume(b',')?;
        }
        Some(JsonValue::Object(fields))
    }

    fn string(&mut self) -> Option<String> {
        self.consume(b'"')?;
        let mut bytes = Vec::new();
        loop {
            let byte = self.next()?;
            match byte {
                b'"' => return String::from_utf8(bytes).ok(),
                b'\\' => match self.next()? {
                    b'"' => bytes.push(b'"'),
                    b'\\' => bytes.push(b'\\'),
                    b'/' => bytes.push(b'/'),
                    b'b' => bytes.push(8),
                    b'f' => bytes.push(12),
                    b'n' => bytes.push(b'\n'),
                    b'r' => bytes.push(b'\r'),
                    b't' => bytes.push(b'\t'),
                    b'u' => {
                        let digits =
                            std::str::from_utf8(self.input.get(self.position..self.position + 4)?)
                                .ok()?;
                        self.position += 4;
                        let scalar = u16::from_str_radix(digits, 16).ok()?;
                        let ch = char::from_u32(u32::from(scalar))?;
                        let mut encoded = [0_u8; 4];
                        bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
                    }
                    _ => return None,
                },
                byte if byte < 0x20 => return None,
                byte => bytes.push(byte),
            }
        }
    }

    fn number(&mut self) -> Option<String> {
        let start = self.position;
        while self.peek().is_some_and(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E')
        }) {
            self.position += 1;
        }
        let value = std::str::from_utf8(&self.input[start..self.position]).ok()?;
        value.parse::<f64>().ok()?;
        Some(value.to_owned())
    }

    fn literal(&mut self, value: &[u8]) -> Option<()> {
        (self.input.get(self.position..self.position + value.len())? == value).then(|| {
            self.position += value.len();
        })
    }

    fn consume(&mut self, expected: u8) -> Option<()> {
        self.whitespace();
        (self.next()? == expected).then_some(())
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.position += 1;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_jc_json_objects_to_stable_table_fields() {
        let view =
            json_to_view(r#"[{"pid":2,"command":"worker","cpu_percent":91.5,"meta":{"ok":true}}]"#)
                .unwrap();
        let View::Table(table) = view else {
            panic!("expected table");
        };
        assert_eq!(table.columns[0].key, "pid");
        assert_eq!(table.columns[1].priority, 0);
        assert_eq!(table.rows[0][2].style, Style::Warning);
        assert_eq!(table.rows[0][3].text, "{\"ok\":true}");
    }
}
