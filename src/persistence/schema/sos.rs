// @feature persistence
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/sos.md
diesel::table! {
    sos_reports (id) {
        id -> Text,
        project_id -> Text,
        issue_id -> Nullable<Text>,
        message -> Text,
        report_type -> Text,
        environment -> Text,
        version -> Text,
        model -> Text,
        trace -> Text,
        metadata_json -> Text,
        reporter_agent -> Text,
        reporter_session_id -> Text,
        reporter_model_id -> Nullable<Text>,
        reporter_config_hash -> Text,
        created_at -> Text,
        trace_ref_json -> Nullable<Text>,
    }
}
