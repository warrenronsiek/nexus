-- @feature persistence
-- @feature delivery-coordination
-- @spec docs/features/persistence.md
-- @spec docs/features/delivery-coordination.md
INSERT INTO delivery_targets(project_id,kind,subject_id,target_kind,target)
SELECT delivery_records.project_id,delivery_records.kind,delivery_records.id,'revision',lower(json_extract(payload_json,'$.body.contribution.head_revision.commit'))
FROM delivery_records
WHERE delivery_records.kind='contribution' AND json_extract(payload_json,'$.body.contribution.head_revision.commit') IS NOT NULL
ON CONFLICT DO NOTHING;

INSERT INTO delivery_targets(project_id,kind,subject_id,target_kind,target)
SELECT delivery_records.project_id,delivery_records.kind,delivery_records.id,'revision',lower(json_extract(mapping.value,'$.revision.commit'))
FROM delivery_records, json_each(payload_json,'$.body.mappings') AS mapping
WHERE delivery_records.kind='contribution'
ON CONFLICT DO NOTHING;

CREATE INDEX delivery_history_by_time ON delivery_observations(project_id,subject_kind,occurred_at,ordinal);
UPDATE delivery_origins SET revision=lower(revision) WHERE revision<>lower(revision);
