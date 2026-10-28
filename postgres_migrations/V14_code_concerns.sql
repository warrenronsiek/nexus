-- @feature persistence
-- @feature code-concerns
-- @spec docs/features/persistence.md
-- @spec docs/features/code-concerns.md
CREATE TABLE concern_observations (
 project_id TEXT NOT NULL, id TEXT NOT NULL, concern_id TEXT NOT NULL,
 canonical_id TEXT NOT NULL, kind TEXT NOT NULL, occurred_at TEXT,
 observed_at TEXT NOT NULL, recorded_at TEXT NOT NULL, effective_time TEXT NOT NULL,
 payload_json TEXT NOT NULL, content_hash TEXT NOT NULL, provenance_json TEXT NOT NULL,
 source_json TEXT, reporter_token_hash TEXT, PRIMARY KEY(project_id,id)
);
CREATE INDEX concern_observation_history ON concern_observations(project_id,concern_id,effective_time,id);
CREATE TABLE code_concerns (
 project_id TEXT NOT NULL, id TEXT NOT NULL, canonical_id TEXT NOT NULL,
 fingerprint TEXT NOT NULL, title TEXT NOT NULL, status TEXT NOT NULL,
 last_observed_at TEXT NOT NULL, work_session_count BIGINT NOT NULL,
 returns_count BIGINT NOT NULL, summary_json TEXT NOT NULL, reporter_token_hash TEXT,
 PRIMARY KEY(project_id,id)
);
CREATE INDEX concern_exact_match ON code_concerns(project_id,fingerprint);
CREATE INDEX concern_canonical ON code_concerns(project_id,canonical_id);
CREATE INDEX concern_recent ON code_concerns(project_id,last_observed_at,id);
CREATE TABLE concern_encounters (
 project_id TEXT NOT NULL, event_id TEXT NOT NULL, concern_id TEXT NOT NULL,
 reported_concern_id TEXT NOT NULL, source_key TEXT, work_session_id TEXT,
 host_session_key TEXT, source_time TEXT, effective_time TEXT NOT NULL,
 observed_at TEXT NOT NULL, effective_day TEXT NOT NULL, repository TEXT, returned_after_resolution INTEGER NOT NULL,
 PRIMARY KEY(project_id,event_id)
);
CREATE INDEX concern_encounter_population ON concern_encounters(project_id,concern_id,effective_time,event_id);
CREATE INDEX concern_encounter_source ON concern_encounters(project_id,source_key);
CREATE TABLE concern_areas (
 project_id TEXT NOT NULL, id TEXT NOT NULL, repository TEXT NOT NULL,
 kind TEXT NOT NULL, area_key TEXT NOT NULL, area_json TEXT NOT NULL,
 PRIMARY KEY(project_id,id)
);
CREATE TABLE concern_attributions (
 project_id TEXT NOT NULL, event_id TEXT NOT NULL, area_id TEXT NOT NULL,
 relation TEXT NOT NULL, PRIMARY KEY(project_id,event_id,area_id,relation)
);
CREATE INDEX concern_area_encounters ON concern_attributions(project_id,area_id,event_id);
CREATE TABLE concern_decisions (
 project_id TEXT NOT NULL, proposal_id TEXT NOT NULL, event_id TEXT NOT NULL,
 concern_id TEXT NOT NULL, decision TEXT NOT NULL, PRIMARY KEY(project_id,proposal_id)
);
