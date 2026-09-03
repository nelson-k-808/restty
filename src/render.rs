use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Breakpoints;
use crate::model::{Alignment, Cell, LogRecord, Style, Table, View};

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub width: usize,
    pub color: bool,
    pub breakpoints: Breakpoints,
    pub border_style: BorderStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    Rounded,
    Sharp,
    Ascii,
    None,
}

impl BorderStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "none" => Self::None,
            "sharp" => Self::Sharp,
            "ascii" => Self::Ascii,
            _ => Self::Rounded,
        }
    }
}

pub fn render(view: &View, options: &RenderOptions) -> String {
    match view {
        View::Table(table) => render_table(table, options),
        View::Logs(records) => render_logs(records, options),
    }
}

fn render_table(table: &Table, options: &RenderOptions) -> String {
    let mut output = String::new();
    for line in &table.prelude {
        output.push_str(&paint(
            &truncate(line, options.width),
            Style::Muted,
            options.color,
        ));
        output.push('\n');
    }
    if table.columns.is_empty() || table.rows.is_empty() {
        return output;
    }

    let primary = table
        .columns
        .iter()
        .enumerate()
        .min_by_key(|(index, column)| (column.priority, *index))
        .map(|(index, _)| index)
        .unwrap_or(0);

    if options.width < options.breakpoints.compact && options.border_style == BorderStyle::None {
        for row in &table.rows {
            if let Some(cell) = row.get(primary) {
                output.push_str(&paint(
                    &truncate(&cell.text, options.width),
                    cell.style,
                    options.color,
                ));
                output.push('\n');
            }
        }
        return output;
    }

    let mut selected = if options.width < options.breakpoints.compact {
        vec![primary]
    } else if options.width < options.breakpoints.core {
        table
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| (column.priority <= 1).then_some(index))
            .collect::<Vec<_>>()
    } else {
        (0..table.columns.len()).collect::<Vec<_>>()
    };
    if selected.is_empty() {
        selected.push(0);
    }
    let padding = if options.border_style != BorderStyle::None {
        usize::from(options.width > options.breakpoints.wide) + 1
    } else if options.width > options.breakpoints.wide {
        3
    } else if options.width >= options.breakpoints.core {
        2
    } else {
        1
    };

    drop_columns_to_fit(
        table,
        &mut selected,
        padding,
        options.width,
        options.border_style,
    );
    let widths = fitted_widths(
        table,
        &selected,
        padding,
        options.width,
        options.border_style,
    );
    if options.border_style == BorderStyle::None || options.width < 5 {
        render_table_line(
            &mut output,
            &selected,
            &widths,
            padding,
            |index| Cell::styled(&table.columns[index].label, Style::Muted),
            table,
            options.color,
        );
        for row in &table.rows {
            render_table_line(
                &mut output,
                &selected,
                &widths,
                padding,
                |index| row[index].clone(),
                table,
                options.color,
            );
        }
    } else {
        render_bordered_table(&mut output, table, &selected, &widths, padding, options);
    }
    output
}

fn drop_columns_to_fit(
    table: &Table,
    selected: &mut Vec<usize>,
    padding: usize,
    width: usize,
    border_style: BorderStyle,
) {
    while selected.len() > 1 && minimum_total(selected.len(), padding, border_style) > width {
        let remove = selected
            .iter()
            .enumerate()
            .max_by_key(|(position, index)| (table.columns[**index].priority, *position))
            .map(|(position, _)| position)
            .unwrap_or(selected.len() - 1);
        selected.remove(remove);
    }
}

fn minimum_total(columns: usize, padding: usize, border_style: BorderStyle) -> usize {
    let cells = columns.saturating_mul(3);
    if border_style == BorderStyle::None {
        cells + columns.saturating_sub(1).saturating_mul(padding)
    } else {
        cells + columns.saturating_mul(padding.saturating_mul(2) + 1) + 1
    }
}

