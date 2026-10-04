// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
diesel::table! {
    advisories (id) {
        id -> Text,
        project_id -> Text,
        session_id -> Text,
        tool_use_id -> Text,
        conflict_id -> Nullable<Text>,
        severity -> Text,
        kind -> Text,
        path -> Text,
        message -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    capability_uses (id) {
        id -> Text,
        project_id -> Text,
        agent -> Text,
        session_id -> Text,
        turn_id -> Nullable<Text>,
        invocation_id -> Text,
        parent_invocation_id -> Nullable<Text>,
        kind -> Text,
        name -> Text,
        source -> Text,
        evidence -> Text,
        outcome -> Text,
        config_hash -> Text,
        model_id -> Nullable<Text>,
        first_observed_at -> Text,
        completed_at -> Nullable<Text>,
    }
}

diesel::table! {
    claims (id) {
        id -> Text,
        project_id -> Text,
        session_id -> Text,
        tool_use_id -> Text,
        path -> Text,
        operation -> Text,
        line_start -> Integer,
        line_end -> Integer,
        state -> Text,
        expires_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    conflicts (id) {
        id -> Text,
        project_id -> Text,
        left_claim_id -> Text,
        right_claim_id -> Text,
        path -> Text,
        severity -> Text,
        kind -> Text,
        status -> Text,
        message -> Text,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    events (id) {
        id -> BigInt,
        project_id -> Text,
        session_id -> Nullable<Text>,
        kind -> Text,
        payload_json -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    flyway_migrations (version) {
        version -> BigInt,
        name -> Text,
        checksum -> Text,
        status -> Text,
    }
}

diesel::table! {
    sessions (session_id) {
        session_id -> Text,
        project_id -> Text,
        agent -> Text,
        worktree -> Nullable<Text>,
        task_summary -> Nullable<Text>,
        prompt_hash -> Nullable<Text>,
        status -> Text,
        config_hash -> Text,
        started_at -> Text,
        last_seen_at -> Text,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    advisories,
    capability_uses,
    claims,
    conflicts,
    events,
    flyway_migrations,
    sessions
);
