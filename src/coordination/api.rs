// @feature coordination
// @feature usage-analytics
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
use super::domain::{
    Claim, ClaimRelease, ConflictRecord, ConflictScope, DashboardRecords, EventRecord,
    HookResponse, ProjectSummary, RecordScope, ScriptUseInput, SessionRecord, SessionStopInput,
    SkillUseInput, StatusCounts, ToolCompletion, ToolHookInput, UserPromptInput,
};
use crate::agents::analyst::AnalysisResult;
use crate::config::Config;
use crate::persistence::UsageSummary;
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
    SkillUse(SkillUseInput),
    ScriptUse(ScriptUseInput),
    SessionStop(SessionStopInput),
    Status,
    Events(EventQuery),
    Sessions(SessionQuery),
    Claims(ClaimQuery),
    Conflicts(ConflictQuery),
    Resolve(ResolveCommand),
    Release(ReleaseCommand),
    Analyze(AnalyzeCommand),
    Projects,
    Dashboard(DashboardQuery),
    Usage(UsageQuery),
    Config,
}

impl ServiceRequest {
    pub fn decode(method: &str, params: Value) -> Result<Self> {
        let Some(family) = RequestFamily::classify(method) else {
            bail!("unknown method {method}");
        };
        match family {
            RequestFamily::Lifecycle => decode_lifecycle(method, params),
            RequestFamily::Read => decode_read(method, params),
            RequestFamily::Command => decode_command(method, params),
            RequestFamily::Report => decode_report(method, params),
        }
    }

    pub fn wire_parts(&self) -> Result<(&'static str, Value)> {
        match request_family(self) {
            RequestFamily::Lifecycle => lifecycle_wire_parts(self),
            RequestFamily::Read => read_wire_parts(self),
            RequestFamily::Command => command_wire_parts(self),
            RequestFamily::Report => report_wire_parts(self),
        }
    }

    pub fn is_lifecycle_method(method: &str) -> bool {
        LIFECYCLE_METHODS.contains(&method)
    }
}

