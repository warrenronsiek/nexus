-- @feature evals
-- @feature persistence
-- @spec docs/features/evals.md
-- @spec docs/features/persistence.md
CREATE INDEX eval_records_receipt ON eval_records(project_id, recorded_at DESC, id ASC);
