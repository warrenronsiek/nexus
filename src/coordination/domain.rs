// @feature coordination
// @spec docs/features/coordination.md
// @boundary dynamic-json
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! string_enum {
    (display $type:ty, $kind:literal, {$($variant:ident => $value:literal),+ $(,)?}) => {
        impl std::fmt::Display for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(match self {
                    $(Self::$variant => $value),+
                })
            }
        }

        string_enum!(parse $type, $kind, {$($variant => $value),+});
    };
    (parse $type:ty, $kind:literal, {$($variant:ident => $value:literal),+ $(,)?}) => {
        impl std::str::FromStr for $type {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {} {other}", $kind)),
                }
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ToolPayload(Value);

impl ToolPayload {
    pub fn as_json(&self) -> &Value {
        &self.0
    }
}

impl From<Value> for ToolPayload {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub session_id: String,
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default = "default_agent")]
    pub agent: String,
}

fn default_agent() -> String {
    "unknown".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPromptInput {
    #[serde(flatten)]
    pub context: HookContext,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHookInput {
    #[serde(flatten)]
    pub context: HookContext,
    pub tool_use_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: ToolPayload,
    #[serde(default)]
    pub tool_output: Option<ToolPayload>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStopInput {
    #[serde(flatten)]
    pub context: HookContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Write,
    Delete,
    Rename,
}

impl Operation {
    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Delete | Self::Rename)
    }
}

string_enum!(display Operation, "operation", {
    Write => "write",
    Delete => "delete",
    Rename => "rename",
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathIntent {
    pub path: String,
    pub operation: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub project_id: String,
    pub session_id: String,
    pub tool_use_id: String,
    pub path: String,
    pub operation: Operation,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub state: ClaimState,
    pub expires_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    Claimed,
    Modified,
    Released,
}

string_enum!(display ClaimState, "claim state", {
    Claimed => "claimed",
    Modified => "modified",
    Released => "released",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

string_enum!(display Severity, "severity", {
    Info => "info",
    Warning => "warning",
    Critical => "critical",
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advisory {
    pub id: String,
    pub severity: Severity,
    pub kind: String,
    pub message: String,
    pub path: String,
    pub other_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResponse {
    /// This is deliberately invariant. Nexus has no execution-control state.
    pub permitted: bool,
    pub recorded: bool,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

impl HookResponse {
    pub fn allow(advisories: Vec<Advisory>) -> Self {
        Self {
            permitted: true,
            recorded: true,
            advisories,
            diagnostic: None,
        }
    }

    pub fn fail_open(error: impl std::fmt::Display) -> Self {
        Self {
            permitted: true,
            recorded: false,
            advisories: Vec::new(),
            diagnostic: Some(format!("nexus could not record this hook: {error}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: i64,
    pub project_id: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub payload: EventPayload,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventPayload(Value);

impl From<Value> for EventPayload {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolResultEvent {
    pub tool_name: String,
    pub output: Option<ToolPayload>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub project_id: String,
    pub agent: String,
    pub worktree: Option<String>,
    pub status: SessionStatus,
    pub task_summary: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Stopped,
}

string_enum!(parse SessionStatus, "session status", {
    Active => "active",
    Stopped => "stopped",
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub id: String,
    pub project_id: String,
    pub left_claim_id: String,
    pub right_claim_id: String,
    pub path: String,
    pub severity: Severity,
    pub kind: String,
    pub status: ConflictStatus,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStatus {
    Open,
    Resolved,
}

string_enum!(parse ConflictStatus, "conflict status", {
    Open => "open",
    Resolved => "resolved",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordScope {
    #[default]
    Active,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictScope {
    #[default]
    Open,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRelease {
    Session,
    Path(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCompletion {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoredAdvisoryPolicy {
    Record,
    Omit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusCounts {
    pub active_sessions: i64,
    pub active_claims: i64,
    pub open_conflicts: i64,
    pub events: i64,
}
