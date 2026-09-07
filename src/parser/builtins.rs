use crate::model::{Cell, Column, Style, ValueType, View};

use super::util::{
    aligned_starts, col, is_month, is_permissions, num_col, split_at_starts,
    split_whitespace_limit, table,
};
use super::Parser;

pub fn all() -> Vec<Box<dyn Parser>> {
    vec![
        Box::new(LsParser),
        Box::new(PsParser),
        Box::new(DfParser),
        Box::new(DuParser),
        Box::new(GitStatusParser),
        Box::new(GitLogParser),
        Box::new(DockerPsParser),
        Box::new(LsblkParser),
        Box::new(FreeParser),
        Box::new(IpRouteParser),
    ]
}

struct LsParser;

impl Parser for LsParser {
    fn name(&self) -> &'static str {
        "ls"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "ls"
            && first_lines
                .lines()
                .any(|line| line.split_whitespace().next().is_some_and(is_permissions))
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut rows = Vec::new();
        let mut prelude = Vec::new();
        for line in input.lines().filter(|line| !line.trim().is_empty()) {
            if line.trim_start().starts_with("total ") {
                prelude.push(line.trim().to_owned());
                continue;
            }
            let tokens = line.split_whitespace().collect::<Vec<_>>();
            if tokens.len() < 9 || !is_permissions(tokens[0]) {
                return None;
            }
            let month = tokens.iter().position(|token| is_month(token))?;
            if month < 5 || month + 3 >= tokens.len() {
                return None;
            }
            let modified = tokens[month..month + 3].join(" ");
            let name = tokens[month + 3..].join(" ");
            let name_style = if tokens[0].starts_with('d') {
                Style::Accent
            } else if tokens[0].starts_with('l') {
                Style::Info
            } else {
                Style::Plain
            };
            rows.push(vec![
                Cell::styled(tokens[0], Style::Muted),
                Cell::plain(tokens[1]),
                Cell::plain(tokens[2]),
                Cell::plain(tokens[3]),
                Cell::styled(tokens[month - 1], Style::Warning),
                Cell::plain(modified),
                Cell::styled(name, name_style),
            ]);
        }
        let mut parsed = table(
            vec![
                col("mode", "MODE", 2).with_value_type(ValueType::Permissions),
                num_col("links", "LINKS", 3),
                col("owner", "OWNER", 2),
                col("group", "GROUP", 3),
                num_col("size", "SIZE", 1),
                col("modified", "MODIFIED", 1).with_value_type(ValueType::Timestamp),
                col("name", "NAME", 0),
            ],
            rows,
        )?;
        parsed.prelude = prelude;
        Some(View::Table(parsed))
    }
}

struct PsParser;

impl Parser for PsParser {
    fn name(&self) -> &'static str {
        "ps"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        if source != "ps" {
            return false;
        }
        let header = first_lines.lines().next().unwrap_or_default();
        header.contains("PID") && (header.contains("COMMAND") || header.contains("CMD"))
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut lines = input.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next()?.split_whitespace().collect::<Vec<_>>();
        if header.len() < 3 {
            return None;
        }
        let mut rows = Vec::new();
        for line in lines {
            let fields = split_whitespace_limit(line, header.len());
            if fields.len() != header.len() {
                return None;
            }
            rows.push(
                fields
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let label = header[index].to_ascii_uppercase();
                        let style = if label == "PID" {
                            Style::Accent
                        } else if label == "%CPU" || label == "%MEM" {
                            Style::Warning
                        } else {
                            Style::Plain
                        };
                        Cell::styled(value, style)
                    })
                    .collect(),
            );
        }
        let columns = header
            .iter()
            .map(|label| {
                let upper = label.to_ascii_uppercase();
                let priority = match upper.as_str() {
                    "COMMAND" | "CMD" => 0,
                    "PID" | "USER" | "%CPU" | "%MEM" => 1,
                    "STAT" | "START" | "STARTED" | "TIME" => 2,
                    _ => 3,
                };
                let column = if matches!(upper.as_str(), "PID" | "%CPU" | "%MEM" | "VSZ" | "RSS") {
                    num_col(&upper.to_ascii_lowercase(), &upper, priority)
                } else {
                    col(&upper.to_ascii_lowercase(), &upper, priority)
                };
                if matches!(upper.as_str(), "%CPU" | "%MEM") {
                    column.with_value_type(ValueType::Percentage)
                } else {
                    column
                }
            })
            .collect();
        Some(View::Table(table(columns, rows)?))
    }
}

