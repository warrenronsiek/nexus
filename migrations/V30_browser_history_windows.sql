-- @feature persistence
-- @feature work-history
-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
CREATE INDEX work_subject_source_time ON work_subjects(project_id, scope, occurred_at DESC, id);
CREATE INDEX work_host_source_time ON work_subjects(project_id, scope, has_host_capture, occurred_at DESC, id);
CREATE INDEX coordination_session_recent ON sessions(project_id, last_seen_at DESC, session_id);
CREATE INDEX coordination_conflict_recent ON conflicts(project_id, updated_at DESC, id);
CREATE INDEX concern_encounter_time ON concern_encounters(project_id, effective_time, concern_id);
CREATE INDEX delivery_record_recent ON delivery_records(project_id, kind, updated_at DESC, id);
CREATE INDEX coordination_project_event_time ON events(project_id, created_at DESC, id DESC);
CREATE INDEX coordination_global_event_time ON events(created_at DESC, id DESC);
CREATE INDEX coordination_global_conflict_time ON conflicts(created_at);
CREATE INDEX coordination_global_advisory_time ON advisories(created_at);
CREATE INDEX delivery_project_recent ON delivery_records(project_id, updated_at DESC, kind, id);
