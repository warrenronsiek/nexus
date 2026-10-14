-- @feature papercuts
-- @feature persistence
-- @spec docs/features/papercuts.md
-- @spec docs/features/persistence.md
CREATE TABLE papercuts (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) > 0),
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 256),
    description TEXT NOT NULL CHECK(length(description) <= 8192),
    fingerprint TEXT NOT NULL CHECK(length(fingerprint) = 64),
    reporter_token_hash TEXT NOT NULL CHECK(length(reporter_token_hash) = 64),
    status TEXT NOT NULL CHECK(status IN ('open', 'resolved', 'consolidated')),
    consolidated_into TEXT REFERENCES papercuts(id),
    resolution TEXT,
    creator_agent TEXT NOT NULL CHECK(length(creator_agent) > 0),
    creator_session_id TEXT NOT NULL CHECK(length(creator_session_id) > 0),
    creator_model_id TEXT,
    creator_config_hash TEXT NOT NULL CHECK(length(creator_config_hash) > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    resolved_at TEXT,
    CHECK(consolidated_into IS NULL OR consolidated_into <> id),
    CHECK(
        (
            status = 'open'
            AND consolidated_into IS NULL
            AND resolution IS NULL
            AND resolved_at IS NULL
        )
        OR (
            status = 'resolved'
            AND consolidated_into IS NULL
            AND resolution IS NOT NULL
            AND length(resolution) > 0
            AND resolved_at IS NOT NULL
        )
        OR (
            status = 'consolidated'
            AND consolidated_into IS NOT NULL
            AND resolution IS NULL
            AND resolved_at IS NULL
        )
    )
);

CREATE UNIQUE INDEX papercuts_open_fingerprint
ON papercuts(project_id, fingerprint)
WHERE status = 'open';

CREATE INDEX papercuts_project_status
ON papercuts(project_id, status, updated_at DESC, id);

CREATE TABLE papercut_upvotes (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) > 0),
    papercut_id TEXT NOT NULL REFERENCES papercuts(id),
    reported_papercut_id TEXT NOT NULL REFERENCES papercuts(id),
    reporter_agent TEXT NOT NULL CHECK(length(reporter_agent) > 0),
    reporter_session_id TEXT NOT NULL CHECK(length(reporter_session_id) > 0),
    reporter_model_id TEXT,
    reporter_config_hash TEXT NOT NULL CHECK(length(reporter_config_hash) > 0),
    note TEXT CHECK(note IS NULL OR length(note) BETWEEN 1 AND 8192),
    created_at TEXT NOT NULL
);

CREATE INDEX papercut_upvotes_canonical_time
ON papercut_upvotes(papercut_id, created_at, id);

CREATE INDEX papercut_upvotes_project_time
ON papercut_upvotes(project_id, created_at, id);

CREATE TABLE papercut_duplicate_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) > 0),
    papercut_id TEXT NOT NULL REFERENCES papercuts(id),
    canonical_id TEXT NOT NULL REFERENCES papercuts(id),
    reporter_session_id TEXT NOT NULL CHECK(length(reporter_session_id) > 0),
    rationale TEXT NOT NULL CHECK(length(rationale) BETWEEN 1 AND 8192),
    question TEXT NOT NULL CHECK(length(question) BETWEEN 1 AND 8192),
    response TEXT,
    status TEXT NOT NULL CHECK(status IN ('pending_reporter', 'approved', 'rejected')),
    reviewer_agent TEXT NOT NULL CHECK(length(reviewer_agent) > 0),
    reviewer_session_id TEXT NOT NULL CHECK(length(reviewer_session_id) > 0),
    provider TEXT,
    model_id TEXT,
    created_at TEXT NOT NULL,
    responded_at TEXT,
    CHECK(papercut_id <> canonical_id),
    CHECK(
        (status = 'pending_reporter' AND response IS NULL AND responded_at IS NULL)
        OR (status <> 'pending_reporter' AND responded_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX papercut_duplicate_reviews_pending_source
ON papercut_duplicate_reviews(project_id, papercut_id)
WHERE status = 'pending_reporter';

CREATE INDEX papercut_duplicate_reviews_project_time
ON papercut_duplicate_reviews(project_id, created_at, id);

CREATE TABLE papercut_review_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL CHECK(length(project_id) > 0),
    papercut_id TEXT NOT NULL REFERENCES papercuts(id),
    outcome TEXT NOT NULL CHECK(outcome IN ('distinct', 'failed')),
    provider TEXT NOT NULL CHECK(length(provider) BETWEEN 1 AND 256),
    model_id TEXT CHECK(model_id IS NULL OR length(model_id) BETWEEN 1 AND 256),
    detail TEXT NOT NULL CHECK(length(detail) BETWEEN 1 AND 8192),
    config_hash TEXT NOT NULL CHECK(length(config_hash) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL CHECK(length(created_at) > 0)
);

CREATE INDEX papercut_review_attempts_project_time
ON papercut_review_attempts(project_id, created_at, id);
