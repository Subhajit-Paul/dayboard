//! Google Calendar (events) + Google Tasks (tasks/subtasks) two-way sync.
//!
//! Conflict policy is last-write-wins by comparing our `updated_at` against
//! Google's `updated` timestamp — no merge, no CRDT. Deletions are
//! propagated via `sync_tombstones` (see schema.sql): deleting a synced
//! item locally records its external_id there, and the next sync issues a
//! real DELETE to Google before doing anything else, so a deleted item
//! doesn't get resurrected by the pull step later in the same run.
//!
//! ponytail: re-parenting an already-synced subtask isn't pushed (Google
//! Tasks needs a separate `move` call for that) — only the parent set at
//! creation time is synced. Add `tasks.move` support if re-parenting synced
//! tasks turns out to matter.

use chrono::DateTime;

use crate::auth;
use crate::db::{now, Db};
use crate::models::{Event, SyncKind, Task};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const CALENDAR_EVENTS_URL: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const TASKS_URL: &str = "https://www.googleapis.com/tasks/v1/lists/@default/tasks";

/// Runs a full sync pass. No-op (not an error) if the user hasn't connected
/// a Google account yet, so callers like the daemon's poll loop can call
/// this unconditionally.
pub fn run(db: &Db) -> Result<()> {
    if !auth::is_authenticated() {
        return Ok(());
    }
    let token = auth::get_access_token()?;
    let http = reqwest::blocking::Client::new();
    sync_tasks(db, &http, &token)?;
    sync_events(db, &http, &token)?;
    Ok(())
}

fn parse_rfc3339(s: &str) -> i64 {
    DateTime::parse_from_rfc3339(s).map(|d| d.timestamp()).unwrap_or_else(|_| now())
}

fn to_rfc3339(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0).unwrap_or_default().to_rfc3339()
}

/// What to do with one remote item given the local row (if any) linked to
/// it. Pure and unit-testable independent of any HTTP/DB plumbing.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    InsertLocal,
    UpdateLocal,
    PushLocal,
    Noop,
}

fn decide(local_updated_at: Option<i64>, remote_updated_at: i64) -> Action {
    match local_updated_at {
        None => Action::InsertLocal,
        Some(local) if remote_updated_at > local => Action::UpdateLocal,
        Some(local) if local > remote_updated_at => Action::PushLocal,
        Some(_) => Action::Noop,
    }
}

// --- Google Tasks ---

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct GTask {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    #[serde(default = "needs_action")]
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated: Option<String>,
    #[serde(default)]
    deleted: bool,
}

fn needs_action() -> String {
    "needsAction".to_string()
}

#[derive(Debug, serde::Deserialize, Default)]
struct GTaskList {
    #[serde(default)]
    items: Vec<GTask>,
}

fn task_body(task: &Task) -> GTask {
    GTask {
        title: task.title.clone(),
        notes: task.description.clone(),
        status: if task.done { "completed".into() } else { "needsAction".into() },
        ..Default::default()
    }
}

fn push_new_task(http: &reqwest::blocking::Client, token: &str, db: &Db, task: &Task) -> Result<()> {
    let parent_external = match task.parent_id {
        Some(pid) => db.get_task(pid)?.and_then(|p| p.external_id),
        None => None,
    };
    let mut req = http.post(TASKS_URL).bearer_auth(token).json(&task_body(task));
    if let Some(parent_ext) = &parent_external {
        req = req.query(&[("parent", parent_ext.as_str())]);
    }
    let created: GTask = req.send()?.error_for_status()?.json()?;
    if let (Some(id), Some(etag)) = (created.id, created.etag) {
        db.set_task_external(task.id, &id, &etag)?;
    }
    Ok(())
}

fn push_task_update(http: &reqwest::blocking::Client, token: &str, db: &Db, task: &Task) -> Result<()> {
    let external_id = task
        .external_id
        .as_ref()
        .ok_or("push_task_update called without external_id")?;
    let updated: GTask = http
        .patch(format!("{TASKS_URL}/{external_id}"))
        .bearer_auth(token)
        .json(&task_body(task))
        .send()?
        .error_for_status()?
        .json()?;
    if let Some(etag) = updated.etag {
        db.set_task_etag(task.id, &etag)?;
    }
    Ok(())
}

fn sync_tasks(db: &Db, http: &reqwest::blocking::Client, token: &str) -> Result<()> {
    for external_id in db.list_tombstones(SyncKind::Task)? {
        let resp = http
            .delete(format!("{TASKS_URL}/{external_id}"))
            .bearer_auth(token)
            .send()?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
            db.remove_tombstone(&external_id, SyncKind::Task)?;
        }
    }

    // Roots before subtasks, so a subtask's parent already has an
    // external_id by the time we try to push it.
    let mut locals = db.list_tasks()?;
    locals.sort_by_key(|t| t.parent_id.is_some());
    for task in &locals {
        if task.external_id.is_none() {
            push_new_task(http, token, db, task)?;
        }
    }

    let remote: GTaskList = http
        .get(TASKS_URL)
        .bearer_auth(token)
        .query(&[("showCompleted", "true"), ("showHidden", "true")])
        .send()?
        .error_for_status()?
        .json()?;

    for item in remote.items {
        if item.deleted {
            continue;
        }
        let Some(external_id) = item.id.clone() else { continue };
        let remote_updated = item.updated.as_deref().map(parse_rfc3339).unwrap_or_else(now);
        let done = item.status == "completed";
        let parent_id = match &item.parent {
            Some(parent_ext) => db.find_task_by_external_id(parent_ext)?.map(|t| t.id),
            None => None,
        };
        let etag = item.etag.as_deref().unwrap_or("");
        let existing = db.find_task_by_external_id(&external_id)?;

        match decide(existing.as_ref().map(|t| t.updated_at), remote_updated) {
            Action::InsertLocal => {
                db.insert_task_from_remote(
                    &item.title,
                    item.notes.as_deref(),
                    done,
                    parent_id,
                    &external_id,
                    etag,
                    remote_updated,
                )?;
            }
            Action::UpdateLocal => {
                let id = existing.unwrap().id;
                db.apply_remote_task(id, &item.title, item.notes.as_deref(), done, parent_id, etag, remote_updated)?;
            }
            Action::PushLocal => {
                push_task_update(http, token, db, &existing.unwrap())?;
            }
            Action::Noop => {}
        }
    }
    Ok(())
}