struct DfParser;

impl Parser for DfParser {
    fn name(&self) -> &'static str {
        "df"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "df"
            && first_lines.lines().next().is_some_and(|line| {
                line.to_ascii_lowercase().contains("filesystem")
                    && (line.contains("Mounted") || line.contains("Capacity"))
            })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut lines = input.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next()?;
        let raw_headers = header.split_whitespace().collect::<Vec<_>>();
        let mounted_index = raw_headers
            .iter()
            .position(|label| label.eq_ignore_ascii_case("mounted"))?;
        if mounted_index < 4 {
            return None;
        }
        let mut headers = raw_headers[..mounted_index]
            .iter()
            .map(|label| (*label).to_owned())
            .collect::<Vec<_>>();
        headers.push(raw_headers[mounted_index..].join(" "));
        let mut rows = Vec::new();
        for line in lines {
            let fields = split_whitespace_limit(line, headers.len());
            if fields.len() != headers.len() {
                return None;
            }
            rows.push(
                fields
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let lower = headers[index].to_ascii_lowercase();
                        let style = if index + 1 == headers.len() {
                            Style::Accent
                        } else if lower.contains("use%")
                            || lower.contains("capacity")
                            || lower.contains("%iused")
                        {
                            match value.trim_end_matches('%').parse::<u8>().unwrap_or(0) {
                                90.. => Style::Error,
                                75..=89 => Style::Warning,
                                _ => Style::Good,
                            }
                        } else {
                            Style::Plain
                        };
                        Cell::styled(value, style)
                    })
                    .collect(),
            );
        }
        let columns = headers
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let lower = label.to_ascii_lowercase();
                let key: String = match lower.as_str() {
                    "filesystem" => "filesystem".into(),
                    "avail" | "available" => "available".into(),
                    "use%" => "use".into(),
                    "mounted on" | "mounted" => "mounted".into(),
                    _ => lower
                        .trim_start_matches('%')
                        .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
                };
                let display: String = match lower.as_str() {
                    "filesystem" => "FILESYSTEM".into(),
                    "used" => "USED".into(),
                    "avail" | "available" => "AVAILABLE".into(),
                    "mounted on" | "mounted" => "MOUNTED ON".into(),
                    _ => label.clone(),
                };
                let priority = if index + 1 == headers.len() {
                    0
                } else if lower == "filesystem"
                    || lower.contains("use%")
                    || lower.contains("capacity")
                {
                    1
                } else if matches!(lower.as_str(), "iused" | "ifree" | "%iused") {
                    3
                } else {
                    2
                };
                let column = Column::new(
                    &key,
                    &display,
                    priority,
                    if index == 0 || index + 1 == headers.len() {
                        crate::model::Alignment::Left
                    } else {
                        crate::model::Alignment::Right
                    },
                );
                if lower.contains("use%") || lower.contains("capacity") || lower.contains("%iused")
                {
                    column.with_value_type(ValueType::Percentage)
                } else {
                    column
                }
            })
            .collect();
        Some(View::Table(table(columns, rows)?))
    }
}

struct DuParser;

impl Parser for DuParser {
    fn name(&self) -> &'static str {
        "du"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "du"
            && first_lines
                .lines()
                .filter(|line| !line.trim().is_empty())
                .all(|line| {
                    let mut fields = line.split_whitespace();
                    fields
                        .next()
                        .is_some_and(|size| size.chars().any(|ch| ch.is_ascii_digit()))
                        && fields.next().is_some()
                })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let rows = input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let split = line.find(char::is_whitespace)?;
                let size = &line[..split];
                let path = line[split..].trim();
                (!path.is_empty()).then(|| {
                    vec![
                        Cell::styled(size, Style::Warning),
                        Cell::styled(path, Style::Accent),
                    ]
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(View::Table(table(
            vec![num_col("size", "SIZE", 1), col("path", "PATH", 0)],
            rows,
        )?))
    }
}

struct GitStatusParser;

impl Parser for GitStatusParser {
    fn name(&self) -> &'static str {
        "git-status"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "git"
            && first_lines
                .lines()
                .filter(|line| !line.is_empty())
                .all(|line| {
                    line.len() >= 4
                        && line.as_bytes().get(2).is_some_and(u8::is_ascii_whitespace)
                        && line
                            .get(..2)
                            .is_some_and(|status| status.chars().all(|ch| " MADRCU?!".contains(ch)))
                })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let rows = input
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let status = line.get(..2)?;
                let path = line.get(3..)?.trim();
                let style = if status.contains('?') || status.contains('A') {
                    Style::Good
                } else if status.contains('D') || status.contains('U') {
                    Style::Error
                } else {
                    Style::Warning
                };
                Some(vec![Cell::styled(status, style), Cell::plain(path)])
            })
            .collect::<Option<Vec<_>>>()?;
        Some(View::Table(table(
            vec![col("status", "STATUS", 1), col("path", "PATH", 0)],
            rows,
        )?))
    }
}

