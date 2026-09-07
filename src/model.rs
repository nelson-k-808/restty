#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Text,
    Percentage,
    Timestamp,
    Permissions,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub key: String,
    pub label: String,
    pub priority: u8,
    pub alignment: Alignment,
    pub value_type: ValueType,
}

impl Column {
    pub fn new(key: &str, label: &str, priority: u8, alignment: Alignment) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            priority,
            alignment,
            value_type: ValueType::Text,
        }
    }

    pub fn with_value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = value_type;
        self
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    pub style: Style,
    pub change: Option<ChangeKind>,
    pub trend: Vec<u64>,
}

impl Cell {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::Plain,
            change: None,
            trend: Vec::new(),
        }
    }

    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            change: None,
            trend: Vec::new(),
        }
    }

    pub fn with_change(mut self, change: ChangeKind) -> Self {
        self.change = Some(change);
        self
    }

    pub fn with_trend(mut self, trend: Vec<u64>) -> Self {
        self.trend = trend;
        self
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
