// @feature coordination
// @feature usage-analytics
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
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

#[derive(Debug, Clone, Serialize, Default)]
#[serde(transparent)]
pub struct ToolPayload(Value);

impl ToolPayload {
    pub fn as_json(&self) -> &Value {
        &self.0
    }

    pub fn first_string<'a>(&'a self, keys: &[&str]) -> Option<&'a str> {
        keys.iter()
            .find_map(|key| self.0.get(*key).and_then(Value::as_str))
    }
}

impl From<Value> for ToolPayload {
    fn from(value: Value) -> Self {
        // Claude MCP hook templates stringify objects; Codex preserves their JSON type.
        // Decode one bounded object layer, retaining ordinary text and malformed input.
        let object = value
            .as_str()
            .filter(|text| text.len() <= 1024 * 1024)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .filter(Value::is_object);
        Self(object.unwrap_or(value))
    }
}

impl<'de> Deserialize<'de> for ToolPayload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(Self::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub session_id: String,
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_path_intents: Vec<PathIntent>,
    #[serde(flatten)]
    pub context: HookContext,
    pub tool_use_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub parent_tool_use_id: Option<String>,
    #[serde(default)]
    pub tool_input: ToolPayload,
    #[serde(default)]
    pub tool_output: Option<ToolPayload>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub is_interrupt: Option<bool>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillEvidence {
    NativeHook,
    ExplicitInvocation,
    InstructionRead,
    AssetExecution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUseInput {
    #[serde(flatten)]
    pub context: HookContext,
    pub invocation_id: String,
    pub skill_name: String,
    pub evidence: SkillEvidence,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptOutcome {
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptUseInput {
    #[serde(flatten)]
    pub context: HookContext,
    pub invocation_id: String,
    pub script_name: String,
    pub outcome: ScriptOutcome,
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
    #[serde(default)]
    pub display_name: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
pub struct RecordWindow<T> {
    pub items: Vec<T>,
    pub truncated: bool,
}

impl<T> RecordWindow<T> {
    pub(crate) fn bounded(mut items: Vec<T>, maximum: usize) -> Self {
        let truncated = items.len() > maximum;
        items.truncate(maximum);
        Self { items, truncated }
    }

    pub(crate) fn try_bounded<Row, Error>(
        rows: Vec<Row>,
        maximum: usize,
        convert: impl FnMut(Row) -> Result<T, Error>,
    ) -> Result<Self, Error> {
        rows.into_iter()
            .map(convert)
            .collect::<Result<Vec<_>, _>>()
            .map(|items| Self::bounded(items, maximum))
    }
}

pub(crate) fn query_window_limit(maximum: usize) -> i64 {
    i64::try_from(maximum.saturating_add(1)).unwrap_or(i64::MAX)
}

pub(crate) fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut end = maximum_bytes.saturating_sub('…'.len_utf8());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value.truncate(end);
    value.push('…');
    value
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub project_id: String,
    pub worktrees: Vec<String>,
    pub agents: Vec<String>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DashboardRecords {
    pub counts: StatusCounts,
    pub events: RecordWindow<EventRecord>,
    pub sessions: RecordWindow<SessionRecord>,
    pub claims: RecordWindow<Claim>,
    pub conflicts: RecordWindow<ConflictRecord>,
}

#[cfg(test)]
mod tests {
    use super::RecordWindow;

    #[test]
    fn tool_payload_accepts_native_and_stringified_objects_without_reinterpreting_text() {
        use super::ToolPayload;
        use serde_json::{json, Value};
        let object = json!({"file_path":"shared.txt","content":"fixture"});
        for wire in [object.clone(), json!(object.to_string())] {
            let payload: ToolPayload = serde_json::from_value(wire).unwrap();
            assert_eq!(payload.as_json(), &object);
        }
        for text in ["ordinary output", "{malformed", "[1,2]", "\"text\""] {
            let payload: ToolPayload = serde_json::from_value(json!(text)).unwrap();
            assert_eq!(payload.as_json(), &Value::String(text.into()));
        }
        let oversized =
            json!({"file_path":"shared.txt","content":"x".repeat(1024 * 1024)}).to_string();
        let payload: ToolPayload = serde_json::from_value(json!(oversized)).unwrap();
        assert_eq!(payload.as_json(), &Value::String(oversized));
        let twice_encoded = json!(object.to_string()).to_string();
        let payload: ToolPayload = serde_json::from_value(json!(twice_encoded)).unwrap();
        assert_eq!(payload.as_json(), &Value::String(twice_encoded));
    }

    #[test]
    fn fallible_record_conversion_preserves_the_bounded_window() {
        let window = RecordWindow::try_bounded(vec![1, 2, 3], 2, |value| {
            Ok::<_, std::convert::Infallible>(value * 10)
        })
        .unwrap();

        assert_eq!(window.items, vec![10, 20]);
        assert!(window.truncated);
    }
}
