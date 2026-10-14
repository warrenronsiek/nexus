-- @feature sos
-- @feature persistence
-- @spec docs/features/sos.md
-- @spec docs/features/persistence.md
CREATE TABLE sos_issues (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) BETWEEN 1 AND 256),
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 256),
    summary TEXT NOT NULL CHECK(length(summary) BETWEEN 1 AND 8192),
    status TEXT NOT NULL CHECK(status IN ('open', 'resolved', 'deprecated')),
    resolution TEXT CHECK(resolution IS NULL OR length(resolution) BETWEEN 1 AND 8192),
    created_at TEXT NOT NULL CHECK(length(created_at) > 0),
    updated_at TEXT NOT NULL CHECK(length(updated_at) > 0),
    closed_at TEXT,
    CHECK(
        (status = 'open' AND resolution IS NULL AND closed_at IS NULL)
        OR (status <> 'open' AND resolution IS NOT NULL AND closed_at IS NOT NULL)
    )
);

CREATE INDEX sos_issues_project_status
ON sos_issues(project_id, status, updated_at DESC, id);

CREATE TABLE sos_reports (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) BETWEEN 1 AND 256),
    issue_id TEXT REFERENCES sos_issues(id),
    message TEXT NOT NULL CHECK(length(message) BETWEEN 1 AND 16384),
    report_type TEXT NOT NULL CHECK(length(report_type) BETWEEN 1 AND 256),
    environment TEXT NOT NULL CHECK(length(environment) BETWEEN 1 AND 512),
    version TEXT NOT NULL CHECK(length(version) BETWEEN 1 AND 256),
    model TEXT NOT NULL CHECK(length(model) BETWEEN 1 AND 256),
    trace TEXT NOT NULL CHECK(length(trace) <= 65536),
    metadata_json TEXT NOT NULL CHECK(length(metadata_json) <= 32768),
    reporter_agent TEXT NOT NULL CHECK(length(reporter_agent) BETWEEN 1 AND 256),
    reporter_session_id TEXT NOT NULL CHECK(length(reporter_session_id) BETWEEN 1 AND 512),
    reporter_model_id TEXT CHECK(reporter_model_id IS NULL OR length(reporter_model_id) BETWEEN 1 AND 256),
    reporter_config_hash TEXT NOT NULL CHECK(length(reporter_config_hash) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL CHECK(length(created_at) > 0)
);

CREATE INDEX sos_reports_project_time
ON sos_reports(project_id, created_at DESC, id);

CREATE INDEX sos_reports_issue_time
ON sos_reports(issue_id, created_at, id);

CREATE TABLE sos_triage_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) BETWEEN 1 AND 256),
    report_id TEXT NOT NULL REFERENCES sos_reports(id),
    issue_id TEXT REFERENCES sos_issues(id),
    outcome TEXT NOT NULL CHECK(outcome IN ('created', 'linked', 'failed')),
    provider TEXT NOT NULL CHECK(length(provider) BETWEEN 1 AND 256),
    model_id TEXT CHECK(model_id IS NULL OR length(model_id) BETWEEN 1 AND 256),
    rationale TEXT NOT NULL CHECK(length(rationale) BETWEEN 1 AND 8192),
    generated_title TEXT CHECK(generated_title IS NULL OR length(generated_title) BETWEEN 1 AND 256),
    generated_summary TEXT CHECK(generated_summary IS NULL OR length(generated_summary) BETWEEN 1 AND 8192),
    config_hash TEXT NOT NULL CHECK(length(config_hash) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL CHECK(length(created_at) > 0),
    CHECK(
        (outcome = 'failed' AND issue_id IS NULL AND generated_title IS NULL AND generated_summary IS NULL)
        OR (outcome <> 'failed' AND issue_id IS NOT NULL AND generated_title IS NOT NULL AND generated_summary IS NOT NULL)
    )
);

CREATE INDEX sos_triage_attempts_report_time
ON sos_triage_attempts(report_id, created_at, id);

CREATE INDEX sos_triage_attempts_project_time
ON sos_triage_attempts(project_id, created_at, id);

CREATE TABLE sos_issue_lifecycle (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) BETWEEN 1 AND 256),
    issue_id TEXT NOT NULL REFERENCES sos_issues(id),
    transition TEXT NOT NULL CHECK(transition IN ('resolved', 'deprecated', 'reopened')),
    reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 8192),
    actor_agent TEXT NOT NULL CHECK(length(actor_agent) BETWEEN 1 AND 256),
    actor_session_id TEXT NOT NULL CHECK(length(actor_session_id) BETWEEN 1 AND 512),
    actor_model_id TEXT CHECK(actor_model_id IS NULL OR length(actor_model_id) BETWEEN 1 AND 256),
    actor_config_hash TEXT NOT NULL CHECK(length(actor_config_hash) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL CHECK(length(created_at) > 0)
);

CREATE INDEX sos_issue_lifecycle_issue_time
ON sos_issue_lifecycle(issue_id, created_at, id);

CREATE INDEX sos_issue_lifecycle_project_time
ON sos_issue_lifecycle(project_id, created_at, id);
