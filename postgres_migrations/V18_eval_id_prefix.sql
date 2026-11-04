-- @feature evals
-- @feature persistence
-- @spec docs/features/evals.md
-- @spec docs/features/persistence.md
CREATE INDEX eval_records_id_prefix ON eval_records(project_id, id text_pattern_ops) INCLUDE (recorded_at);
