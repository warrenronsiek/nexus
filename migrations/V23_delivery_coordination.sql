-- @feature delivery-coordination
-- @feature persistence
-- @spec docs/features/delivery-coordination.md
-- @spec docs/features/persistence.md
CREATE TABLE delivery_observations (
 project_id TEXT NOT NULL, id TEXT NOT NULL, ordinal BIGINT NOT NULL,
 subject_kind TEXT NOT NULL, subject_id TEXT NOT NULL, occurred_at TEXT NOT NULL,
 recorded_at TEXT NOT NULL, payload_json TEXT NOT NULL, content_hash TEXT NOT NULL,
 provenance_json TEXT NOT NULL, source_json TEXT,
 PRIMARY KEY(project_id,id),UNIQUE(project_id,ordinal)
);
CREATE INDEX delivery_event_subject ON delivery_observations(project_id,subject_kind,subject_id,ordinal);
CREATE TABLE delivery_records (
 project_id TEXT NOT NULL, kind TEXT NOT NULL,id TEXT NOT NULL,
 repository TEXT NOT NULL,title TEXT NOT NULL,state TEXT NOT NULL,updated_at TEXT NOT NULL,
 recipient_agent TEXT,recipient_session TEXT,payload_json TEXT NOT NULL,
 PRIMARY KEY(project_id,kind,id)
);
CREATE INDEX delivery_record_list ON delivery_records(project_id,kind,repository,state,id);
CREATE INDEX delivery_inbox ON delivery_records(project_id,recipient_agent,recipient_session,state,id);
CREATE TABLE delivery_origins (
 project_id TEXT NOT NULL,mapping_id TEXT NOT NULL,hunk_index INTEGER NOT NULL,
 contribution_id TEXT NOT NULL,repository TEXT NOT NULL,revision TEXT NOT NULL,
 path TEXT NOT NULL,fingerprint TEXT,new_start BIGINT NOT NULL,new_lines BIGINT NOT NULL,
 PRIMARY KEY(project_id,mapping_id,hunk_index)
);
CREATE INDEX delivery_origin_lookup ON delivery_origins(project_id,repository,revision,path,new_start);
CREATE INDEX delivery_origin_fingerprint ON delivery_origins(project_id,repository,fingerprint);
CREATE TABLE delivery_relations (
 project_id TEXT NOT NULL,source_kind TEXT NOT NULL,source_id TEXT NOT NULL,
 target_kind TEXT NOT NULL,target_id TEXT NOT NULL,
 PRIMARY KEY(project_id,source_kind,source_id,target_kind,target_id)
);
CREATE INDEX delivery_related_target ON delivery_relations(project_id,target_kind,target_id,source_kind,source_id);
CREATE TABLE delivery_features (
 project_id TEXT NOT NULL,contribution_id TEXT NOT NULL,feature TEXT NOT NULL,
 PRIMARY KEY(project_id,contribution_id,feature)
);
CREATE INDEX delivery_feature_lookup ON delivery_features(project_id,feature,contribution_id);
CREATE TABLE delivery_presence (
 project_id TEXT NOT NULL,agent TEXT NOT NULL,session_id TEXT NOT NULL,
 observed_at TEXT NOT NULL,payload_json TEXT NOT NULL,
 PRIMARY KEY(project_id,agent,session_id)
);
CREATE TABLE delivery_targets (
 project_id TEXT NOT NULL,kind TEXT NOT NULL,subject_id TEXT NOT NULL,target_kind TEXT NOT NULL,target TEXT NOT NULL,
 PRIMARY KEY(project_id,kind,subject_id,target_kind,target)
);
CREATE INDEX delivery_target_lookup ON delivery_targets(project_id,target,kind,subject_id);
CREATE UNIQUE INDEX delivery_provider_pr_identity ON delivery_targets(project_id,target) WHERE target_kind='provider_pr';
CREATE TABLE delivery_watches (
 project_id TEXT NOT NULL,id TEXT NOT NULL,pull_request_id TEXT NOT NULL,
 registration_json TEXT NOT NULL,actor_json TEXT NOT NULL,generation TEXT NOT NULL,
 enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),next_poll_at TEXT NOT NULL,
 last_attempt_at TEXT,last_success_at TEXT,last_error TEXT,last_digest TEXT,provider_updated_at TEXT,
 PRIMARY KEY(project_id,id)
);
CREATE INDEX delivery_watch_due ON delivery_watches(enabled,next_poll_at,project_id,id);
