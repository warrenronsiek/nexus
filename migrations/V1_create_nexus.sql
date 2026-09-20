-- @feature persistence
-- @spec docs/features/persistence.md
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    project_id TEXT NOT NULL,
    session_id TEXT,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX events_project_id ON events(project_id, id);

CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    worktree TEXT,
    task_summary TEXT,
    prompt_hash TEXT,
    status TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    started_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL
);
CREATE INDEX sessions_project_id ON sessions(project_id, status);

CREATE TABLE claims (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    tool_use_id TEXT NOT NULL,
    path TEXT NOT NULL,
    operation TEXT NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    state TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, session_id, tool_use_id, path, operation, line_start, line_end)
);
CREATE INDEX claims_active_path ON claims(project_id, path, state, expires_at);

CREATE TABLE conflicts (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    left_claim_id TEXT NOT NULL,
    right_claim_id TEXT NOT NULL,
    path TEXT NOT NULL,
    severity TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX conflicts_project_id ON conflicts(project_id, status);

CREATE TABLE advisories (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    tool_use_id TEXT NOT NULL,
    conflict_id TEXT,
    severity TEXT NOT NULL,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL
);
