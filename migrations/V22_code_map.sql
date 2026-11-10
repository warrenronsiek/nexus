-- @feature code-map
-- @feature persistence
-- @spec docs/features/code-map.md
-- @spec docs/features/persistence.md
CREATE TABLE code_map_snapshots (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    producer TEXT NOT NULL,
    measure TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    reporter TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
);
CREATE INDEX code_map_measure_snapshots
    ON code_map_snapshots (project_id, producer, measure, observed_at DESC, id DESC);

ALTER TABLE concern_encounters ADD COLUMN map_dimensions_json TEXT NOT NULL DEFAULT '{}';