// --- Google Calendar events ---

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct GTime {
    #[serde(skip_serializing_if = "Option::is_none", rename = "dateTime")]
    date_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct GEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    start: GTime,
    end: GTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct GEventList {
    #[serde(default)]
    items: Vec<GEvent>,
}

fn gtime_to_ts(t: &GTime) -> i64 {
    if let Some(dt) = &t.date_time {
        return parse_rfc3339(dt);
    }
    if let Some(date) = &t.date {
        // All-day event: midnight UTC of that date.
        return parse_rfc3339(&format!("{date}T00:00:00Z"));
    }
    now()
}

fn event_body(event: &Event) -> GEvent {
    GEvent {
        summary: Some(event.title.clone()),
        description: event.description.clone(),
        start: GTime { date_time: Some(to_rfc3339(event.start_at)), date: None },
        end: GTime { date_time: Some(to_rfc3339(event.end_at)), date: None },
        ..Default::default()
    }
}

fn push_new_event(http: &reqwest::blocking::Client, token: &str, db: &Db, event: &Event) -> Result<()> {
    let created: GEvent = http
        .post(CALENDAR_EVENTS_URL)
        .bearer_auth(token)
        .json(&event_body(event))
        .send()?
        .error_for_status()?
        .json()?;
    if let (Some(id), Some(etag)) = (created.id, created.etag) {
        db.set_event_external(event.id, &id, &etag)?;
    }
    Ok(())
}

fn push_event_update(http: &reqwest::blocking::Client, token: &str, db: &Db, event: &Event) -> Result<()> {
    let external_id = event
        .external_id
        .as_ref()
        .ok_or("push_event_update called without external_id")?;
    let updated: GEvent = http
        .put(format!("{CALENDAR_EVENTS_URL}/{external_id}"))
        .bearer_auth(token)
        .json(&event_body(event))
        .send()?
        .error_for_status()?
        .json()?;
    if let Some(etag) = updated.etag {
        db.set_event_etag(event.id, &etag)?;
    }
    Ok(())
}

fn sync_events(db: &Db, http: &reqwest::blocking::Client, token: &str) -> Result<()> {
    for external_id in db.list_tombstones(SyncKind::Event)? {
        let resp = http
            .delete(format!("{CALENDAR_EVENTS_URL}/{external_id}"))
            .bearer_auth(token)
            .send()?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
            db.remove_tombstone(&external_id, SyncKind::Event)?;
        }
    }

    for event in db.list_events()? {
        if event.external_id.is_none() {
            push_new_event(http, token, db, &event)?;
        }
    }

    // A window covering recent past through a year out keeps this from
    // pulling someone's entire multi-decade calendar history.
    let time_min = to_rfc3339(now() - 30 * 86400);
    let time_max = to_rfc3339(now() + 365 * 86400);
    let remote: GEventList = http
        .get(CALENDAR_EVENTS_URL)
        .bearer_auth(token)
        .query(&[
            ("singleEvents", "true"),
            ("timeMin", time_min.as_str()),
            ("timeMax", time_max.as_str()),
        ])
        .send()?
        .error_for_status()?
        .json()?;

    for item in remote.items {
        if item.status.as_deref() == Some("cancelled") {
            continue;
        }
        let Some(external_id) = item.id.clone() else { continue };
        let remote_updated = item.updated.as_deref().map(parse_rfc3339).unwrap_or_else(now);
        let remote_created = item.created.as_deref().map(parse_rfc3339).unwrap_or(remote_updated);
        let title = item.summary.clone().unwrap_or_default();
        let start_at = gtime_to_ts(&item.start);
        let end_at = gtime_to_ts(&item.end);
        let etag = item.etag.as_deref().unwrap_or("");
        let existing = db.find_event_by_external_id(&external_id)?;

        match decide(existing.as_ref().map(|e| e.updated_at), remote_updated) {
            Action::InsertLocal => {
                db.insert_event_from_remote(
                    &title,
                    item.description.as_deref(),
                    start_at,
                    end_at,
                    &external_id,
                    etag,
                    remote_created,
                    remote_updated,
                )?;
            }
            Action::UpdateLocal => {
                let id = existing.unwrap().id;
                db.apply_remote_event(id, &title, item.description.as_deref(), start_at, end_at, etag, remote_updated)?;
            }
            Action::PushLocal => {
                push_event_update(http, token, db, &existing.unwrap())?;
            }
            Action::Noop => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_picks_the_newer_side() {
        assert_eq!(decide(None, 100), Action::InsertLocal);
        assert_eq!(decide(Some(100), 200), Action::UpdateLocal);
        assert_eq!(decide(Some(200), 100), Action::PushLocal);
        assert_eq!(decide(Some(150), 150), Action::Noop);
    }
}
