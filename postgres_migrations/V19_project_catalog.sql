-- @feature persistence
-- @feature observability-ui
-- @spec docs/features/persistence.md
-- @spec docs/features/observability-ui.md
-- Project discovery reads this catalog instead of aggregating every history table.
CREATE TABLE projects (
    project_id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL
);
CREATE INDEX projects_recent ON projects(last_seen_at DESC, project_id);
INSERT INTO projects (project_id, display_name, first_seen_at, last_seen_at)
SELECT project_id, NULL, MIN(first_seen_at), MAX(last_seen_at)
FROM (
    SELECT project_id, MIN(started_at) AS first_seen_at, MAX(last_seen_at) AS last_seen_at FROM sessions GROUP BY project_id
    UNION ALL SELECT project_id, MIN(recorded_at), MAX(recorded_at) FROM work_observations GROUP BY project_id
    UNION ALL SELECT project_id, MIN(recorded_at), MAX(recorded_at) FROM eval_records GROUP BY project_id
    UNION ALL SELECT project_id, MIN(recorded_at), MAX(recorded_at) FROM concern_observations GROUP BY project_id
    UNION ALL SELECT project_id, MIN(created_at), MAX(updated_at) FROM papercuts GROUP BY project_id
    UNION ALL SELECT project_id, MIN(created_at), MAX(created_at) FROM sos_reports GROUP BY project_id
    UNION ALL SELECT project_id, MIN(created_at), MAX(updated_at) FROM sos_issues GROUP BY project_id
    UNION ALL SELECT project_id, MIN(created_at), MAX(updated_at) FROM memory_spaces WHERE project_id IS NOT NULL GROUP BY project_id
    UNION ALL SELECT project_id, MIN(first_observed_at), MAX(COALESCE(completed_at, first_observed_at)) FROM capability_uses GROUP BY project_id
    UNION ALL SELECT project_id, MIN(observed_at), MAX(observed_at) FROM analytics_calls GROUP BY project_id
    UNION ALL SELECT project_id, MIN(first_seen), MAX(last_seen) FROM analytics_resources GROUP BY project_id
    UNION ALL SELECT project_id, MIN(first_seen), MAX(last_seen) FROM analytics_exposures GROUP BY project_id
    UNION ALL SELECT project_id, MIN(created_at), MAX(created_at) FROM events GROUP BY project_id
) AS activity
GROUP BY project_id;
