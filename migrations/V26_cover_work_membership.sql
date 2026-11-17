-- @feature persistence
-- @feature work-history
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
-- SQLite covers the same filter with a trailing key column instead of INCLUDE.
DROP INDEX work_links_target;
CREATE INDEX work_links_target ON work_links(project_id, target_scope, target_id, relation, source_id, source_scope);
