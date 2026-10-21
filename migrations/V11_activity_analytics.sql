-- @feature persistence
-- @feature usage-analytics
-- @spec docs/features/persistence.md
-- @spec docs/features/usage-analytics.md
CREATE TABLE analytics_calls (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    session_key TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    parent_id TEXT,
    tool_name TEXT NOT NULL,
    outcome TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    completed_at TEXT
);
CREATE TABLE analytics_actions (
    id TEXT PRIMARY KEY NOT NULL,
    call_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    origin TEXT NOT NULL,
    category_id TEXT NOT NULL,
    description TEXT NOT NULL,
    pattern_key TEXT NOT NULL,
    resource_version TEXT,
    evidence TEXT NOT NULL,
    model_id TEXT,
    observed_at TEXT NOT NULL
);
CREATE TABLE analytics_exposures (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    scope TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    origin TEXT NOT NULL,
    exposure TEXT NOT NULL,
    version TEXT,
    description TEXT NOT NULL,
    activities_json TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    active BOOLEAN NOT NULL
);
CREATE TABLE analytics_categories (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    definition TEXT NOT NULL,
    replaced_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE analytics_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    action_id TEXT NOT NULL,
    category_id TEXT NOT NULL,
    description TEXT NOT NULL,
    evidence TEXT NOT NULL,
    model_id TEXT,
    policy_version TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX analytics_calls_window ON analytics_calls(project_id, observed_at);
CREATE INDEX analytics_actions_window ON analytics_actions(project_id, observed_at, category_id);
CREATE INDEX analytics_actions_resource ON analytics_actions(resource_key, observed_at);
CREATE INDEX analytics_actions_pattern ON analytics_actions(project_id, pattern_key, evidence);
CREATE INDEX analytics_exposures_resource ON analytics_exposures(resource_key, last_seen);
CREATE INDEX analytics_revisions_action ON analytics_revisions(action_id, created_at);
CREATE TABLE analytics_resources (
    id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, resource_key TEXT NOT NULL,
    name TEXT NOT NULL, kind TEXT NOT NULL, origin TEXT NOT NULL, description TEXT NOT NULL,
    activities_json TEXT NOT NULL, version TEXT, first_seen TEXT NOT NULL, last_seen TEXT NOT NULL
);
CREATE INDEX analytics_resources_catalog ON analytics_resources(project_id, name, resource_key);

CREATE INDEX analytics_calls_parent ON analytics_calls(parent_id);