struct GitLogParser;

impl Parser for GitLogParser {
    fn name(&self) -> &'static str {
        "git-log"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "git"
            && first_lines
                .lines()
                .filter(|line| !line.trim().is_empty())
                .all(|line| {
                    line.split_once(char::is_whitespace)
                        .is_some_and(|(hash, _)| {
                            (7..=64).contains(&hash.len())
                                && hash.chars().all(|ch| ch.is_ascii_hexdigit())
                        })
                })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let rows = input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (hash, subject) = line.split_once(char::is_whitespace)?;
                Some(vec![
                    Cell::styled(hash, Style::Accent),
                    Cell::plain(subject.trim()),
                ])
            })
            .collect::<Option<Vec<_>>>()?;
        Some(View::Table(table(
            vec![col("commit", "COMMIT", 1), col("subject", "SUBJECT", 0)],
            rows,
        )?))
    }
}

struct DockerPsParser;

impl Parser for DockerPsParser {
    fn name(&self) -> &'static str {
        "docker-ps"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "docker"
            && first_lines.lines().next().is_some_and(|header| {
                header.contains("CONTAINER ID")
                    && header.contains("IMAGE")
                    && header.contains("STATUS")
            })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut lines = input.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next()?;
        let starts = aligned_starts(header);
        let headers = split_at_starts(header, &starts);
        if headers.len() < 5 {
            return None;
        }
        let mut rows = Vec::new();
        for line in lines {
            let values = split_at_starts(line, &starts);
            if values.len() != headers.len() {
                return None;
            }
            rows.push(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let label = headers[index].as_str();
                        let style = match label {
                            "CONTAINER ID" => Style::Accent,
                            "STATUS" if value.to_ascii_lowercase().starts_with("up") => Style::Good,
                            "STATUS" => Style::Warning,
                            _ => Style::Plain,
                        };
                        Cell::styled(value, style)
                    })
                    .collect(),
            );
        }
        let columns = headers
            .iter()
            .map(|label| {
                let key = label.to_ascii_lowercase().replace(' ', "_");
                let priority = match label.as_str() {
                    "NAMES" => 0,
                    "CONTAINER ID" | "IMAGE" | "STATUS" => 1,
                    "PORTS" => 2,
                    _ => 3,
                };
                Column::new(&key, label, priority, crate::model::Alignment::Left)
            })
            .collect();
        Some(View::Table(table(columns, rows)?))
    }
}

struct LsblkParser;

