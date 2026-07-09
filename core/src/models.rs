#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub done: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub external_id: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub id: i64,
    pub task_id: i64,
    pub remind_at: i64,
    pub fired_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub start_at: i64,
    pub end_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub external_id: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Task,
    Event,
}

impl SyncKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncKind::Task => "task",
            SyncKind::Event => "event",
        }
    }
}
