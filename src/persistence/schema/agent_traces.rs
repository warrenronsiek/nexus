// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md

diesel::table! {
    trace_batches (id) {
        id -> Text,
        project_id -> Text,
        application_id -> Text,
        environment -> Text,
        batch_id -> Text,
        content_hash -> Text,
        received_at -> Text,
        accepted_events -> Integer,
        duplicate_events -> Integer,
        projected -> Bool,
    }
}

diesel::table! {
    trace_events (id) {
        id -> Text,
        batch_key -> Text,
        project_id -> Text,
        application_id -> Text,
        environment -> Text,
        trace_id -> Text,
        event_id -> Text,
        agent_id -> Text,
        conversation_id -> Nullable<Text>,
        occurred_at -> Text,
        source_sequence -> Nullable<BigInt>,
        event_json -> Text,
        payload_json -> Nullable<Text>,
        payload_bytes -> BigInt,
        payload_expired -> Bool,
        content_hash -> Text,
    }
}

diesel::table! {
    trace_invocations (id) {
        id -> Text,
        project_id -> Text,
        application_id -> Text,
        environment -> Text,
        trace_id -> Text,
        agent_id -> Text,
        invocation_id -> Text,
        kind -> Text,
        name -> Nullable<Text>,
        version -> Nullable<Text>,
        occurred_at -> Text,
        outcome -> Nullable<Text>,
        duration_ms -> Nullable<BigInt>,
        started_event_id -> Nullable<Text>,
        finished_event_id -> Nullable<Text>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(trace_batches, trace_events);
