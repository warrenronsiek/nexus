-- @feature persistence
-- @feature usage-analytics
-- @spec docs/features/persistence.md
-- @spec docs/features/usage-analytics.md
CREATE TABLE capability_uses (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT,
    invocation_id TEXT NOT NULL,
    parent_invocation_id TEXT,
    kind TEXT NOT NULL CHECK(kind IN ('tool', 'skill', 'script')),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 512),
    source TEXT NOT NULL CHECK(source IN ('host_hook', 'pi_event', 'shell_inference', 'nexus_exec', 'nexus_internal')),
    evidence TEXT NOT NULL CHECK(evidence IN ('native_hook', 'explicit_invocation', 'instruction_read', 'asset_execution', 'shell_parsed', 'wrapped')),
    outcome TEXT NOT NULL CHECK(outcome IN ('pending', 'succeeded', 'failed', 'interrupted', 'completed_unknown', 'observed')),
    config_hash TEXT NOT NULL,
    model_id TEXT,
    first_observed_at TEXT NOT NULL,
    completed_at TEXT,
    UNIQUE(project_id, agent, session_id, kind, invocation_id)
);

CREATE INDEX capability_uses_window
ON capability_uses(kind, first_observed_at, name);

CREATE INDEX capability_uses_project_window
ON capability_uses(project_id, kind, first_observed_at, name);

CREATE UNIQUE INDEX capability_uses_skill_turn
ON capability_uses(project_id, agent, session_id, kind, turn_id, name)
WHERE kind = 'skill' AND turn_id IS NOT NULL;
