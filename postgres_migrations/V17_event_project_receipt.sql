-- @feature persistence
-- @feature observability-ui
-- @spec docs/features/persistence.md
-- @spec docs/features/observability-ui.md
CREATE INDEX events_project_receipt ON events(project_id, created_at DESC);
