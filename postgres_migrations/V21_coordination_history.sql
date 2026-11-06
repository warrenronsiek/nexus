-- @feature persistence
-- @feature coordination
-- @spec docs/features/persistence.md
-- @spec docs/features/coordination.md
-- Recent-history counts and session timelines read advisories and conflicts by
-- project and time, and advisories delivered to a counterpart by conflict.
CREATE INDEX advisories_project_created_at ON advisories(project_id, created_at);
CREATE INDEX advisories_conflict_created_at ON advisories(conflict_id, created_at);
CREATE INDEX conflicts_project_created_at ON conflicts(project_id, created_at);
CREATE INDEX conflicts_project_updated_at ON conflicts(project_id, updated_at);
