-- @feature persistence
-- @feature work-history
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
-- Keep the lookup key order while covering the membership source-scope filter.
CREATE INDEX work_links_target_covering ON work_links(project_id, target_scope, target_id, relation, source_id) INCLUDE(source_scope);
DROP INDEX work_links_target;
ALTER INDEX work_links_target_covering RENAME TO work_links_target;
