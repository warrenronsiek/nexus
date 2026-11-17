-- @feature persistence
-- @feature work-history
-- @feature usage-analytics
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
-- @spec docs/features/usage-analytics.md
-- Exact session correlation must not scan all historical calls and actions.
CREATE INDEX analytics_calls_session_invocation
    ON analytics_calls(project_id, session_key, invocation_id, id);
CREATE INDEX analytics_actions_call
    ON analytics_actions(call_id, id);