fn fitted_widths(
    table: &Table,
    selected: &[usize],
    padding: usize,
    width: usize,
    border_style: BorderStyle,
) -> Vec<usize> {
    let mut widths = selected
        .iter()
        .map(|index| {
            table
                .rows
                .iter()
                .filter_map(|row| row.get(*index))
                .map(|cell| display_width(&cell.text))
                .chain(std::iter::once(display_width(&table.columns[*index].label)))
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect::<Vec<_>>();
    let overhead = if border_style == BorderStyle::None {
        selected.len().saturating_sub(1) * padding
    } else {
        selected.len() * (padding * 2 + 1) + 1
    };
    let available = width.saturating_sub(overhead).max(selected.len());
    while widths.iter().sum::<usize>() > available {
        let Some((largest, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, value)| **value > 1)
            .max_by_key(|(_, value)| **value)
        else {
            break;
        };
        widths[largest] -= 1;
    }
    widths
}

#[derive(Clone, Copy)]
struct BorderGlyphs {
    top_left: char,
    top_join: char,
    top_right: char,
    middle_left: char,
    middle_join: char,
    middle_right: char,
    bottom_left: char,
    bottom_join: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

fn border_glyphs(style: BorderStyle) -> BorderGlyphs {
    match style {
        BorderStyle::Rounded => BorderGlyphs {
            top_left: '╭',
            top_join: '┬',
            top_right: '╮',
            middle_left: '├',
            middle_join: '┼',
            middle_right: '┤',
            bottom_left: '╰',
            bottom_join: '┴',
            bottom_right: '╯',
            horizontal: '─',
            vertical: '│',
        },
        BorderStyle::Sharp => BorderGlyphs {
            top_left: '┌',
            top_join: '┬',
            top_right: '┐',
            middle_left: '├',
            middle_join: '┼',
            middle_right: '┤',
            bottom_left: '└',
            bottom_join: '┴',
            bottom_right: '┘',
            horizontal: '─',
            vertical: '│',
        },
        BorderStyle::Ascii | BorderStyle::None => BorderGlyphs {
            top_left: '+',
            top_join: '+',
            top_right: '+',
            middle_left: '+',
            middle_join: '+',
            middle_right: '+',
            bottom_left: '+',
            bottom_join: '+',
            bottom_right: '+',
            horizontal: '-',
            vertical: '|',
        },
    }
}

fn render_bordered_table(
    output: &mut String,
    table: &Table,
    selected: &[usize],
    widths: &[usize],
    padding: usize,
    options: &RenderOptions,
) {
    let glyphs = border_glyphs(options.border_style);
    render_border_rule(
        output,
        widths,
        padding,
        glyphs.top_left,
        glyphs.top_join,
        glyphs.top_right,
        glyphs.horizontal,
        options.color,
    );
    render_bordered_row(
        output,
        table,
        selected,
        widths,
        padding,
        glyphs.vertical,
        true,
        None,
        options.color,
    );
    render_border_rule(
        output,
        widths,
        padding,
        glyphs.middle_left,
        glyphs.middle_join,
        glyphs.middle_right,
        glyphs.horizontal,
        options.color,
    );
    for row in &table.rows {
        render_bordered_row(
            output,
            table,
            selected,
            widths,
            padding,
            glyphs.vertical,
            false,
            Some(row),
            options.color,
        );
    }
    render_border_rule(
        output,
        widths,
        padding,
        glyphs.bottom_left,
        glyphs.bottom_join,
        glyphs.bottom_right,
        glyphs.horizontal,
        options.color,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_border_rule(
    output: &mut String,
    widths: &[usize],
    padding: usize,
    left: char,
    join: char,
    right: char,
    horizontal: char,
    color: bool,
) {
    let mut line = String::new();
    line.push(left);
    for (position, width) in widths.iter().enumerate() {
        if position > 0 {
            line.push(join);
        }
        line.extend(std::iter::repeat(horizontal).take(width + padding * 2));
    }
    line.push(right);
    output.push_str(&paint(&line, Style::Muted, color));
    output.push('\n');
}

#[allow(clippy::too_many_arguments)]
fn render_bordered_row(
    output: &mut String,
    table: &Table,
    selected: &[usize],
    widths: &[usize],
    padding: usize,
    vertical: char,
    header: bool,
    row: Option<&[Cell]>,
    color: bool,
) {
    output.push_str(&paint(&vertical.to_string(), Style::Muted, color));
    for (position, index) in selected.iter().enumerate() {
        output.push_str(&" ".repeat(padding));
        let cell = if header {
            Cell::styled(&table.columns[*index].label, Style::Accent)
        } else {
            row.and_then(|cells| cells.get(*index))
                .cloned()
                .unwrap_or_else(|| Cell::plain(""))
        };
        let value = truncate(&cell.text, widths[position]);
        let padded = pad(&value, widths[position], table.columns[*index].alignment);
        output.push_str(&paint(&padded, cell.style, color));
        output.push_str(&" ".repeat(padding));
        output.push_str(&paint(&vertical.to_string(), Style::Muted, color));
    }
    output.push('\n');
}

fn render_table_line<F>(
    output: &mut String,
    selected: &[usize],
    widths: &[usize],
    padding: usize,
    cell_at: F,
    table: &Table,
    color: bool,
) where
    F: Fn(usize) -> Cell,
{
    for (position, index) in selected.iter().enumerate() {
        if position > 0 {
            output.push_str(&" ".repeat(padding));
        }
        let cell = cell_at(*index);
        let value = truncate(&cell.text, widths[position]);
        let padded = if position + 1 == selected.len()
            && table.columns[*index].alignment == Alignment::Left
        {
            value
        } else {
            pad(&value, widths[position], table.columns[*index].alignment)
        };
        output.push_str(&paint(&padded, cell.style, color));
    }
    output.push('\n');
}

fn render_logs(records: &[LogRecord], options: &RenderOptions) -> String {
    let timestamp_width = records
        .iter()
        .filter_map(|record| record.timestamp.as_deref())
        .map(display_width)
        .max()
        .unwrap_or(0)
        .min(25);
    let level_width = records
        .iter()
        .filter_map(|record| record.level.as_deref())
        .map(display_width)
        .max()
        .unwrap_or(0)
        .min(8);
    let mut output = String::new();
    for record in records {
        if options.width < options.breakpoints.compact {
            let combined = [
                record.timestamp.as_deref(),
                record.level.as_deref(),
                Some(&record.message),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            output.push_str(&paint(
                &truncate(&combined, options.width),
                record.style,
                options.color,
            ));
            output.push('\n');
            continue;
        }
        let prefix_width = timestamp_width
            + level_width
            + usize::from(timestamp_width > 0)
            + usize::from(level_width > 0);
        if timestamp_width > 0 {
            let timestamp = record.timestamp.as_deref().unwrap_or("");
            output.push_str(&paint(
                &pad(
                    &truncate(timestamp, timestamp_width),
                    timestamp_width,
                    Alignment::Left,
                ),
                Style::Muted,
                options.color,
            ));
            output.push(' ');
        }
        if level_width > 0 {
            let level = record.level.as_deref().unwrap_or("");
            output.push_str(&paint(
                &pad(&truncate(level, level_width), level_width, Alignment::Left),
                record.style,
                options.color,
            ));
            output.push(' ');
        }
        output.push_str(&paint(
            &truncate(
                &record.message,
                options.width.saturating_sub(prefix_width).max(1),
            ),
            if record.level.as_deref() == Some("JSON") {
                Style::Info
            } else {
                Style::Plain
            },
            options.color,
        ));
        output.push('\n');
    }
    output
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn truncate(value: &str, max_width: usize) -> String {
    if display_width(value) <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".into();
    }
    let target = max_width - 1;
    let mut width = 0;
    let mut result = String::new();
    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > target {
            break;
        }
        result.push(ch);
        width += char_width;
    }
    result.push('…');
    result
}

fn pad(value: &str, width: usize, alignment: Alignment) -> String {
    let missing = width.saturating_sub(display_width(value));
    match alignment {
        Alignment::Left => format!("{value}{}", " ".repeat(missing)),
        Alignment::Right => format!("{}{value}", " ".repeat(missing)),
    }
}

fn paint(value: &str, style: Style, enabled: bool) -> String {
    if !enabled || style == Style::Plain || value.trim().is_empty() {
        return value.to_owned();
    }
    let code = match style {
        Style::Plain => return value.to_owned(),
        Style::Muted => "2;37",
        Style::Accent => "1;36",
        Style::Good => "32",
        Style::Warning => "33",
        Style::Error => "1;31",
        Style::Info => "34",
        Style::Debug => "2;35",
    };
    format!("\x1b[{code}m{value}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_by_display_cells() {
        assert_eq!(truncate("hello", 4), "hel…");
        assert_eq!(truncate("界面", 3), "界…");
        assert_eq!(display_width(&truncate("🦀 crab", 5)), 5);
    }

    #[test]
    fn bordered_tables_never_exceed_requested_width() {
        let table = Table {
            columns: vec![
                crate::model::Column::new("name", "NAME", 0, Alignment::Left),
                crate::model::Column::new("value", "VALUE", 1, Alignment::Right),
            ],
            rows: vec![vec![
                Cell::plain("a very long unicode 🦀 name"),
                Cell::plain("123456789"),
            ]],
            prelude: Vec::new(),
        };
        for width in 1..=200 {
            let output = render(
                &View::Table(table.clone()),
                &RenderOptions {
                    width,
                    color: false,
                    breakpoints: Breakpoints {
                        compact: 60,
                        core: 100,
                        wide: 160,
                    },
                    border_style: BorderStyle::Rounded,
                },
            );
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "render exceeded {width} columns:\n{output}"
            );
        }
    }
}