fn decode_lifecycle(method: &str, params: Value) -> Result<ServiceRequest> {
    match method {
        "user_prompt" => decode(params).map(ServiceRequest::UserPrompt),
        "pre_tool_use" => decode_tool_hook(ToolHookPhase::Before, params),
        "post_tool_use" => {
            decode_tool_hook(ToolHookPhase::After(ToolCompletion::Succeeded), params)
        }
        "post_tool_failure" => {
            decode_tool_hook(ToolHookPhase::After(ToolCompletion::Failed), params)
        }
        "skill_use" => decode(params).map(ServiceRequest::SkillUse),
        "script_use" => decode(params).map(ServiceRequest::ScriptUse),
        "session_stop" => decode(params).map(ServiceRequest::SessionStop),
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn decode_tool_hook(phase: ToolHookPhase, params: Value) -> Result<ServiceRequest> {
    Ok(ServiceRequest::ToolHook {
        phase,
        input: decode(params)?,
    })
}

fn decode_read(method: &str, params: Value) -> Result<ServiceRequest> {
    match method {
        "status" => Ok(ServiceRequest::Status),
        "events" => decode(params).map(ServiceRequest::Events),
        "sessions" => decode(params).map(ServiceRequest::Sessions),
        "claims" => decode(params).map(ServiceRequest::Claims),
        "conflicts" => decode(params).map(ServiceRequest::Conflicts),
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn decode_command(method: &str, params: Value) -> Result<ServiceRequest> {
    match method {
        "resolve" => decode(params).map(ServiceRequest::Resolve),
        "release" => decode(params).map(ServiceRequest::Release),
        "analyze" => decode(params).map(ServiceRequest::Analyze),
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn decode_report(method: &str, params: Value) -> Result<ServiceRequest> {
    match method {
        "projects" => Ok(ServiceRequest::Projects),
        "dashboard" => decode(params).map(ServiceRequest::Dashboard),
        "usage" => decode(params).map(ServiceRequest::Usage),
        "config" => Ok(ServiceRequest::Config),
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn request_family(request: &ServiceRequest) -> RequestFamily {
    match request {
        ServiceRequest::UserPrompt(_)
        | ServiceRequest::ToolHook { .. }
        | ServiceRequest::SkillUse(_)
        | ServiceRequest::ScriptUse(_)
        | ServiceRequest::SessionStop(_) => RequestFamily::Lifecycle,
        ServiceRequest::Status
        | ServiceRequest::Events(_)
        | ServiceRequest::Sessions(_)
        | ServiceRequest::Claims(_)
        | ServiceRequest::Conflicts(_) => RequestFamily::Read,
        ServiceRequest::Resolve(_) | ServiceRequest::Release(_) | ServiceRequest::Analyze(_) => {
            RequestFamily::Command
        }
        ServiceRequest::Projects
        | ServiceRequest::Dashboard(_)
        | ServiceRequest::Usage(_)
        | ServiceRequest::Config => RequestFamily::Report,
    }
}

fn lifecycle_wire_parts(request: &ServiceRequest) -> Result<(&'static str, Value)> {
    match request {
        ServiceRequest::UserPrompt(input) => encode("user_prompt", input),
        ServiceRequest::ToolHook { phase, input } => encode(phase.method(), input),
        ServiceRequest::SkillUse(input) => encode("skill_use", input),
        ServiceRequest::ScriptUse(input) => encode("script_use", input),
        ServiceRequest::SessionStop(input) => encode("session_stop", input),
        _ => unreachable!("request family was selected before encoding"),
    }
}

fn read_wire_parts(request: &ServiceRequest) -> Result<(&'static str, Value)> {
    match request {
        ServiceRequest::Status => Ok(empty("status")),
        ServiceRequest::Events(query) => encode("events", query),
        ServiceRequest::Sessions(query) => encode("sessions", query),
        ServiceRequest::Claims(query) => encode("claims", query),
        ServiceRequest::Conflicts(query) => encode("conflicts", query),
        _ => unreachable!("request family was selected before encoding"),
    }
}

fn command_wire_parts(request: &ServiceRequest) -> Result<(&'static str, Value)> {
    match request {
        ServiceRequest::Resolve(command) => encode("resolve", command),
        ServiceRequest::Release(command) => encode("release", command),
        ServiceRequest::Analyze(command) => encode("analyze", command),
        _ => unreachable!("request family was selected before encoding"),
    }
}

fn report_wire_parts(request: &ServiceRequest) -> Result<(&'static str, Value)> {
    match request {
        ServiceRequest::Projects => Ok(empty("projects")),
        ServiceRequest::Dashboard(query) => encode("dashboard", query),
        ServiceRequest::Usage(query) => encode("usage", query),
        ServiceRequest::Config => Ok(empty("config")),
        _ => unreachable!("request family was selected before encoding"),
    }
}

const LIFECYCLE_METHODS: &[&str] = &[
    "user_prompt",
    "pre_tool_use",
    "post_tool_use",
    "post_tool_failure",
    "skill_use",
    "script_use",
    "session_stop",
];
const READ_METHODS: &[&str] = &["status", "events", "sessions", "claims", "conflicts"];
const COMMAND_METHODS: &[&str] = &["resolve", "release", "analyze"];
const REPORT_METHODS: &[&str] = &["projects", "dashboard", "usage", "config"];

#[derive(Clone, Copy)]
enum RequestFamily {
    Lifecycle,
    Read,
    Command,
    Report,
}

impl RequestFamily {
    fn classify(method: &str) -> Option<Self> {
        [
            (Self::Lifecycle, LIFECYCLE_METHODS),
            (Self::Read, READ_METHODS),
            (Self::Command, COMMAND_METHODS),
            (Self::Report, REPORT_METHODS),
        ]
        .into_iter()
        .find_map(|(family, methods)| methods.contains(&method).then_some(family))
    }
}

fn encode<T: Serialize>(method: &'static str, params: &T) -> Result<(&'static str, Value)> {
    Ok((method, serde_json::to_value(params)?))
}

fn empty(method: &'static str) -> (&'static str, Value) {
    (method, Value::Object(Default::default()))
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DashboardQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UsageQuery {
    #[serde(default)]
    pub project_id: Option<String>,
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
    Projects(ProjectsResponse),
    Dashboard(DashboardResponse),
    Usage(UsageResponse),
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
pub struct ProjectsResponse {
    pub ok: bool,
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Serialize)]
pub struct DashboardResponse {
    pub ok: bool,
    pub project_id: Option<String>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub refresh_interval_ms: u64,
    #[serde(flatten)]
    pub records: DashboardRecords,
}

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub ok: bool,
    pub project_id: Option<String>,
    #[serde(flatten)]
    pub summary: UsageSummary,
    pub capture_health: CaptureHealthResponse,
}

#[derive(Debug, Serialize)]
pub struct CaptureHealthResponse {
    pub label: &'static str,
    pub received: u64,
    pub recorded: u64,
    pub dropped: u64,
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
