-- @feature persistence
-- @feature agent-memory
-- @spec docs/features/persistence.md
-- @spec docs/features/agent-memory.md
CREATE TABLE sessions_v6 (
    session_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    worktree TEXT,
    task_summary TEXT,
    prompt_hash TEXT,
    status TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    started_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY(project_id, session_id)
);

INSERT INTO sessions_v6 (
    session_id,
    project_id,
    agent,
    worktree,
    task_summary,
    prompt_hash,
    status,
    config_hash,
    started_at,
    last_seen_at
)
SELECT
    session_id,
    project_id,
    agent,
    worktree,
    task_summary,
    prompt_hash,
    status,
    config_hash,
    started_at,
    last_seen_at
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_v6 RENAME TO sessions;
CREATE INDEX sessions_project_id ON sessions(project_id, status);

CREATE TABLE memory_activations_v6 (
    id TEXT PRIMARY KEY NOT NULL,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    read_scope TEXT NOT NULL CHECK(read_scope IN ('global', 'project', 'layered')),
    project_id TEXT,
    activation_scope_key TEXT NOT NULL,
    delivered_items INTEGER NOT NULL CHECK(delivered_items >= 0),
    omitted_ranges INTEGER NOT NULL CHECK(omitted_ranges >= 0),
    activated_at TEXT NOT NULL,
    UNIQUE(agent, session_id, activation_scope_key),
    CHECK(activation_scope_key = read_scope || ':' || COALESCE(project_id, '')),
    CHECK(
        (read_scope = 'global' AND project_id IS NULL)
        OR (
            read_scope IN ('project', 'layered')
            AND project_id IS NOT NULL
            AND length(project_id) > 0
        )
    )
);

INSERT INTO memory_activations_v6 (
    id,
    agent,
    session_id,
    read_scope,
    project_id,
    activation_scope_key,
    delivered_items,
    omitted_ranges,
    activated_at
)
SELECT
    id,
    agent,
    session_id,
    read_scope,
    project_id,
    read_scope || ':' || COALESCE(project_id, ''),
    delivered_items,
    omitted_ranges,
    activated_at
FROM memory_activations;

DROP TABLE memory_activations;
ALTER TABLE memory_activations_v6 RENAME TO memory_activations;
CREATE INDEX memory_activations_recent
ON memory_activations(activated_at DESC);
CREATE INDEX memory_activations_scope_recent
ON memory_activations(project_id, activated_at DESC);
