#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Muted,
    Accent,
    Good,
    Warning,
    Error,
    Info,
    Debug,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub key: String,
    pub label: String,
    pub priority: u8,
    pub alignment: Alignment,
}

impl Column {
    pub fn new(key: &str, label: &str, priority: u8, alignment: Alignment) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            priority,
            alignment,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    pub style: Style,
}

impl Cell {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::Plain,
        }
    }

    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    pub prelude: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LogRecord {
    pub timestamp: Option<String>,
    pub level: Option<String>,
    pub message: String,
    pub style: Style,
}

#[derive(Debug, Clone)]
pub enum View {
    Table(Table),
    Logs(Vec<LogRecord>),
}

#[derive(Debug, Clone)]
pub struct ParsedView {
    pub view: View,
    pub confidence: f32,
}
