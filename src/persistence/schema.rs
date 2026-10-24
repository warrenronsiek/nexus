// @feature persistence
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @feature evals
// @spec docs/features/evals.md
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
    database_imports (source_digest) {
        source_digest -> Text,
        source_kind -> Text,
        format_version -> Integer,
        row_count -> BigInt,
        imported_at -> Text,
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
        activation_scope_key -> Text,
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
    papercut_duplicate_reviews (id) {
        id -> Text,
        project_id -> Text,
        papercut_id -> Text,
        canonical_id -> Text,
        reporter_session_id -> Text,
        rationale -> Text,
        question -> Text,
        response -> Nullable<Text>,
        status -> Text,
        reviewer_agent -> Text,
        reviewer_session_id -> Text,
        provider -> Nullable<Text>,
        model_id -> Nullable<Text>,
        created_at -> Text,
        responded_at -> Nullable<Text>,
    }
}

diesel::table! {
    papercut_review_attempts (id) {
        id -> Text,
        project_id -> Text,
        papercut_id -> Text,
        outcome -> Text,
        provider -> Text,
        model_id -> Nullable<Text>,
        detail -> Text,
        config_hash -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    papercut_upvotes (id) {
        id -> Text,
        project_id -> Text,
        papercut_id -> Text,
        reported_papercut_id -> Text,
        reporter_agent -> Text,
        reporter_session_id -> Text,
        reporter_model_id -> Nullable<Text>,
        reporter_config_hash -> Text,
        note -> Nullable<Text>,
        created_at -> Text,
    }
}

diesel::table! {
    papercuts (id) {
        id -> Text,
        project_id -> Text,
        title -> Text,
        description -> Text,
        fingerprint -> Text,
        reporter_token_hash -> Text,
        status -> Text,
        consolidated_into -> Nullable<Text>,
        resolution -> Nullable<Text>,
        creator_agent -> Text,
        creator_session_id -> Text,
        creator_model_id -> Nullable<Text>,
        creator_config_hash -> Text,
        created_at -> Text,
        updated_at -> Text,
        resolved_at -> Nullable<Text>,
    }
}

diesel::table! {
    sessions (project_id, session_id) {
        project_id -> Text,
        session_id -> Text,
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

diesel::table! {
    sos_issue_lifecycle (id) {
        id -> Text,
        project_id -> Text,
        issue_id -> Text,
        transition -> Text,
        reason -> Text,
        actor_agent -> Text,
        actor_session_id -> Text,
        actor_model_id -> Nullable<Text>,
        actor_config_hash -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    sos_issues (id) {
        id -> Text,
        project_id -> Text,
        title -> Text,
        summary -> Text,
        status -> Text,
        resolution -> Nullable<Text>,
        created_at -> Text,
        updated_at -> Text,
        closed_at -> Nullable<Text>,
    }
}

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
    }
}

diesel::table! {
    sos_triage_attempts (id) {
        id -> Text,
        project_id -> Text,
        report_id -> Text,
        issue_id -> Nullable<Text>,
        outcome -> Text,
        provider -> Text,
        model_id -> Nullable<Text>,
        rationale -> Text,
        generated_title -> Nullable<Text>,
        generated_summary -> Nullable<Text>,
        config_hash -> Text,
        created_at -> Text,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    advisories,
    capability_uses,
    database_imports,
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
    papercut_duplicate_reviews,
    papercut_review_attempts,
    papercut_upvotes,
    papercuts,
    sessions,
    sos_issue_lifecycle,
    sos_issues,
    sos_reports,
    sos_triage_attempts
);

diesel::table! {
    analytics_calls (id) {
        id -> Text,
        project_id -> Text,
        agent -> Text,
        session_key -> Text,
        invocation_id -> Text,
        parent_id -> Nullable<Text>,
        tool_name -> Text,
        outcome -> Text,
        observed_at -> Text,
        completed_at -> Nullable<Text>,
    }
}
diesel::table! {
    analytics_actions (id) {
        id -> Text,
        call_id -> Text,
        project_id -> Text,
        session_key -> Text,
        resource_key -> Text,
        name -> Text,
        kind -> Text,
        origin -> Text,
        category_id -> Text,
        description -> Text,
        pattern_key -> Text,
        resource_version -> Nullable<Text>,
        evidence -> Text,
        model_id -> Nullable<Text>,
        observed_at -> Text,
    }
}
diesel::table! {
    analytics_exposures (id) {
        id -> Text,
        project_id -> Text,
        session_key -> Text,
        scope -> Text,
        resource_key -> Text,
        name -> Text,
        kind -> Text,
        origin -> Text,
        exposure -> Text,
        version -> Nullable<Text>,
        description -> Text,
        activities_json -> Text,
        first_seen -> Text,
        last_seen -> Text,
        active -> Bool,
    }
}
diesel::table! {
    analytics_categories (id) {
        id -> Text,
        name -> Text,
        definition -> Text,
        replaced_by -> Nullable<Text>,
        created_at -> Text,
        updated_at -> Text,
    }
}
diesel::table! {
    analytics_revisions (id) {
        id -> Text,
        action_id -> Text,
        category_id -> Text,
        description -> Text,
        evidence -> Text,
        model_id -> Nullable<Text>,
        policy_version -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    analytics_resources (id) {
        id -> Text, project_id -> Text, resource_key -> Text, name -> Text, kind -> Text,
        origin -> Text, description -> Text, activities_json -> Text, version -> Nullable<Text>, first_seen -> Text, last_seen -> Text,
    }
}
diesel::allow_tables_to_appear_in_same_query!(
    analytics_calls,
    analytics_actions,
    analytics_exposures,
    analytics_categories,
    analytics_revisions,
    analytics_resources
);

diesel::table! {
    eval_records (project_id, id) {
        project_id -> Text,
        id -> Text,
        kind -> Text,
        title -> Text,
        occurred_at -> Text,
        recorded_at -> Text,
        payload_json -> Text,
        content_hash -> Text,
        provenance_json -> Text,
        search_text -> Text,
    }
}
diesel::table! {
    eval_links (project_id, source_id, target_id, relation) {
        project_id -> Text,
        source_id -> Text,
        target_id -> Text,
        relation -> Text,
    }
}
diesel::table! {
    eval_dimensions (project_id, record_id, dimension, value) {
        project_id -> Text,
        record_id -> Text,
        dimension -> Text,
        value -> Text,
    }
}
diesel::table! {
    eval_blobs (project_id, digest) {
        project_id -> Text,
        digest -> Text,
        upload_id -> Text,
        byte_size -> BigInt,
        chunk_count -> Integer,
        verified -> Bool,
        created_at -> Text,
    }
}
diesel::table! {
    eval_blob_chunks (project_id, digest, chunk_index) {
        project_id -> Text,
        digest -> Text,
        chunk_index -> Integer,
        bytes -> Binary,
    }
}
diesel::allow_tables_to_appear_in_same_query!(
    eval_records,
    eval_links,
    eval_dimensions,
    eval_blobs,
    eval_blob_chunks
);

diesel::table! {
    work_observations (project_id, id) {
        project_id -> Text, id -> Text, subject_scope -> Text, subject_id -> Text,
        kind -> Text, occurred_at -> Text, recorded_at -> Text, payload_json -> Text,
        content_hash -> Text, provenance_json -> Text, source_json -> Nullable<Text>,
    }
}
diesel::table! {
    work_subjects (project_id, scope, id) {
        project_id -> Text, scope -> Text, id -> Text, title -> Text, kind -> Text,
        repository -> Nullable<Text>, host_session_id -> Nullable<Text>, environment -> Nullable<Text>,
        outcome -> Nullable<Text>, occurred_at -> Text, recorded_at -> Text,
    }
}
diesel::table! {
    work_links (project_id, event_id, relation, target_scope, target_id) {
        project_id -> Text, event_id -> Text, source_scope -> Text, source_id -> Text,
        relation -> Text, target_scope -> Text, target_id -> Text, target_json -> Text,
    }
}
diesel::allow_tables_to_appear_in_same_query!(
    work_observations,
    work_subjects,
    work_links,
    eval_records
);
