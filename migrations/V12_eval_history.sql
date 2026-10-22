-- @feature evals
-- @feature persistence
-- @spec docs/features/evals.md
-- @spec docs/features/persistence.md
CREATE TABLE eval_records (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    search_text TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
);
CREATE INDEX eval_records_kind ON eval_records(project_id, kind, id);
CREATE INDEX eval_records_time ON eval_records(project_id, occurred_at, id);
CREATE TABLE eval_links (
    project_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    PRIMARY KEY (project_id, source_id, target_id, relation),
    FOREIGN KEY (project_id, source_id) REFERENCES eval_records(project_id, id),
    FOREIGN KEY (project_id, target_id) REFERENCES eval_records(project_id, id)
);
CREATE INDEX eval_links_target ON eval_links(project_id, target_id);
CREATE TABLE eval_dimensions (
    project_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    dimension TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (project_id, record_id, dimension, value),
    FOREIGN KEY (project_id, record_id) REFERENCES eval_records(project_id, id)
);
CREATE INDEX eval_dimensions_filter ON eval_dimensions(project_id, dimension, value, record_id);
CREATE TABLE eval_blobs (
    project_id TEXT NOT NULL,
    digest TEXT NOT NULL,
    upload_id TEXT NOT NULL,
    byte_size BIGINT NOT NULL CHECK (byte_size >= 0 AND byte_size <= 1073741824),
    chunk_count INTEGER NOT NULL CHECK (chunk_count >= 0 AND chunk_count <= 4096),
    verified BOOLEAN NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (project_id, digest)
);
CREATE TABLE eval_blob_chunks (
    project_id TEXT NOT NULL,
    digest TEXT NOT NULL,
    chunk_index INTEGER NOT NULL CHECK (chunk_index >= 0 AND chunk_index < 4096),
    bytes BLOB NOT NULL,
    PRIMARY KEY (project_id, digest, chunk_index),
    FOREIGN KEY (project_id, digest) REFERENCES eval_blobs(project_id, digest)
);
