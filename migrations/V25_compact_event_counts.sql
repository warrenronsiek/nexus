-- @feature persistence
-- @feature observability-ui
-- @spec docs/features/persistence.md
-- @spec docs/features/observability-ui.md
-- SQLite already serves these counts from its covering project/rowid index;
-- PostgreSQL's duplicate-key posting-list optimization has no SQLite equivalent.
SELECT 1;
