-- @feature agent-traces
-- @feature persistence
-- @spec docs/features/agent-traces.md
-- @spec docs/features/persistence.md

CREATE TABLE trace_batches (
    id TEXT NOT NULL PRIMARY KEY,
    project_id TEXT NOT NULL,
    application_id TEXT NOT NULL,
    environment TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    received_at TEXT NOT NULL,
    accepted_events INTEGER NOT NULL,
    duplicate_events INTEGER NOT NULL,
    projected BOOLEAN NOT NULL
);
CREATE INDEX trace_batches_received ON trace_batches (received_at, id);
CREATE INDEX trace_batches_pending ON trace_batches (received_at, id) WHERE projected = false;
CREATE INDEX trace_batches_project_pending ON trace_batches (project_id, application_id, environment, received_at) WHERE projected = false;

CREATE TABLE trace_events (
    id TEXT NOT NULL PRIMARY KEY,
    batch_key TEXT NOT NULL REFERENCES trace_batches (id),
    project_id TEXT NOT NULL,
    application_id TEXT NOT NULL,
    environment TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    conversation_id TEXT,
    occurred_at TEXT NOT NULL,
    source_sequence BIGINT,
    event_json TEXT NOT NULL,
    payload_json TEXT,
    payload_bytes BIGINT NOT NULL,
    payload_expired BOOLEAN NOT NULL,
    content_hash TEXT NOT NULL
);
CREATE INDEX trace_events_batch ON trace_events (batch_key);
CREATE INDEX trace_events_payload_expiry ON trace_events (batch_key, id) WHERE payload_json IS NOT NULL;
CREATE INDEX trace_events_trace ON trace_events (project_id, application_id, environment, trace_id, (source_sequence IS NULL), source_sequence, occurred_at, event_id);
CREATE INDEX trace_events_project_time ON trace_events (project_id, occurred_at);
CREATE INDEX trace_events_scope ON trace_events (project_id, application_id, environment, agent_id);

CREATE TABLE trace_invocations (
    id TEXT NOT NULL PRIMARY KEY,
    project_id TEXT NOT NULL,
    application_id TEXT NOT NULL,
    environment TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT,
    version TEXT,
    occurred_at TEXT NOT NULL,
    outcome TEXT,
    duration_ms BIGINT,
    started_event_id TEXT,
    finished_event_id TEXT
);
CREATE INDEX trace_invocations_project_time ON trace_invocations (project_id, occurred_at, id);
CREATE INDEX trace_invocations_scope_time ON trace_invocations (project_id, application_id, environment, agent_id, occurred_at, id);
CREATE INDEX trace_invocations_capability ON trace_invocations (project_id, application_id, environment, kind, name, version);

ALTER TABLE sos_reports ADD COLUMN trace_ref_json TEXT;
