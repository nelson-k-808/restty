use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Breakpoints;
use crate::model::{Alignment, Cell, ChangeKind, LogRecord, Style, Table, ValueType, View};

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub width: usize,
    pub color: bool,
    pub breakpoints: Breakpoints,
    pub border_style: BorderStyle,
    pub palette: ColorPalette,
    pub truecolor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPalette {
    Default,
    Colorblind,
}

impl ColorPalette {
    pub fn from_name(name: &str) -> Self {
        match name {
            "colorblind" | "colorblind-safe" => Self::Colorblind,
            _ => Self::Default,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    Rounded,
    Light,
    Heavy,
    Double,
    Ascii,
    None,
}

impl BorderStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "none" => Self::None,
            "light" | "sharp" => Self::Light,
            "heavy" => Self::Heavy,
            "double" => Self::Double,
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
            options,
        ));
        output.push('\n');
    }
    if table.columns.is_empty() || table.rows.is_empty() {
        return output;
    }

    let layout = table_layout(table, options);
    let primary = layout.selected.first().copied().unwrap_or(0);

    if options.width < options.breakpoints.compact && options.border_style == BorderStyle::None {
        for row in &table.rows {
            if let Some(cell) = row.get(primary) {
                let value = format_cell_value(&table.columns[primary], cell, options);
                output.push_str(&paint_typed_cell(
                    &truncate(&value, options.width),
                    cell,
                    &table.columns[primary],
                    options,
                ));
                output.push('\n');
            }
        }
        return output;
    }

    let selected = layout.selected;
    let widths = layout.widths;
    let padding = layout.padding;
    if options.border_style == BorderStyle::None || options.width < 5 {
        render_table_line(
            &mut output,
            &selected,
            &widths,
            padding,
            |index| Cell::styled(&table.columns[index].label, Style::Muted),
            table,
            options,
        );
        for row in &table.rows {
            render_table_line(
                &mut output,
                &selected,
                &widths,
                padding,
                |index| row[index].clone(),
                table,
                options,
            );
        }
    } else {
        render_bordered_table(&mut output, table, &selected, &widths, padding, options);
    }
    output
}

#[derive(Debug, Clone)]
pub(crate) struct TableLayout {
    pub selected: Vec<usize>,
    pub widths: Vec<usize>,
    pub padding: usize,
}

