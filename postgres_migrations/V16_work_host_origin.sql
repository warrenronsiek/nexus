-- @feature persistence
-- @feature work-history
-- @spec docs/features/persistence.md
-- @spec docs/features/work-history.md
ALTER TABLE work_subjects ADD COLUMN has_host_capture BOOLEAN NOT NULL DEFAULT FALSE;
UPDATE work_subjects SET has_host_capture = TRUE
WHERE scope = 'session' AND EXISTS (
    SELECT 1 FROM work_observations AS observation
    WHERE observation.project_id = work_subjects.project_id
      AND observation.subject_scope = 'session'
      AND observation.subject_id = work_subjects.id
      AND observation.payload_json::jsonb -> 'body' -> 'context' -> 'labels' ->> 'nexus.capture.host' IS NOT NULL
);
CREATE INDEX work_subject_receipt ON work_subjects(project_id, scope, recorded_at DESC, id);
CREATE INDEX work_host_receipt ON work_subjects(project_id, scope, has_host_capture, recorded_at DESC, id);
