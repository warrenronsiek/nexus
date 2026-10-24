-- @feature persistence
-- @feature work-history
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
CREATE TABLE work_observations (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    subject_scope TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    source_json TEXT,
    PRIMARY KEY(project_id,id)
);
CREATE INDEX work_observation_subject ON work_observations(project_id,subject_scope,subject_id,id);
CREATE TABLE work_subjects (
    project_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    repository TEXT,
    host_session_id TEXT,
    environment TEXT,
    outcome TEXT,
    occurred_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY(project_id,scope,id)
);
CREATE INDEX work_subject_scope ON work_subjects(project_id,scope,id);
CREATE TABLE work_links (
    project_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    source_scope TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    target_scope TEXT NOT NULL,
    target_id TEXT NOT NULL,
    target_json TEXT NOT NULL,
    PRIMARY KEY(project_id,event_id,relation,target_scope,target_id)
);
CREATE INDEX work_links_target ON work_links(project_id,target_scope,target_id,relation,source_id);
CREATE INDEX work_links_source ON work_links(project_id,source_scope,source_id,relation);
