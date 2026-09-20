// @feature coordination
// @spec docs/features/coordination.md
// @boundary dynamic-json
use super::domain::{
    Claim, ClaimRelease, ConflictRecord, ConflictScope, EventRecord, HookResponse, RecordScope,
    SessionRecord, SessionStopInput, StatusCounts, ToolCompletion, ToolHookInput, UserPromptInput,
};
use crate::agents::analyst::AnalysisResult;
use crate::config::Config;
use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug)]
pub enum ServiceRequest {
    UserPrompt(UserPromptInput),
    ToolHook {
        phase: ToolHookPhase,
        input: ToolHookInput,
    },
    SessionStop(SessionStopInput),
    Status,
    Events(EventQuery),
    Sessions(SessionQuery),
    Claims(ClaimQuery),
    Conflicts(ConflictQuery),
    Resolve(ResolveCommand),
    Release(ReleaseCommand),
    Analyze(AnalyzeCommand),
    Config,
}

impl ServiceRequest {
    pub fn decode(method: &str, params: Value) -> Result<Self> {
        Ok(match method {
            "user_prompt" => Self::UserPrompt(decode(params)?),
            "pre_tool_use" => Self::ToolHook {
                phase: ToolHookPhase::Before,
                input: decode(params)?,
            },
            "post_tool_use" => Self::ToolHook {
                phase: ToolHookPhase::After(ToolCompletion::Succeeded),
                input: decode(params)?,
            },
            "post_tool_failure" => Self::ToolHook {
                phase: ToolHookPhase::After(ToolCompletion::Failed),
                input: decode(params)?,
            },
            "session_stop" => Self::SessionStop(decode(params)?),
            "status" => Self::Status,
            "events" => Self::Events(decode(params)?),
            "sessions" => Self::Sessions(decode(params)?),
            "claims" => Self::Claims(decode(params)?),
            "conflicts" => Self::Conflicts(decode(params)?),
            "resolve" => Self::Resolve(decode(params)?),
            "release" => Self::Release(decode(params)?),
            "analyze" => Self::Analyze(decode(params)?),
            "config" => Self::Config,
            _ => bail!("unknown method {method}"),
        })
    }

    pub fn wire_parts(&self) -> Result<(&'static str, Value)> {
        Ok(match self {
            Self::UserPrompt(input) => ("user_prompt", serde_json::to_value(input)?),
            Self::ToolHook { phase, input } => (phase.method(), serde_json::to_value(input)?),
            Self::SessionStop(input) => ("session_stop", serde_json::to_value(input)?),
            Self::Status => ("status", Value::Object(Default::default())),
            Self::Events(query) => ("events", serde_json::to_value(query)?),
            Self::Sessions(query) => ("sessions", serde_json::to_value(query)?),
            Self::Claims(query) => ("claims", serde_json::to_value(query)?),
            Self::Conflicts(query) => ("conflicts", serde_json::to_value(query)?),
            Self::Resolve(command) => ("resolve", serde_json::to_value(command)?),
            Self::Release(command) => ("release", serde_json::to_value(command)?),
            Self::Analyze(command) => ("analyze", serde_json::to_value(command)?),
            Self::Config => ("config", Value::Object(Default::default())),
        })
    }

    pub fn is_lifecycle_method(method: &str) -> bool {
        matches!(
            method,
            "user_prompt" | "pre_tool_use" | "post_tool_use" | "post_tool_failure" | "session_stop"
        )
    }
}

