// @feature agent-traces
// @spec docs/features/agent-traces.md
// @entrypoint TraceBatch::validate
// @boundary dynamic-json

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::str::FromStr;

pub const MAX_BATCH_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_EVENTS: usize = 512;
pub const MAX_PAGE_ITEMS: usize = 100;
const MAX_EVENT_PAYLOAD_BYTES: usize = 512 * 1024;
const MAX_LABEL_BYTES: usize = 256;

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }

        impl FromStr for $name {
            type Err = anyhow::Error;

            fn from_str(value: &str) -> Result<Self> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    _ => bail!("invalid {}", stringify!($name)),
                }
            }
        }
    };
}

wire_enum!(TraceEventKind {
    AgentStarted => "agent_started",
    AgentFinished => "agent_finished",
    Message => "message",
    Prompt => "prompt",
    ModelStarted => "model_started",
    ModelFinished => "model_finished",
    ToolStarted => "tool_started",
    ToolFinished => "tool_finished",
    SkillOffered => "skill_offered",
    SkillLoaded => "skill_loaded",
    SkillStarted => "skill_started",
    SkillFinished => "skill_finished",
});

wire_enum!(TraceOutcome {
    Succeeded => "succeeded",
    Failed => "failed",
    Canceled => "canceled",
    ValidationFailed => "validation_failed",
    Rejected => "rejected",
    TimedOut => "timed_out",
    Unknown => "unknown",
});