impl Parser for LsblkParser {
    fn name(&self) -> &'static str {
        "lsblk"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "lsblk"
            && first_lines.lines().next().is_some_and(|header| {
                header.split_whitespace().any(|label| label == "NAME")
                    && header.split_whitespace().any(|label| label == "TYPE")
            })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut lines = input.lines().filter(|line| !line.trim().is_empty());
        let headers = lines
            .next()?
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if headers.len() < 2 {
            return None;
        }
        let mut rows = Vec::new();
        for line in lines {
            let mut fields = split_whitespace_limit(line, headers.len());
            if fields.is_empty() {
                continue;
            }
            if fields.len() == 1 && !rows.is_empty() {
                let continuation = fields.pop().unwrap_or_default();
                fields.resize(headers.len(), String::new());
                if let Some(last) = fields.last_mut() {
                    *last = continuation;
                }
            } else {
                fields.resize(headers.len(), String::new());
            }
            rows.push(
                fields
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        Cell::styled(
                            value,
                            match headers[index].as_str() {
                                "NAME" => Style::Accent,
                                "MOUNTPOINT" | "MOUNTPOINTS" => Style::Good,
                                "SIZE" => Style::Warning,
                                _ => Style::Plain,
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let columns = headers
            .iter()
            .map(|label| {
                let priority = match label.as_str() {
                    "NAME" => 0,
                    "SIZE" | "TYPE" | "MOUNTPOINT" | "MOUNTPOINTS" => 1,
                    "RM" | "RO" => 2,
                    _ => 3,
                };
                Column::new(
                    &label.to_ascii_lowercase().replace(':', "_"),
                    label,
                    priority,
                    if matches!(label.as_str(), "RM" | "RO") {
                        crate::model::Alignment::Right
                    } else {
                        crate::model::Alignment::Left
                    },
                )
            })
            .collect();
        Some(View::Table(table(columns, rows)?))
    }
}

struct FreeParser;

impl Parser for FreeParser {
    fn name(&self) -> &'static str {
        "free"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "free"
            && first_lines.lines().any(|line| {
                let lower = line.to_ascii_lowercase();
                lower.contains("total") && lower.contains("used") && lower.contains("available")
            })
            && first_lines
                .lines()
                .any(|line| line.trim_start().starts_with("Mem:"))
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut lines = input.lines().filter(|line| !line.trim().is_empty());
        let headers = lines
            .next()?
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if headers.len() < 3 {
            return None;
        }
        let mut rows = Vec::new();
        for line in lines {
            let mut fields = split_whitespace_limit(line, headers.len() + 1);
            if fields.len() < 2 {
                return None;
            }
            fields.resize(headers.len() + 1, String::new());
            rows.push(
                fields
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let style = if index == 0 {
                            Style::Accent
                        } else {
                            match headers.get(index - 1).map(String::as_str) {
                                Some("used") => Style::Warning,
                                Some("free" | "available") => Style::Good,
                                _ => Style::Plain,
                            }
                        };
                        Cell::styled(value.trim_end_matches(':'), style)
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let mut columns = vec![col("type", "TYPE", 0)];
        columns.extend(headers.iter().map(|label| {
            let priority = match label.as_str() {
                "total" | "used" | "available" => 1,
                _ => 2,
            };
            num_col(label, &label.to_ascii_uppercase(), priority)
        }));
        Some(View::Table(table(columns, rows)?))
    }
}

struct IpRouteParser;

impl Parser for IpRouteParser {
    fn name(&self) -> &'static str {
        "ip-route"
    }

    fn detect(&self, source: &str, first_lines: &str) -> bool {
        source == "ip"
            && first_lines
                .lines()
                .filter(|line| !line.trim().is_empty())
                .any(|line| {
                    let words = line.split_whitespace().collect::<Vec<_>>();
                    words.contains(&"dev")
                        && (words.contains(&"via")
                            || words.contains(&"scope")
                            || words.first() == Some(&"default"))
                })
    }

    fn parse(&self, input: &str) -> Option<View> {
        let mut rows = Vec::new();
        for line in input.lines().filter(|line| !line.trim().is_empty()) {
            let words = line.split_whitespace().collect::<Vec<_>>();
            if words.is_empty() {
                continue;
            }
            let mut position = 1;
            let mut destination = words[0].to_owned();
            if matches!(
                words[0],
                "local" | "broadcast" | "unreachable" | "blackhole" | "prohibit" | "throw"
            ) && words.len() > 1
            {
                destination.push(' ');
                destination.push_str(words[1]);
                position = 2;
            }
            let mut via = String::new();
            let mut device = String::new();
            let mut protocol = String::new();
            let mut scope = String::new();
            let mut source = String::new();
            let mut metric = String::new();
            let mut details = Vec::new();
            while position < words.len() {
                let target = match words[position] {
                    "via" => Some(&mut via),
                    "dev" => Some(&mut device),
                    "proto" => Some(&mut protocol),
                    "scope" => Some(&mut scope),
                    "src" => Some(&mut source),
                    "metric" => Some(&mut metric),
                    _ => None,
                };
                if let Some(target) = target {
                    if let Some(value) = words.get(position + 1) {
                        *target = (*value).to_owned();
                        position += 2;
                        continue;
                    }
                }
                details.push(words[position]);
                position += 1;
            }
            rows.push(vec![
                Cell::styled(destination, Style::Accent),
                Cell::styled(via, Style::Info),
                Cell::styled(device, Style::Good),
                Cell::plain(protocol),
                Cell::plain(scope),
                Cell::plain(source),
                Cell::plain(metric),
                Cell::styled(details.join(" "), Style::Muted),
            ]);
        }
        Some(View::Table(table(
            vec![
                col("destination", "DESTINATION", 0),
                col("gateway", "GATEWAY", 1),
                col("device", "DEVICE", 1),
                col("protocol", "PROTOCOL", 2),
                col("scope", "SCOPE", 2),
                col("source", "SOURCE", 2),
                num_col("metric", "METRIC", 3),
                col("details", "DETAILS", 3),
            ],
            rows,
        )?))
    }
}
