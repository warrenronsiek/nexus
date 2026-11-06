-- @feature persistence
-- @feature usage-analytics
-- @spec docs/features/persistence.md
-- @spec docs/features/usage-analytics.md
-- Latest observed SKILL.md text per project and skill; replaced only when its hash changes.
CREATE TABLE analytics_skill_documents (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_path TEXT NOT NULL,
    observed_at TEXT NOT NULL
);
CREATE INDEX analytics_skill_documents_resource ON analytics_skill_documents(resource_key, observed_at);