wire_enum!(TraceCapability { Tool => "tool", Skill => "skill" });

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEvent {
    pub event_id: String,
    pub trace_id: String,
    pub agent_id: String,
    pub kind: TraceEventKind,
    pub occurred_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TraceOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceBatch {
    pub schema_version: u32,
    pub batch_id: String,
    pub application_id: String,
    pub environment: String,
    pub events: Vec<TraceEvent>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
pub struct TraceReceipt {
    pub batch_id: String,
    pub accepted_events: usize,
    pub duplicate_events: usize,
    pub received_at: String,
}

#[derive(Debug, thiserror::Error)]
#[error("trace {0} was already accepted with different content")]
pub struct TraceConflict(pub &'static str);

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceReference {
    pub application_id: String,
    pub environment: String,
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraceFilter {
    pub application_id: Option<String>,
    pub environment: Option<String>,
    pub agent_id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub unversioned: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub cursor: Option<String>,
    pub kind: Option<TraceCapability>,
    pub outcome: Option<TraceOutcome>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TraceQuery {
    List {
        #[serde(default)]
        filter: TraceFilter,
    },
    Get {
        application_id: String,
        environment: String,
        trace_id: String,
        event_id: Option<String>,
        #[serde(default)]
        include_payload: bool,
        limit: Option<usize>,
        cursor: Option<String>,
    },
    Calls {
        #[serde(default)]
        filter: TraceFilter,
    },
    Usage {
        #[serde(default)]
        filter: TraceFilter,
    },
    Status {
        application_id: Option<String>,
        environment: Option<String>,
    },
}

impl TraceBatch {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported trace schema_version");
        validate_text(&self.batch_id, "batch_id")?;
        validate_scope(&self.application_id, &self.environment)?;
        ensure!(
            (1..=MAX_BATCH_EVENTS).contains(&self.events.len()),
            "trace batch must have 1 to {MAX_BATCH_EVENTS} events"
        );
        let mut ids = HashSet::with_capacity(self.events.len());
        for event in &self.events {
            event.validate()?;
            ensure!(
                ids.insert(&event.event_id),
                "duplicate event_id in trace batch"
            );
        }
        Ok(())
    }
}

impl TraceEvent {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_text(&self.event_id, "event_id")?;
        validate_text(&self.trace_id, "trace_id")?;
        validate_text(&self.agent_id, "agent_id")?;
        validate_optional_texts([
            (&self.conversation_id, "conversation_id"),
            (&self.turn_id, "turn_id"),
            (&self.invocation_id, "invocation_id"),
            (&self.name, "name"),
            (&self.version, "version"),
            (&self.parent_event_id, "parent_event_id"),
        ])?;
        canonical_time(&self.occurred_at).context("invalid occurred_at")?;
        ensure!(
            self.sequence.unwrap_or(0) >= 0,
            "sequence must be nonnegative"
        );
        ensure!(
            self.duration_ms.unwrap_or(0) >= 0,
            "duration_ms must be nonnegative"
        );
        match self.kind {
            TraceEventKind::ToolStarted
            | TraceEventKind::ToolFinished
            | TraceEventKind::SkillStarted
            | TraceEventKind::SkillFinished => {
                ensure!(
                    self.invocation_id.is_some() && self.name.is_some(),
                    "tool and skill invocations require invocation_id and name"
                );
            }
            TraceEventKind::SkillOffered | TraceEventKind::SkillLoaded => {
                ensure!(self.name.is_some(), "skill observations require name");
            }
            _ => {}
        }
        if let Some(payload) = &self.payload {
            ensure!(
                serde_json::to_vec(payload)?.len() <= MAX_EVENT_PAYLOAD_BYTES,
                "event payload exceeds {MAX_EVENT_PAYLOAD_BYTES} encoded bytes"
            );
        }
        Ok(())
    }
}

impl TraceReference {
    pub fn validate(&self) -> Result<()> {
        validate_scope(&self.application_id, &self.environment)?;
        validate_text(&self.trace_id, "trace_id")?;
        validate_optional_texts([(&self.event_id, "event_id")])
    }
}

impl TraceQuery {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::List { filter } | Self::Calls { filter } | Self::Usage { filter } => {
                filter.validate()
            }
            Self::Get {
                application_id,
                environment,
                trace_id,
                event_id,
                limit,
                cursor,
                ..
            } => {
                validate_scope(application_id, environment)?;
                validate_text(trace_id, "trace_id")?;
                validate_optional_texts([(event_id, "event_id")])?;
                validate_page(*limit, cursor)
            }
            Self::Status {
                application_id,
                environment,
            } => validate_optional_texts([
                (application_id, "application_id"),
                (environment, "environment"),
            ]),
        }
    }
}

impl TraceFilter {
    fn validate(&self) -> Result<()> {
        validate_optional_texts([
            (&self.application_id, "application_id"),
            (&self.environment, "environment"),
            (&self.agent_id, "agent_id"),
            (&self.name, "name"),
            (&self.version, "version"),
        ])?;
        ensure!(
            !self.unversioned || self.version.is_none(),
            "version and unversioned are mutually exclusive"
        );
        let since = self
            .since
            .as_deref()
            .map(canonical_time)
            .transpose()
            .context("invalid since")?;
        let until = self
            .until
            .as_deref()
            .map(canonical_time)
            .transpose()
            .context("invalid until")?;
        if let (Some(since), Some(until)) = (&since, &until) {
            ensure!(since < until, "since must precede until");
        }
        validate_page(self.limit, &self.cursor)
    }
}

pub fn canonical_time(value: &str) -> Result<String> {
    Ok(DateTime::parse_from_rfc3339(value)?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Nanos, true))
}

pub(crate) fn validate_scope(application_id: &str, environment: &str) -> Result<()> {
    validate_text(application_id, "application_id")?;
    validate_text(environment, "environment")
}

fn validate_text(value: &str, field: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_LABEL_BYTES && value.trim() == value
            && !value.chars().any(char::is_control),
        "{field} must contain 1 to {MAX_LABEL_BYTES} bytes without surrounding whitespace or control characters"
    );
    Ok(())
}

fn validate_optional_texts<const N: usize>(values: [(&Option<String>, &str); N]) -> Result<()> {
    for (value, field) in values {
        if let Some(value) = value {
            validate_text(value, field)?;
        }
    }
    Ok(())
}

fn validate_page(limit: Option<usize>, cursor: &Option<String>) -> Result<()> {
    if let Some(limit) = limit {
        ensure!(
            (1..=MAX_PAGE_ITEMS).contains(&limit),
            "limit must be between 1 and {MAX_PAGE_ITEMS}"
        );
    }
    if let Some(cursor) = cursor {
        ensure!(
            cursor.len() <= 4096 && !cursor.is_empty(),
            "invalid trace cursor length"
        );
    }
    Ok(())
}