fn decode<T: DeserializeOwned>(params: Value) -> Result<T> {
    Ok(serde_json::from_value(params)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolHookPhase {
    Before,
    After(ToolCompletion),
}

impl ToolHookPhase {
    fn method(self) -> &'static str {
        match self {
            Self::Before => "pre_tool_use",
            Self::After(ToolCompletion::Succeeded) => "post_tool_use",
            Self::After(ToolCompletion::Failed) => "post_tool_failure",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EventQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_event_limit")]
    pub limit: usize,
}

impl EventQuery {
    pub fn bounded_limit(&self) -> usize {
        self.limit.clamp(1, 10_000)
    }
}

fn default_event_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionQuery {
    #[serde(default)]
    pub scope: RecordScope,
}

impl SessionQuery {
    pub fn scope(&self) -> RecordScope {
        self.scope
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ClaimQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: RecordScope,
}

impl ClaimQuery {
    pub fn scope(&self) -> RecordScope {
        self.scope
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConflictQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: ConflictScope,
}

impl ConflictQuery {
    pub fn scope(&self) -> ConflictScope {
        self.scope
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveCommand {
    pub conflict_id: String,
    #[serde(default = "default_resolution")]
    pub resolution: String,
}

fn default_resolution() -> String {
    "acknowledged".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseCommand {
    pub session_id: String,
    #[serde(default)]
    pub path: Option<String>,
}

impl ReleaseCommand {
    pub fn release(&self) -> ClaimRelease {
        self.path
            .clone()
            .map(ClaimRelease::Path)
            .unwrap_or(ClaimRelease::Session)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeCommand {
    pub conflict_id: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ServiceResponse {
    Hook(HookResponse),
    Prompt(PromptResponse),
    Status(StatusResponse),
    Events(EventsResponse),
    Sessions(SessionsResponse),
    Claims(ClaimsResponse),
    Conflicts(ConflictsResponse),
    Resolved(ResolvedResponse),
    Released(ReleasedResponse),
    Analysis(AnalysisResponse),
    Config(Box<ConfigResponse>),
    Error(ErrorResponse),
}

impl ServiceResponse {
    pub fn error(error: impl std::fmt::Display) -> Self {
        Self::Error(ErrorResponse {
            ok: false,
            error: error.to_string(),
        })
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("service responses are serializable")
    }
}

#[derive(Debug, Serialize)]
pub struct PromptResponse {
    pub permitted: bool,
    pub recorded: bool,
    pub project_id: String,
    pub prompt_hash: String,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub ok: bool,
    pub status: &'static str,
    pub counts: StatusCounts,
    pub config_hash: String,
}

#[derive(Debug, Serialize)]
pub struct EventsResponse {
    pub ok: bool,
    pub events: Vec<EventRecord>,
}

#[derive(Debug, Serialize)]
pub struct SessionsResponse {
    pub ok: bool,
    pub sessions: Vec<SessionRecord>,
}

#[derive(Debug, Serialize)]
pub struct ClaimsResponse {
    pub ok: bool,
    pub claims: Vec<Claim>,
}

#[derive(Debug, Serialize)]
pub struct ConflictsResponse {
    pub ok: bool,
    pub conflicts: Vec<ConflictRecord>,
}

#[derive(Debug, Serialize)]
pub struct ResolvedResponse {
    pub ok: bool,
    pub conflict_id: String,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReleasedResponse {
    pub ok: bool,
    pub released: usize,
}

#[derive(Debug, Serialize)]
pub struct AnalysisResponse {
    pub ok: bool,
    pub analysis: AnalysisResult,
}

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub ok: bool,
    pub config: Config,
    pub sources: Vec<PathBuf>,
    pub hash: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_scope_is_an_enum_not_a_boolean_switch() {
        let request =
            ServiceRequest::decode("claims", json!({"project_id":"project-1","scope":"all"}))
                .unwrap();
        let ServiceRequest::Claims(query) = request else {
            panic!("expected claims query");
        };
        assert_eq!(query.scope(), RecordScope::All);

        let error = ServiceRequest::decode("claims", json!({"active_only":false})).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn open_ended_tool_json_is_confined_to_the_tool_payload() {
        let request = ServiceRequest::decode(
            "pre_tool_use",
            json!({
                "session_id":"session-1",
                "tool_use_id":"tool-1",
                "tool_name":"future_tool",
                "tool_input":{"future":{"shape":[1,2,3]}}
            }),
        )
        .unwrap();
        let ServiceRequest::ToolHook { input, .. } = request else {
            panic!("expected tool hook");
        };
        assert_eq!(input.tool_input.as_json()["future"]["shape"][2], 3);
    }
}
