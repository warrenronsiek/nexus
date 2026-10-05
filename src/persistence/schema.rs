// @feature persistence
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
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
    memory_activations (id) {
        id -> Text,
        agent -> Text,
        session_id -> Text,
        read_scope -> Text,
        project_id -> Nullable<Text>,
        delivered_items -> Integer,
        omitted_ranges -> Integer,
        activated_at -> Text,
    }
}

diesel::table! {
    memory_compaction_attempts (id) {
        id -> Text,
        space_id -> Text,
        level -> Integer,
        start_ordinal -> BigInt,
        end_ordinal -> BigInt,
        source_hash -> Text,
        provider -> Text,
        model_id -> Nullable<Text>,
        outcome -> Text,
        diagnostic -> Nullable<Text>,
        was_fallback -> Bool,
        consecutive_failures -> Integer,
        attempted_at -> Text,
        next_retry_at -> Nullable<Text>,
    }
}

diesel::table! {
    memory_entries (id) {
        id -> Text,
        space_id -> Text,
        ordinal -> BigInt,
        content -> Text,
        content_hash -> Text,
        agent -> Text,
        session_id -> Text,
        model_id -> Nullable<Text>,
        config_hash -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    memory_frontier_snapshots (space_id) {
        space_id -> Text,
        revision -> BigInt,
        node_refs_json -> Text,
        omissions_json -> Text,
        item_count -> Integer,
        omission_count -> Integer,
        byte_count -> Integer,
        built_at -> Text,
    }
}

diesel::table! {
    memory_compaction_queue (space_id, level, start_ordinal, end_ordinal) {
        space_id -> Text,
        level -> Integer,
        start_ordinal -> BigInt,
        end_ordinal -> BigInt,
        source_hash -> Text,
        codex_retry_at -> Nullable<Text>,
        claude_retry_at -> Nullable<Text>,
        queued_at -> Text,
    }
}

diesel::table! {
    memory_spaces (id) {
        id -> Text,
        scope -> Text,
        project_id -> Nullable<Text>,
        next_ordinal -> BigInt,
        revision -> BigInt,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    memory_summaries (id) {
        id -> Text,
        space_id -> Text,
        level -> Integer,
        start_ordinal -> BigInt,
        end_ordinal -> BigInt,
        content -> Text,
        content_hash -> Text,
        source_hash -> Text,
        provider -> Text,
        model_id -> Nullable<Text>,
        created_at -> Text,
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
    memory_activations,
    memory_compaction_attempts,
    memory_compaction_queue,
    memory_entries,
    memory_frontier_snapshots,
    memory_spaces,
    memory_summaries,
    sessions
);
