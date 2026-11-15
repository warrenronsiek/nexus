// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
// Delivery coordination projection and immutable source journal.
diesel::table! {
 delivery_observations (project_id,id) {
 project_id -> Text,id -> Text,ordinal -> BigInt,subject_kind -> Text,subject_id -> Text,
 occurred_at -> Text,recorded_at -> Text,payload_json -> Text,content_hash -> Text,provenance_json -> Text,source_json -> Nullable<Text>,
 }
}
diesel::table! {
 delivery_records (project_id,kind,id) {
 project_id -> Text,kind -> Text,id -> Text,repository -> Text,title -> Text,state -> Text,updated_at -> Text,
 recipient_agent -> Nullable<Text>,recipient_session -> Nullable<Text>,payload_json -> Text,
 }
}
diesel::table! {
 delivery_origins (project_id,contribution_id,mapping_id,hunk_index) {
 project_id -> Text,mapping_id -> Text,hunk_index -> Integer,contribution_id -> Text,repository -> Text,revision -> Text,
 path -> Text,fingerprint -> Nullable<Text>,new_start -> BigInt,new_lines -> BigInt,
 }
}
diesel::table! {
 delivery_relations (project_id,source_kind,source_id,target_kind,target_id) {
 project_id -> Text,source_kind -> Text,source_id -> Text,target_kind -> Text,target_id -> Text,
 }
}
diesel::table! {
 delivery_features (project_id,contribution_id,feature) {project_id -> Text,contribution_id -> Text,feature -> Text,}
}
diesel::table! {
 delivery_presence (project_id,agent,session_id) {project_id -> Text,agent -> Text,session_id -> Text,observed_at -> Text,payload_json -> Text,}
}
diesel::allow_tables_to_appear_in_same_query!(
    delivery_records,
    delivery_features,
    delivery_relations
);

diesel::table! {
 delivery_targets (project_id,kind,subject_id,target_kind,target) {
 project_id -> Text,kind -> Text,subject_id -> Text,target_kind -> Text,target -> Text,
 }
}
diesel::allow_tables_to_appear_in_same_query!(delivery_records, delivery_targets);

diesel::table! {
 delivery_watches (project_id,id) {
 project_id -> Text,id -> Text,pull_request_id -> Text,registration_json -> Text,actor_json -> Text,generation -> Text,enabled -> Integer,next_poll_at -> Text,last_attempt_at -> Nullable<Text>,last_success_at -> Nullable<Text>,last_error -> Nullable<Text>,last_digest -> Nullable<Text>,provider_updated_at -> Nullable<Text>,
 }
}
