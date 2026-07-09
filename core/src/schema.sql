CREATE TABLE IF NOT EXISTS tasks (
    id          INTEGER PRIMARY KEY,
    parent_id   INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT,
    done        INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    external_id TEXT,
    etag        TEXT
);
CREATE INDEX IF NOT EXISTS idx_tasks_parent_id ON tasks(parent_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_external_id
    ON tasks(external_id) WHERE external_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS reminders (
    id         INTEGER PRIMARY KEY,
    task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    remind_at  INTEGER NOT NULL,
    fired_at   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_reminders_task_id ON reminders(task_id);
CREATE INDEX IF NOT EXISTS idx_reminders_pending ON reminders(remind_at) WHERE fired_at IS NULL;

CREATE TABLE IF NOT EXISTS events (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    description TEXT,
    start_at    INTEGER NOT NULL,
    end_at      INTEGER NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    external_id TEXT,
    etag        TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_external_id
    ON events(external_id) WHERE external_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_start_at ON events(start_at);

-- Records a locally-deleted task/event that still had a Google external_id,
-- so periodic sync knows to delete it remotely too instead of silently
-- resurrecting it on the next pull.
CREATE TABLE IF NOT EXISTS sync_tombstones (
    external_id TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('task', 'event')),
    PRIMARY KEY (external_id, kind)
);
