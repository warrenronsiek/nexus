// @feature audit-registry
// @feature persistence
// @spec docs/features/audit-registry.md
// @spec docs/features/persistence.md
diesel::table! {
    audit_records (project_id, id) {
        project_id -> Text,
        id -> Text,
        ordinal -> BigInt,
        kind -> Text,
        control_key -> Text,
        control_version -> Integer,
        run_id -> Nullable<Text>,
        finding_id -> Nullable<Text>,
        occurred_at -> Text,
        recorded_at -> Text,
        actor_json -> Text,
        input_json -> Text,
    }
}
