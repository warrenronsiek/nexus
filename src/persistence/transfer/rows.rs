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
use super::super::schema::{
    advisories, analytics_actions, analytics_calls, analytics_categories, analytics_exposures,
    analytics_resources, analytics_revisions, capability_uses, claims, conflicts, events,
    memory_activations, memory_compaction_attempts, memory_compaction_queue, memory_entries,
    memory_frontier_snapshots, memory_spaces, memory_summaries, papercut_duplicate_reviews,
    papercut_review_attempts, papercut_upvotes, papercuts, sessions, sos_issue_lifecycle,
    sos_issues, sos_reports, sos_triage_attempts,
};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

macro_rules! portable_row {
    ($name:ident, $table:ident, { $($field:ident: $kind:ty),+ $(,)? }) => {
        #[derive(Deserialize, Insertable, Queryable, Selectable, Serialize)]
        #[diesel(table_name = $table)]
        pub(super) struct $name {
            $(pub(super) $field: $kind),+
        }
    };
}

portable_row!(AdvisoryTransfer, advisories, {
    id: String,
    project_id: String,
    session_id: String,
    tool_use_id: String,
    conflict_id: Option<String>,
    severity: String,
    kind: String,
    path: String,
    message: String,
    created_at: String,
});

portable_row!(CapabilityUseTransfer, capability_uses, {
    id: String,
    project_id: String,
    agent: String,
    session_id: String,
    turn_id: Option<String>,
    invocation_id: String,
    parent_invocation_id: Option<String>,
    kind: String,
    name: String,
    source: String,
    evidence: String,
    outcome: String,
    config_hash: String,
    model_id: Option<String>,
    first_observed_at: String,
    completed_at: Option<String>,
});

portable_row!(ClaimTransfer, claims, {
    id: String,
    project_id: String,
    session_id: String,
    tool_use_id: String,
    path: String,
    operation: String,
    line_start: i32,
    line_end: i32,
    state: String,
    expires_at: String,
    updated_at: String,
});

portable_row!(ConflictTransfer, conflicts, {
    id: String,
    project_id: String,
    left_claim_id: String,
    right_claim_id: String,
    path: String,
    severity: String,
    kind: String,
    status: String,
    message: String,
    created_at: String,
    updated_at: String,
});

portable_row!(EventTransfer, events, {
    id: i64,
    project_id: String,
    session_id: Option<String>,
    kind: String,
    payload_json: String,
    created_at: String,
});

portable_row!(MemoryActivationTransfer, memory_activations, {
    id: String,
    agent: String,
    session_id: String,
    read_scope: String,
    project_id: Option<String>,
    activation_scope_key: String,
    delivered_items: i32,
    omitted_ranges: i32,
    activated_at: String,
});

portable_row!(MemoryCompactionAttemptTransfer, memory_compaction_attempts, {
    id: String,
    space_id: String,
    level: i32,
    start_ordinal: i64,
    end_ordinal: i64,
    source_hash: String,
    provider: String,
    model_id: Option<String>,
    outcome: String,
    diagnostic: Option<String>,
    was_fallback: bool,
    consecutive_failures: i32,
    attempted_at: String,
    next_retry_at: Option<String>,
});

portable_row!(MemoryCompactionQueueTransfer, memory_compaction_queue, {
    space_id: String,
    level: i32,
    start_ordinal: i64,
    end_ordinal: i64,
    source_hash: String,
    codex_retry_at: Option<String>,
    claude_retry_at: Option<String>,
    queued_at: String,
});

portable_row!(MemoryEntryTransfer, memory_entries, {
    id: String,
    space_id: String,
    ordinal: i64,
    content: String,
    content_hash: String,
    agent: String,
    session_id: String,
    model_id: Option<String>,
    config_hash: String,
    created_at: String,
});

portable_row!(MemoryFrontierTransfer, memory_frontier_snapshots, {
    space_id: String,
    revision: i64,
    node_refs_json: String,
    omissions_json: String,
    item_count: i32,
    omission_count: i32,
    byte_count: i32,
    built_at: String,
});

portable_row!(MemorySpaceTransfer, memory_spaces, {
    id: String,
    scope: String,
    project_id: Option<String>,
    next_ordinal: i64,
    revision: i64,
    created_at: String,
    updated_at: String,
});

portable_row!(MemorySummaryTransfer, memory_summaries, {
    id: String,
    space_id: String,
    level: i32,
    start_ordinal: i64,
    end_ordinal: i64,
    content: String,
    content_hash: String,
    source_hash: String,
    provider: String,
    model_id: Option<String>,
    created_at: String,
});

