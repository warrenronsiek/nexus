-- @feature persistence
-- @feature work-history
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
-- Request timing must seek activity kinds instead of scanning every subject.
CREATE INDEX work_subject_kind ON work_subjects(project_id, scope, kind, id);
