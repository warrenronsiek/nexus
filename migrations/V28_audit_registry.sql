-- @feature audit-registry
-- @feature persistence
-- @spec docs/features/audit-registry.md
-- @spec docs/features/persistence.md
CREATE TABLE audit_records (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    ordinal BIGINT NOT NULL CHECK (ordinal > 0),
    kind TEXT NOT NULL CHECK (kind IN ('define', 'retire', 'run', 'correct_run', 'disposition', 'verification')),
    control_key TEXT NOT NULL,
    control_version INTEGER NOT NULL CHECK (control_version > 0),
    run_id TEXT,
    finding_id TEXT,
    occurred_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    actor_json TEXT NOT NULL,
    input_json TEXT NOT NULL,
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, ordinal)
);

CREATE INDEX audit_run_history ON audit_records (project_id, run_id, ordinal);
CREATE INDEX audit_control_history ON audit_records (project_id, control_key, kind, control_version, ordinal);
CREATE INDEX audit_run_period ON audit_records (project_id, occurred_at, ordinal) WHERE kind = 'run';