portable_row!(PapercutTransfer, papercuts, {
    id: String,
    project_id: String,
    title: String,
    description: String,
    fingerprint: String,
    reporter_token_hash: String,
    status: String,
    consolidated_into: Option<String>,
    resolution: Option<String>,
    creator_agent: String,
    creator_session_id: String,
    creator_model_id: Option<String>,
    creator_config_hash: String,
    created_at: String,
    updated_at: String,
    resolved_at: Option<String>,
});

portable_row!(PapercutUpvoteTransfer, papercut_upvotes, {
    id: String,
    project_id: String,
    papercut_id: String,
    reported_papercut_id: String,
    reporter_agent: String,
    reporter_session_id: String,
    reporter_model_id: Option<String>,
    reporter_config_hash: String,
    note: Option<String>,
    created_at: String,
});

portable_row!(PapercutDuplicateReviewTransfer, papercut_duplicate_reviews, {
    id: String,
    project_id: String,
    papercut_id: String,
    canonical_id: String,
    reporter_session_id: String,
    rationale: String,
    question: String,
    response: Option<String>,
    status: String,
    reviewer_agent: String,
    reviewer_session_id: String,
    provider: Option<String>,
    model_id: Option<String>,
    created_at: String,
    responded_at: Option<String>,
});

portable_row!(PapercutReviewAttemptTransfer, papercut_review_attempts, {
    id: String,
    project_id: String,
    papercut_id: String,
    outcome: String,
    provider: String,
    model_id: Option<String>,
    detail: String,
    config_hash: String,
    created_at: String,
});

portable_row!(SessionTransfer, sessions, {
    project_id: String,
    session_id: String,
    agent: String,
    worktree: Option<String>,
    task_summary: Option<String>,
    prompt_hash: Option<String>,
    status: String,
    config_hash: String,
    started_at: String,
    last_seen_at: String,
});

portable_row!(SosIssueTransfer, sos_issues, {
    id: String,
    project_id: String,
    title: String,
    summary: String,
    status: String,
    resolution: Option<String>,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
});

portable_row!(SosReportTransfer, sos_reports, {
    id: String,
    project_id: String,
    issue_id: Option<String>,
    message: String,
    report_type: String,
    environment: String,
    version: String,
    model: String,
    trace: String,
    metadata_json: String,
    reporter_agent: String,
    reporter_session_id: String,
    reporter_model_id: Option<String>,
    reporter_config_hash: String,
    created_at: String,
});

portable_row!(SosTriageAttemptTransfer, sos_triage_attempts, {
    id: String,
    project_id: String,
    report_id: String,
    issue_id: Option<String>,
    outcome: String,
    provider: String,
    model_id: Option<String>,
    rationale: String,
    generated_title: Option<String>,
    generated_summary: Option<String>,
    config_hash: String,
    created_at: String,
});

portable_row!(SosIssueLifecycleTransfer, sos_issue_lifecycle, {
    id: String,
    project_id: String,
    issue_id: String,
    transition: String,
    reason: String,
    actor_agent: String,
    actor_session_id: String,
    actor_model_id: Option<String>,
    actor_config_hash: String,
    created_at: String,
});

portable_row!(AnalyticsCallsTransfer, analytics_calls, {
    id: String,
    project_id: String,
    agent: String,
    session_key: String,
    invocation_id: String,
    parent_id: Option<String>,
    tool_name: String,
    outcome: String,
    observed_at: String,
    completed_at: Option<String>,
});

portable_row!(AnalyticsActionsTransfer, analytics_actions, {
    id: String,
    call_id: String,
    project_id: String,
    session_key: String,
    resource_key: String,
    name: String,
    kind: String,
    origin: String,
    category_id: String,
    description: String,
    pattern_key: String,
    resource_version: Option<String>,
    evidence: String,
    model_id: Option<String>,
    observed_at: String,
});

portable_row!(AnalyticsExposuresTransfer, analytics_exposures, {
    id: String,
    project_id: String,
    session_key: String,
    scope: String,
    resource_key: String,
    name: String,
    kind: String,
    origin: String,
    exposure: String,
    version: Option<String>,
    description: String,
    activities_json: String,
    first_seen: String,
    last_seen: String,
    active: bool,
});

portable_row!(AnalyticsCategoriesTransfer, analytics_categories, {
    id: String,
    name: String,
    definition: String,
    replaced_by: Option<String>,
    created_at: String,
    updated_at: String,
});

portable_row!(AnalyticsRevisionsTransfer, analytics_revisions, {
    id: String,
    action_id: String,
    category_id: String,
    description: String,
    evidence: String,
    model_id: Option<String>,
    policy_version: String,
    created_at: String,
});

portable_row!(AnalyticsResourcesTransfer, analytics_resources, {
    id: String,
    project_id: String,
    resource_key: String,
    name: String,
    kind: String,
    origin: String,
    description: String,
    activities_json: String,
    version: Option<String>,
    first_seen: String,
    last_seen: String,
});
