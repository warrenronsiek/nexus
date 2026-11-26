// @feature agent-traces
// @spec docs/features/agent-traces.md

use super::{TraceCapability, TraceEvent, TraceOutcome};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, Serializer};

/// Read contracts for CLI, MCP, and browser clients. Variant fields retain
/// the existing response envelopes.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TraceReadResponse {
    List {
        traces: Vec<TraceSummary>,
        #[serde(flatten)]
        page: TracePage,
    },
    Events {
        events: Vec<TraceEventView>,
        #[serde(flatten)]
        page: TracePage,
    },
    Calls {
        calls: Vec<TraceInvocation>,
        #[serde(flatten)]
        page: TracePage,
    },
    Usage {
        usage: Vec<TraceUsage>,
        #[serde(flatten)]
        page: TracePage,
    },
    Status {
        ok: bool,
        pending_batches: i64,
        oldest_pending_at: Option<TraceTimestamp>,
        scopes: Vec<TraceApplicationScope>,
        scopes_truncated: bool,
    },
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TracePage {
    pub ok: bool,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

impl TracePage {
    pub(crate) fn new(next_cursor: Option<String>) -> Self {
        Self {
            ok: true,
            truncated: next_cursor.is_some(),
            next_cursor,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TraceTimestamp(#[serde(serialize_with = "serialize_time")] DateTime<Utc>);

impl From<DateTime<Utc>> for TraceTimestamp {
    fn from(value: DateTime<Utc>) -> Self {
        Self(value)
    }
}

fn serialize_time<S: Serializer>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_rfc3339_opts(SecondsFormat::Nanos, true))
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TraceSummary {
    pub application_id: String,
    pub environment: String,
    pub trace_id: String,
    pub occurred_at: TraceTimestamp,
    pub agent_id: Option<String>,
    pub conversation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TracePayloadState {
    Available,
    NotCaptured,
    Expired,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct TraceEventView {
    #[serde(flatten)]
    pub event: TraceEvent,
    pub application_id: String,
    pub environment: String,
    pub has_payload: bool,
    pub payload_state: TracePayloadState,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TraceInvocation {
    pub application_id: String,
    pub environment: String,
    pub trace_id: String,
    pub agent_id: String,
    pub invocation_id: String,
    pub kind: TraceCapability,
    pub name: Option<String>,
    pub version: Option<String>,
    pub occurred_at: TraceTimestamp,
    pub outcome: Option<TraceOutcome>,
    pub duration_ms: Option<i64>,
    pub started_event_id: Option<String>,
    pub finished_event_id: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TraceUsage {
    pub application_id: String,
    pub environment: String,
    pub agent_id: String,
    pub kind: TraceCapability,
    pub name: Option<String>,
    pub version: Option<String>,
    pub calls: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub incomplete: i64,
    pub canceled: i64,
    pub unknown: i64,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TraceApplicationScope {
    pub application_id: String,
    pub environment: String,
    pub agent_id: String,
}
