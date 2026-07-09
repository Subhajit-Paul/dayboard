use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{Event, Reminder, SyncKind, Task};

pub struct Db {
    conn: Connection,
}

fn data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("caldav");
    }
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".local/share/caldav")
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

fn task_from_row(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        done: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        external_id: row.get(7)?,
        etag: row.get(8)?,
    })
}

const TASK_COLUMNS: &str =
    "id, parent_id, title, description, done, created_at, updated_at, external_id, etag";

fn event_from_row(row: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        start_at: row.get(3)?,
        end_at: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        external_id: row.get(7)?,
        etag: row.get(8)?,
    })
}

const EVENT_COLUMNS: &str =
    "id, title, description, start_at, end_at, created_at, updated_at, external_id, etag";

impl Db {
    /// Opens the default database at `$XDG_DATA_HOME/caldav/caldav.db` (or
    /// `~/.local/share/caldav/caldav.db`), creating the directory and schema
    /// if needed.
    pub fn open_default() -> rusqlite::Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).expect("failed to create data dir");
        Self::open(dir.join("caldav.db"))
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Db { conn })
    }

    // --- tasks ---

    pub fn create_task(&self, title: &str, parent_id: Option<i64>) -> rusqlite::Result<i64> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO tasks (parent_id, title, done, created_at, updated_at) \
             VALUES (?1, ?2, 0, ?3, ?3)",
            params![parent_id, title, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_tasks(&self) -> rusqlite::Result<Vec<Task>> {
        // Flat, id-ordered; callers that need a tree (e.g. the TUI) group by
        // parent_id themselves rather than pushing that into SQL.
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks ORDER BY id"))?;
        let rows = stmt.query_map([], task_from_row)?;
        rows.collect()
    }

    pub fn get_task(&self, id: i64) -> rusqlite::Result<Option<Task>> {
        self.conn
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
                params![id],
                task_from_row,
            )
            .optional()
    }

    pub fn find_task_by_external_id(&self, external_id: &str) -> rusqlite::Result<Option<Task>> {
        self.conn
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE external_id = ?1"),
                params![external_id],
                task_from_row,
            )
            .optional()
    }

    pub fn update_task_title(&self, id: i64, title: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now(), id],
        )?;
        Ok(())
    }

    pub fn toggle_done(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET done = NOT done, updated_at = ?1 WHERE id = ?2",
            params![now(), id],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: i64) -> rusqlite::Result<()> {
        if let Some(task) = self.get_task(id)?
            && let Some(external_id) = task.external_id
        {
            self.add_tombstone(&external_id, SyncKind::Task)?;
        }
        // ON DELETE CASCADE removes subtasks and reminders. Subtasks that
        // had their own external_id are tombstoned too so their deletion
        // also propagates.
        for child in self.list_tasks()?.into_iter().filter(|t| t.parent_id == Some(id)) {
            if let Some(external_id) = child.external_id {
                self.add_tombstone(&external_id, SyncKind::Task)?;
            }
        }
        self.conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn has_children(&self, id: i64) -> rusqlite::Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE parent_id = ?1)",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n != 0)
    }

    /// Inserts a task that originated on the remote (Google Tasks) side,
    /// preserving the remote id/etag/timestamp instead of generating fresh
    /// local ones.
    #[allow(clippy::too_many_arguments)] // mirrors the tasks table columns 1:1
    pub fn insert_task_from_remote(
        &self,
        title: &str,
        description: Option<&str>,
        done: bool,
        parent_id: Option<i64>,
        external_id: &str,
        etag: &str,
        updated_at: i64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO tasks (parent_id, title, description, done, created_at, updated_at, external_id, etag) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
            params![parent_id, title, description, done as i64, updated_at, external_id, etag],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Overwrites a local task's content from the remote side (remote won
    /// the last-write-wins comparison). Does not touch `id`/`external_id`/`created_at`.
    #[allow(clippy::too_many_arguments)] // mirrors the tasks table columns 1:1
    pub fn apply_remote_task(
        &self,
        id: i64,
        title: &str,
        description: Option<&str>,
        done: bool,
        parent_id: Option<i64>,
        etag: &str,
        updated_at: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET title = ?1, description = ?2, done = ?3, parent_id = ?4, \
             etag = ?5, updated_at = ?6 WHERE id = ?7",
            params![title, description, done as i64, parent_id, etag, updated_at, id],
        )?;
        Ok(())
    }

    /// Attaches the id/etag Google assigned after a local task was pushed
    /// for the first time. Not a content change, so `updated_at` is untouched.
    pub fn set_task_external(&self, id: i64, external_id: &str, etag: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET external_id = ?1, etag = ?2 WHERE id = ?3",
            params![external_id, etag, id],
        )?;
        Ok(())
    }

    /// Refreshes just the etag after a push that updated an already-linked task.
    pub fn set_task_etag(&self, id: i64, etag: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE tasks SET etag = ?1 WHERE id = ?2", params![etag, id])?;
        Ok(())
    }

    // --- events ---

    pub fn create_event(&self, title: &str, start_at: i64, end_at: i64) -> rusqlite::Result<i64> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO events (title, start_at, end_at, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![title, start_at, end_at, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_events(&self) -> rusqlite::Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {EVENT_COLUMNS} FROM events ORDER BY start_at"))?;
        let rows = stmt.query_map([], event_from_row)?;
        rows.collect()
    }

    pub fn get_event(&self, id: i64) -> rusqlite::Result<Option<Event>> {
        self.conn
            .query_row(
                &format!("SELECT {EVENT_COLUMNS} FROM events WHERE id = ?1"),
                params![id],
                event_from_row,
            )
            .optional()
    }

    pub fn find_event_by_external_id(&self, external_id: &str) -> rusqlite::Result<Option<Event>> {
        self.conn
            .query_row(
                &format!("SELECT {EVENT_COLUMNS} FROM events WHERE external_id = ?1"),
                params![external_id],
                event_from_row,
            )
            .optional()
    }

    pub fn delete_event(&self, id: i64) -> rusqlite::Result<()> {
        if let Some(event) = self.get_event(id)?
            && let Some(external_id) = event.external_id
        {
            self.add_tombstone(&external_id, SyncKind::Event)?;
        }
        self.conn.execute("DELETE FROM events WHERE id = ?1", params![id])?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // mirrors the events table columns 1:1
    pub fn insert_event_from_remote(
        &self,
        title: &str,
        description: Option<&str>,
        start_at: i64,
        end_at: i64,
        external_id: &str,
        etag: &str,
        created_at: i64,
        updated_at: i64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO events (title, description, start_at, end_at, created_at, updated_at, external_id, etag) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![title, description, start_at, end_at, created_at, updated_at, external_id, etag],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)] // mirrors the events table columns 1:1
    pub fn apply_remote_event(
        &self,
        id: i64,
        title: &str,
        description: Option<&str>,
        start_at: i64,
        end_at: i64,
        etag: &str,
        updated_at: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE events SET title = ?1, description = ?2, start_at = ?3, end_at = ?4, \
             etag = ?5, updated_at = ?6 WHERE id = ?7",
            params![title, description, start_at, end_at, etag, updated_at, id],
        )?;
        Ok(())
    }

    pub fn set_event_external(&self, id: i64, external_id: &str, etag: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE events SET external_id = ?1, etag = ?2 WHERE id = ?3",
            params![external_id, etag, id],
        )?;
        Ok(())
    }

    pub fn set_event_etag(&self, id: i64, etag: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE events SET etag = ?1 WHERE id = ?2", params![etag, id])?;
        Ok(())
    }

    // --- sync tombstones ---

    pub fn add_tombstone(&self, external_id: &str, kind: SyncKind) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sync_tombstones (external_id, kind) VALUES (?1, ?2)",
            params![external_id, kind.as_str()],
        )?;
        Ok(())
    }

    pub fn list_tombstones(&self, kind: SyncKind) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT external_id FROM sync_tombstones WHERE kind = ?1")?;
        let rows = stmt.query_map(params![kind.as_str()], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn remove_tombstone(&self, external_id: &str, kind: SyncKind) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM sync_tombstones WHERE external_id = ?1 AND kind = ?2",
            params![external_id, kind.as_str()],
        )?;
        Ok(())
    }

    // --- reminders ---

    pub fn create_reminder(&self, task_id: i64, remind_at: i64) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO reminders (task_id, remind_at, fired_at) VALUES (?1, ?2, NULL)",
            params![task_id, remind_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// task_id -> count of reminders, for annotating the task list without
    /// an N+1 query per task.
    pub fn reminder_counts(&self) -> rusqlite::Result<std::collections::HashMap<i64, i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT task_id, COUNT(*) FROM reminders GROUP BY task_id")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        rows.collect()
    }

    pub fn list_reminders_for_task(&self, task_id: i64) -> rusqlite::Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, remind_at, fired_at FROM reminders WHERE task_id = ?1 ORDER BY remind_at",
        )?;
        let rows = stmt.query_map(params![task_id], |row| {
            Ok(Reminder {
                id: row.get(0)?,
                task_id: row.get(1)?,
                remind_at: row.get(2)?,
                fired_at: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Reminders due at or before `at` that haven't fired yet, joined with
    /// their task title so the daemon doesn't need a second query.
    pub fn list_due_reminders(&self, at: i64) -> rusqlite::Result<Vec<(Reminder, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.task_id, r.remind_at, r.fired_at, t.title \
             FROM reminders r JOIN tasks t ON t.id = r.task_id \
             WHERE r.fired_at IS NULL AND r.remind_at <= ?1",
        )?;
        let rows = stmt.query_map(params![at], |row| {
            Ok((
                Reminder {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    remind_at: row.get(2)?,
                    fired_at: row.get(3)?,
                },
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.collect()
    }

    pub fn mark_reminder_fired(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE reminders SET fired_at = ?1 WHERE id = ?2",
            params![now(), id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_subtasks_reminders_roundtrip() {
        let db = Db::open(":memory:").unwrap();

        let parent = db.create_task("Buy milk", None).unwrap();
        let child = db.create_task("Buy oat milk", Some(parent)).unwrap();

        let tasks = db.list_tasks().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks.iter().find(|t| t.id == child).unwrap().parent_id, Some(parent));
        assert!(!tasks.iter().find(|t| t.id == parent).unwrap().done);

        db.toggle_done(parent).unwrap();
        assert!(db.list_tasks().unwrap().iter().find(|t| t.id == parent).unwrap().done);

        assert!(db.has_children(parent).unwrap());
        assert!(!db.has_children(child).unwrap());

        let past = now() - 10;
        let future = now() + 10_000;
        let due_reminder = db.create_reminder(child, past).unwrap();
        db.create_reminder(child, future).unwrap();

        let due = db.list_due_reminders(now()).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0.id, due_reminder);
        assert_eq!(due[0].1, "Buy oat milk");

        db.mark_reminder_fired(due_reminder).unwrap();
        assert!(db.list_due_reminders(now()).unwrap().is_empty());

        // Deleting the parent cascades to its subtask and the subtask's reminders.
        db.delete_task(parent).unwrap();
        assert!(db.list_tasks().unwrap().is_empty());
        assert!(db.list_reminders_for_task(child).unwrap().is_empty());
    }

    #[test]
    fn delete_of_synced_task_leaves_a_tombstone() {
        let db = Db::open(":memory:").unwrap();
        let id = db.create_task("Sync me", None).unwrap();
        db.set_task_external(id, "google-123", "etag-1").unwrap();

        db.delete_task(id).unwrap();

        assert_eq!(db.list_tombstones(SyncKind::Task).unwrap(), vec!["google-123".to_string()]);
    }

    #[test]
    fn events_roundtrip_and_remote_insert() {
        let db = Db::open(":memory:").unwrap();
        let local = db.create_event("Standup", 1000, 1900).unwrap();
        let remote = db
            .insert_event_from_remote("Planning", Some("notes"), 2000, 2900, "g-event-1", "etag-1", 500, 500)
            .unwrap();

        let events = db.list_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|e| e.id == local && e.external_id.is_none()));
        let remote_event = events.iter().find(|e| e.id == remote).unwrap();
        assert_eq!(remote_event.external_id.as_deref(), Some("g-event-1"));

        db.apply_remote_event(remote, "Planning v2", None, 2100, 3000, "etag-2", 600).unwrap();
        let updated = db.get_event(remote).unwrap().unwrap();
        assert_eq!(updated.title, "Planning v2");
        assert_eq!(updated.etag.as_deref(), Some("etag-2"));
    }
}