pub(crate) fn table_layout(table: &Table, options: &RenderOptions) -> TableLayout {
    if table.columns.is_empty() {
        return TableLayout {
            selected: Vec::new(),
            widths: Vec::new(),
            padding: 1,
        };
    }
    let primary = table
        .columns
        .iter()
        .enumerate()
        .min_by_key(|(index, column)| (column.priority, *index))
        .map(|(index, _)| index)
        .unwrap_or(0);
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
        selected.push(primary);
    }
    let padding = if options.border_style == BorderStyle::None {
        1
    } else {
        usize::from(options.width > options.breakpoints.wide) + 1
    };
    drop_columns_to_fit(
        table,
        &mut selected,
        padding,
        options.width,
        options.border_style,
    );
    let widths = fitted_widths(table, &selected, padding, options);
    TableLayout {
        selected,
        widths,
        padding,
    }
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
    options: &RenderOptions,
) -> Vec<usize> {
    let mut widths = selected
        .iter()
        .map(|index| {
            table
                .rows
                .iter()
                .filter_map(|row| row.get(*index))
                .map(|cell| {
                    display_width(&format_cell_value(&table.columns[*index], cell, options))
                })
                .chain(std::iter::once(display_width(&table.columns[*index].label)))
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect::<Vec<_>>();
    let overhead = if options.border_style == BorderStyle::None {
        selected.len().saturating_sub(1) * padding
    } else {
        selected.len() * (padding * 2 + 1) + 1
    };
    let available = options.width.saturating_sub(overhead).max(selected.len());
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
        BorderStyle::Light => BorderGlyphs {
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
        BorderStyle::Heavy => BorderGlyphs {
            top_left: '┏',
            top_join: '┳',
            top_right: '┓',
            middle_left: '┣',
            middle_join: '╋',
            middle_right: '┫',
            bottom_left: '┗',
            bottom_join: '┻',
            bottom_right: '┛',
            horizontal: '━',
            vertical: '┃',
        },
        BorderStyle::Double => BorderGlyphs {
            top_left: '╔',
            top_join: '╦',
            top_right: '╗',
            middle_left: '╠',
            middle_join: '╬',
            middle_right: '╣',
            bottom_left: '╚',
            bottom_join: '╩',
            bottom_right: '╝',
            horizontal: '═',
            vertical: '║',
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
        options,
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
        options,
    );
    render_border_rule(
        output,
        widths,
        padding,
        glyphs.middle_left,
        glyphs.middle_join,
        glyphs.middle_right,
        glyphs.horizontal,
        options,
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
            options,
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
        options,
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
    options: &RenderOptions,
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
    output.push_str(&paint(&line, Style::Muted, options));
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
    options: &RenderOptions,
) {
    output.push_str(&paint(&vertical.to_string(), Style::Muted, options));
    for (position, index) in selected.iter().enumerate() {
        output.push_str(&" ".repeat(padding));
        let cell = if header {
            Cell::styled(&table.columns[*index].label, Style::Accent)
        } else {
            row.and_then(|cells| cells.get(*index))
                .cloned()
                .unwrap_or_else(|| Cell::plain(""))
        };
        let value = format_cell_value(&table.columns[*index], &cell, options);
        let value = truncate(&value, widths[position]);
        let padded = pad(&value, widths[position], table.columns[*index].alignment);
        output.push_str(&paint_typed_cell(
            &padded,
            &cell,
            &table.columns[*index],
            options,
        ));
        output.push_str(&" ".repeat(padding));
        output.push_str(&paint(&vertical.to_string(), Style::Muted, options));
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
    options: &RenderOptions,
) where
    F: Fn(usize) -> Cell,
{
    for (position, index) in selected.iter().enumerate() {
        if position > 0 {
            output.push_str(&" ".repeat(padding));
        }
        let cell = cell_at(*index);
        let value = format_cell_value(&table.columns[*index], &cell, options);
        let value = truncate(&value, widths[position]);
        let padded = if position + 1 == selected.len()
            && table.columns[*index].alignment == Alignment::Left
        {
            value
        } else {
            pad(&value, widths[position], table.columns[*index].alignment)
        };
        output.push_str(&paint_typed_cell(
            &padded,
            &cell,
            &table.columns[*index],
            options,
        ));
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
                options,
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
                options,
            ));
            output.push(' ');
        }
        if level_width > 0 {
            let level = record.level.as_deref().unwrap_or("");
            output.push_str(&paint(
                &pad(&truncate(level, level_width), level_width, Alignment::Left),
                record.style,
                options,
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
            options,
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

fn paint(value: &str, style: Style, options: &RenderOptions) -> String {
    if !options.color || style == Style::Plain || value.trim().is_empty() {
        return value.to_owned();
    }
    let code = style_code(style, options);
    format!("\x1b[{code}m{value}\x1b[0m")
}

fn style_code(style: Style, options: &RenderOptions) -> &'static str {
    if options.truecolor {
        return match (options.palette, style) {
            (_, Style::Plain) => "0",
            (_, Style::Muted) => "2;38;2;148;163;184",
            (_, Style::Accent) => "1;38;2;34;211;238",
            (ColorPalette::Default, Style::Good) => "38;2;34;197;94",
            (ColorPalette::Default, Style::Warning) => "38;2;250;204;21",
            (ColorPalette::Default, Style::Error) => "1;38;2;248;113;113",
            (ColorPalette::Default, Style::Info) => "38;2;96;165;250",
            (ColorPalette::Default, Style::Debug) => "2;38;2;192;132;252",
            (ColorPalette::Colorblind, Style::Good) => "38;2;86;180;233",
            (ColorPalette::Colorblind, Style::Warning) => "38;2;240;228;66",
            (ColorPalette::Colorblind, Style::Error) => "1;38;2;204;121;167",
            (ColorPalette::Colorblind, Style::Info) => "38;2;0;158;115",
            (ColorPalette::Colorblind, Style::Debug) => "2;38;2;213;94;0",
        };
    }
    match (options.palette, style) {
        (_, Style::Plain) => "0",
        (_, Style::Muted) => "2;37",
        (_, Style::Accent) => "1;36",
        (ColorPalette::Default, Style::Good) => "32",
        (ColorPalette::Default, Style::Warning) => "33",
        (ColorPalette::Default, Style::Error) => "1;31",
        (ColorPalette::Default, Style::Info) => "34",
        (ColorPalette::Default, Style::Debug) => "2;35",
        (ColorPalette::Colorblind, Style::Good) => "34",
        (ColorPalette::Colorblind, Style::Warning) => "33",
        (ColorPalette::Colorblind, Style::Error) => "1;35",
        (ColorPalette::Colorblind, Style::Info) => "36",
        (ColorPalette::Colorblind, Style::Debug) => "2;31",
    }
}

fn paint_cell(value: &str, cell: &Cell, options: &RenderOptions) -> String {
    if !options.color || value.trim().is_empty() {
        return value.to_owned();
    }
    let change = match (options.palette, options.truecolor, cell.change) {
        (_, _, None) => None,
        (ColorPalette::Default, false, Some(ChangeKind::Added)) => Some("30;42"),
        (ColorPalette::Default, false, Some(ChangeKind::Removed)) => Some("30;41"),
        (ColorPalette::Default, false, Some(ChangeKind::Changed)) => Some("30;43"),
        (ColorPalette::Colorblind, false, Some(ChangeKind::Added)) => Some("37;44"),
        (ColorPalette::Colorblind, false, Some(ChangeKind::Removed)) => Some("37;45"),
        (ColorPalette::Colorblind, false, Some(ChangeKind::Changed)) => Some("30;43"),
        (ColorPalette::Default, true, Some(ChangeKind::Added)) => Some("30;48;2;34;197;94"),
        (ColorPalette::Default, true, Some(ChangeKind::Removed)) => Some("30;48;2;248;113;113"),
        (ColorPalette::Default, true, Some(ChangeKind::Changed)) => Some("30;48;2;250;204;21"),
        (ColorPalette::Colorblind, true, Some(ChangeKind::Added)) => Some("30;48;2;86;180;233"),
        (ColorPalette::Colorblind, true, Some(ChangeKind::Removed)) => Some("30;48;2;204;121;167"),
        (ColorPalette::Colorblind, true, Some(ChangeKind::Changed)) => Some("30;48;2;240;228;66"),
    };
    change
        .map(|code| format!("\x1b[{code}m{value}\x1b[0m"))
        .unwrap_or_else(|| paint(value, cell.style, options))
}

fn paint_typed_cell(
    value: &str,
    cell: &Cell,
    column: &crate::model::Column,
    options: &RenderOptions,
) -> String {
    if column.value_type != ValueType::Permissions || !options.color || cell.change.is_some() {
        return paint_cell(value, cell, options);
    }
    let mut output = String::new();
    for ch in value.chars() {
        let style = match ch {
            'd' | 'l' => Style::Accent,
            'r' => Style::Good,
            'w' => Style::Warning,
            'x' | 's' | 't' => Style::Info,
            '-' => Style::Muted,
            _ => Style::Plain,
        };
        output.push_str(&paint(&ch.to_string(), style, options));
    }
    output
}

pub(crate) fn format_cell_value(
    column: &crate::model::Column,
    cell: &Cell,
    options: &RenderOptions,
) -> String {
    match column.value_type {
        ValueType::Percentage if options.width >= options.breakpoints.core => {
            let Some(value) = cell.text.trim().trim_end_matches('%').parse::<f64>().ok() else {
                return cell.text.clone();
            };
            let mut rendered = format!("{} {}", cell.text, gauge_bar(value, 8));
            if options.width >= options.breakpoints.wide && !cell.trend.is_empty() {
                rendered.push(' ');
                rendered.push_str(&sparkline(&cell.trend));
            }
            rendered
        }
        ValueType::Timestamp => relative_timestamp(&cell.text, options),
        ValueType::Text | ValueType::Permissions | ValueType::Percentage => cell.text.clone(),
    }
}

fn gauge_bar(value: f64, width: usize) -> String {
    const RAMP: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let eighths = ((value.clamp(0.0, 100.0) / 100.0) * (width * 8) as f64).round() as usize;
    let full = (eighths / 8).min(width);
    let partial = eighths % 8;
    let mut output = "█".repeat(full);
    if full < width && partial > 0 {
        output.push(RAMP[partial - 1]);
    }
    let occupied = full + usize::from(full < width && partial > 0);
    output.push_str(&" ".repeat(width.saturating_sub(occupied)));
    output
}

fn sparkline(samples: &[u64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    samples
        .iter()
        .map(|sample| BARS[((sample.min(&100) * 7) / 100) as usize])
        .collect()
}

fn relative_timestamp(value: &str, options: &RenderOptions) -> String {
    let Ok(timestamp) = value.parse::<u64>() else {
        return value.to_owned();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now.saturating_sub(timestamp);
    let relative = if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        format!("{}d ago", seconds / 86_400)
    };
    if options.width > options.breakpoints.wide {
        format!("{relative} ({value})")
    } else {
        relative
    }
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
    fn gauges_use_fractional_block_cells() {
        assert_eq!(gauge_bar(0.0, 8), "        ");
        assert_eq!(gauge_bar(1.0, 8), "▏       ");
        assert_eq!(gauge_bar(47.0, 8), "███▊    ");
        assert_eq!(gauge_bar(100.0, 8), "████████");
    }

    #[test]
    fn all_named_border_styles_have_distinct_glyph_sets() {
        assert_eq!(border_glyphs(BorderStyle::Light).top_left, '┌');
        assert_eq!(border_glyphs(BorderStyle::Rounded).top_left, '╭');
        assert_eq!(border_glyphs(BorderStyle::Heavy).top_left, '┏');
        assert_eq!(border_glyphs(BorderStyle::Double).top_left, '╔');
        assert_eq!(border_glyphs(BorderStyle::Ascii).top_left, '+');
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
                    palette: ColorPalette::Default,
                    truecolor: false,
                },
            );
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "render exceeded {width} columns:\n{output}"
            );
        }
    }
}
