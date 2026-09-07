use std::fmt::Write as _;

use crate::model::{Table, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredFormat {
    Json,
    Csv,
    Yaml,
}

impl StructuredFormat {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            "yaml" | "yml" => Some(Self::Yaml),
            _ => None,
        }
    }
}

pub fn serialize(view: &View, format: StructuredFormat) -> String {
    match format {
        StructuredFormat::Json => to_json(view),
        StructuredFormat::Csv => to_csv(view),
        StructuredFormat::Yaml => to_yaml(view),
    }
}

fn to_json(view: &View) -> String {
    let mut output = String::from("[\n");
    match view {
        View::Table(table) => {
            for (row_index, row) in table.rows.iter().enumerate() {
                output.push_str("  {");
                for (index, column) in table.columns.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    write_json_string(&mut output, &column.key);
                    output.push_str(": ");
                    write_json_string(
                        &mut output,
                        row.get(index).map(|cell| cell.text.as_str()).unwrap_or(""),
                    );
                }
                output.push('}');
                if row_index + 1 < table.rows.len() {
                    output.push(',');
                }
                output.push('\n');
            }
        }
        View::Logs(records) => {
            for (index, record) in records.iter().enumerate() {
                output.push_str("  {\"timestamp\": ");
                write_json_option(&mut output, record.timestamp.as_deref());
                output.push_str(", \"level\": ");
                write_json_option(&mut output, record.level.as_deref());
                output.push_str(", \"message\": ");
                write_json_string(&mut output, &record.message);
                output.push('}');
                if index + 1 < records.len() {
                    output.push(',');
                }
                output.push('\n');
            }
        }
    }
    output.push_str("]\n");
    output
}

fn to_csv(view: &View) -> String {
    let mut output = String::new();
    match view {
        View::Table(table) => {
            csv_row(
                &mut output,
                table.columns.iter().map(|column| column.key.as_str()),
            );
            for row in &table.rows {
                csv_row(
                    &mut output,
                    table.columns.iter().enumerate().map(|(index, _)| {
                        row.get(index).map(|cell| cell.text.as_str()).unwrap_or("")
                    }),
                );
            }
        }
        View::Logs(records) => {
            csv_row(&mut output, ["timestamp", "level", "message"]);
            for record in records {
                csv_row(
                    &mut output,
                    [
                        record.timestamp.as_deref().unwrap_or(""),
                        record.level.as_deref().unwrap_or(""),
                        &record.message,
                    ],
                );
            }
        }
    }
    output
}

fn csv_row<'a>(output: &mut String, values: impl IntoIterator<Item = &'a str>) {
    for (index, value) in values.into_iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        if value.contains([',', '"', '\n', '\r']) {
            output.push('"');
            output.push_str(&value.replace('"', "\"\""));
            output.push('"');
        } else {
            output.push_str(value);
        }
    }
    output.push('\n');
}

fn to_yaml(view: &View) -> String {
    let mut output = String::new();
    match view {
        View::Table(table) => write_yaml_table(&mut output, table),
        View::Logs(records) => {
            for record in records {
                output.push_str("- timestamp: ");
                write_yaml_option(&mut output, record.timestamp.as_deref());
                output.push_str("  level: ");
                write_yaml_option(&mut output, record.level.as_deref());
                output.push_str("  message: ");
                write_json_string(&mut output, &record.message);
                output.push('\n');
            }
        }
    }
    output
}

fn write_yaml_table(output: &mut String, table: &Table) {
    for row in &table.rows {
        for (index, column) in table.columns.iter().enumerate() {
            output.push_str(if index == 0 { "- " } else { "  " });
            write_json_string(output, &column.key);
            output.push_str(": ");
            write_json_string(
                output,
                row.get(index).map(|cell| cell.text.as_str()).unwrap_or(""),
            );
            output.push('\n');
        }
    }
}

fn write_json_option(output: &mut String, value: Option<&str>) {
    if let Some(value) = value {
        write_json_string(output, value);
    } else {
        output.push_str("null");
    }
}

fn write_yaml_option(output: &mut String, value: Option<&str>) {
    if let Some(value) = value {
        write_json_string(output, value);
        output.push('\n');
    } else {
        output.push_str("null\n");
    }
}

fn write_json_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            ch if ch < '\u{20}' => {
                let _ = write!(output, "\\u{:04x}", ch as u32);
            }
            ch => output.push(ch),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Alignment, Cell, Column, Table};

    fn view() -> View {
        View::Table(Table {
            columns: vec![
                Column::new("name", "NAME", 0, Alignment::Left),
                Column::new("value", "VALUE", 1, Alignment::Right),
            ],
            rows: vec![vec![Cell::plain("alpha"), Cell::plain("a,\"b")]],
            prelude: Vec::new(),
        })
    }

    #[test]
    fn serializes_stable_keys_in_all_formats() {
        assert_eq!(
            serialize(&view(), StructuredFormat::Json),
            "[\n  {\"name\": \"alpha\", \"value\": \"a,\\\"b\"}\n]\n"
        );
        assert_eq!(
            serialize(&view(), StructuredFormat::Csv),
            "name,value\nalpha,\"a,\"\"b\"\n"
        );
        assert_eq!(
            serialize(&view(), StructuredFormat::Yaml),
            "- \"name\": \"alpha\"\n  \"value\": \"a,\\\"b\"\n"
        );
    }
}
