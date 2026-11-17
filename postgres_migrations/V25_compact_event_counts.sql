-- @feature persistence
-- @feature observability-ui
-- @spec docs/features/persistence.md
-- @spec docs/features/observability-ui.md
-- Equal project keys can share compact B-tree posting lists. Composite receipt
-- keys stay necessary for history pages but repeat a unique suffix per event.
CREATE INDEX events_project_count ON events(project_id);
