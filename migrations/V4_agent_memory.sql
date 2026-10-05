-- @feature agent-memory
-- @feature persistence
-- @spec docs/features/agent-memory.md
-- @spec docs/features/persistence.md
CREATE TABLE memory_spaces (
    id TEXT PRIMARY KEY NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('global', 'project')),
    project_id TEXT,
    next_ordinal BIGINT NOT NULL DEFAULT 0 CHECK(next_ordinal >= 0),
    revision BIGINT NOT NULL DEFAULT 0 CHECK(revision >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK(
        (scope = 'global' AND project_id IS NULL)
        OR (scope = 'project' AND project_id IS NOT NULL AND length(project_id) > 0)
    )
);

CREATE UNIQUE INDEX memory_spaces_one_global
ON memory_spaces(scope)
WHERE scope = 'global';

CREATE UNIQUE INDEX memory_spaces_project_identity
ON memory_spaces(project_id)
WHERE scope = 'project';

CREATE TABLE memory_entries (
    id TEXT PRIMARY KEY NOT NULL,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    ordinal BIGINT NOT NULL CHECK(ordinal >= 0),
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    model_id TEXT,
    config_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(space_id, ordinal),
    UNIQUE(space_id, content_hash)
);

CREATE INDEX memory_entries_newest
ON memory_entries(space_id, ordinal DESC);

CREATE TRIGGER memory_entries_immutable_update
BEFORE UPDATE ON memory_entries
BEGIN
    SELECT RAISE(ABORT, 'memory entries are immutable');
END;

CREATE TRIGGER memory_entries_immutable_delete
BEFORE DELETE ON memory_entries
BEGIN
    SELECT RAISE(ABORT, 'memory entries are immutable');
END;

CREATE TABLE memory_summaries (
    id TEXT PRIMARY KEY NOT NULL,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    level INTEGER NOT NULL CHECK(level > 0),
    start_ordinal BIGINT NOT NULL CHECK(start_ordinal >= 0),
    end_ordinal BIGINT NOT NULL CHECK(end_ordinal > start_ordinal),
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_id TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(space_id, start_ordinal, end_ordinal)
);

CREATE INDEX memory_summaries_tree
ON memory_summaries(space_id, level, start_ordinal);

CREATE TABLE memory_frontier_snapshots (
    space_id TEXT PRIMARY KEY NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK(revision >= 0),
    node_refs_json TEXT NOT NULL,
    omissions_json TEXT NOT NULL,
    item_count INTEGER NOT NULL CHECK(item_count >= 0 AND item_count <= 64),
    omission_count INTEGER NOT NULL CHECK(omission_count >= 0 AND omission_count <= 65),
    byte_count INTEGER NOT NULL CHECK(byte_count >= 0 AND byte_count <= 16384),
    built_at TEXT NOT NULL,
    CHECK(length(node_refs_json) <= 8192),
    CHECK(length(omissions_json) <= 32768)
);

CREATE TABLE memory_compaction_queue (
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    level INTEGER NOT NULL CHECK(level > 0),
    start_ordinal BIGINT NOT NULL CHECK(start_ordinal >= 0),
    end_ordinal BIGINT NOT NULL CHECK(end_ordinal > start_ordinal),
    source_hash TEXT NOT NULL,
    codex_retry_at TEXT,
    claude_retry_at TEXT,
    queued_at TEXT NOT NULL,
    PRIMARY KEY(space_id, level, start_ordinal, end_ordinal)
);

CREATE INDEX memory_compaction_queue_ready
ON memory_compaction_queue(level, space_id, start_ordinal);

CREATE INDEX memory_compaction_queue_codex_ready
ON memory_compaction_queue(level, codex_retry_at, space_id, start_ordinal);

CREATE INDEX memory_compaction_queue_claude_ready
ON memory_compaction_queue(level, claude_retry_at, space_id, start_ordinal);

CREATE INDEX memory_compaction_queue_codex_health
ON memory_compaction_queue(space_id, codex_retry_at);

CREATE INDEX memory_compaction_queue_claude_health
ON memory_compaction_queue(space_id, claude_retry_at);

CREATE TABLE memory_activations (
    id TEXT PRIMARY KEY NOT NULL,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    read_scope TEXT NOT NULL CHECK(read_scope IN ('global', 'project', 'layered')),
    project_id TEXT,
    delivered_items INTEGER NOT NULL CHECK(delivered_items >= 0),
    omitted_ranges INTEGER NOT NULL CHECK(omitted_ranges >= 0),
    activated_at TEXT NOT NULL,
    UNIQUE(agent, session_id),
    CHECK(
        (read_scope = 'global' AND project_id IS NULL)
        OR (
            read_scope IN ('project', 'layered')
            AND project_id IS NOT NULL
            AND length(project_id) > 0
        )
    )
);

CREATE INDEX memory_activations_recent
ON memory_activations(activated_at DESC);

CREATE INDEX memory_activations_scope_recent
ON memory_activations(project_id, activated_at DESC);

CREATE TABLE memory_compaction_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    level INTEGER NOT NULL CHECK(level > 0),
    start_ordinal BIGINT NOT NULL CHECK(start_ordinal >= 0),
    end_ordinal BIGINT NOT NULL CHECK(end_ordinal > start_ordinal),
    source_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_id TEXT,
    outcome TEXT NOT NULL CHECK(outcome IN ('succeeded', 'failed', 'timed_out', 'invalid_output', 'stale')),
    diagnostic TEXT,
    was_fallback BOOLEAN NOT NULL DEFAULT FALSE,
    consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK(consecutive_failures >= 0),
    attempted_at TEXT NOT NULL,
    next_retry_at TEXT
);

CREATE INDEX memory_compaction_attempts_cooldown
ON memory_compaction_attempts(space_id, level, start_ordinal, end_ordinal, source_hash, provider, attempted_at DESC);

CREATE INDEX memory_compaction_attempts_provider_recent
ON memory_compaction_attempts(space_id, provider, attempted_at DESC, id DESC);
