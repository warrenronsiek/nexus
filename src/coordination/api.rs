// @feature coordination
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @boundary dynamic-json
mod analytics;
mod memory;
mod papercuts;
mod responses;
mod sos;

pub use memory::*;
pub use papercuts::*;
pub use responses::*;
pub use sos::*;

use super::domain::{
    ClaimRelease, ConflictScope, HookContext, RecordScope, ScriptUseInput, SessionStopInput,
    SkillUseInput, ToolCompletion, ToolHookInput, UserPromptInput,
};
use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_EVENT_RESULTS: usize = 64;
pub(crate) const MAX_SESSION_RESULTS: usize = 64;
pub(crate) const MAX_SESSION_SUMMARY_BYTES: usize = 1_024;
pub(crate) const MAX_CLAIM_RESULTS: usize = 64;
pub(crate) const MAX_CONFLICT_RESULTS: usize = 32;
pub(crate) const MAX_PROJECT_RESULTS: usize = 16;
pub(crate) const MAX_PROJECT_SUMMARY_SESSIONS: usize = 4;
pub(crate) const MAX_DASHBOARD_EVENT_RESULTS: usize = 32;
pub(crate) const MAX_DASHBOARD_RECORD_RESULTS: usize = 16;
pub(crate) const MAX_USAGE_DIMENSION_RESULTS: usize = 64;
pub(crate) const MAX_USAGE_SESSION_IDENTITY_ROWS: usize = 4_096;
pub(crate) const MAX_OPEN_PAPERCUT_RESULTS: usize = 32;
pub(crate) const MAX_MEMORY_SEARCH_REGEX_BYTES: usize = 4 * 1024;
pub(crate) const MAX_LIFECYCLE_ADVISORIES: usize = 32;
pub(crate) const MAX_SESSION_ID_BYTES: usize = 512;
pub(crate) const MAX_AGENT_BYTES: usize = 256;

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
    MemoryStatus(MemoryStatusQuery),
    MemoryContext(MemoryContextQuery),
    MemoryAdd(MemoryAddCommand),
    MemorySearch(MemorySearchQuery),
    MemoryExpand(MemorySummaryQuery),
    MemoryInvalidate(MemorySummaryQuery),
    MemoryConsolidate,
    Papercuts(PapercutCommand),
    Sos(SosCommand),
    Analytics(crate::analytics::types::AnalyticsCommand),
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
    let request = match method {
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
    }?;
    validate_lifecycle_request(&request)?;
    Ok(request)
}

fn validate_lifecycle_request(request: &ServiceRequest) -> Result<()> {
    let context = match request {
        ServiceRequest::UserPrompt(input) => &input.context,
        ServiceRequest::ToolHook { input, .. } => {
            validate_required_field("tool_use_id", &input.tool_use_id, 512)?;
            validate_required_field("tool_name", &input.tool_name, 256)?;
            validate_optional_field(
                "parent_tool_use_id",
                input.parent_tool_use_id.as_deref(),
                512,
            )?;
            &input.context
        }
        ServiceRequest::SkillUse(input) => {
            validate_required_field("invocation_id", &input.invocation_id, 512)?;
            validate_required_field("skill_name", &input.skill_name, 256)?;
            validate_optional_field("actor", input.actor.as_deref(), 256)?;
            &input.context
        }
        ServiceRequest::ScriptUse(input) => {
            validate_required_field("invocation_id", &input.invocation_id, 512)?;
            validate_required_field("script_name", &input.script_name, 4_096)?;
            &input.context
        }
        ServiceRequest::SessionStop(input) => &input.context,
        _ => return Ok(()),
    };
    validate_hook_context(context)
}

fn validate_hook_context(context: &HookContext) -> Result<()> {
    validate_required_field("session_id", &context.session_id, MAX_SESSION_ID_BYTES)?;
    validate_required_field("agent", &context.agent, MAX_AGENT_BYTES)?;
    validate_optional_field("project_root", context.project_root.as_deref(), 4_096)?;
    validate_optional_field("turn_id", context.turn_id.as_deref(), 256)?;
    validate_optional_field("model", context.model.as_deref(), 256)
}

pub(crate) fn validate_required_field(field: &str, value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > maximum {
        bail!("{field} must contain 1 to {maximum} bytes");
    }
    if value.chars().any(char::is_control) {
        bail!("{field} cannot contain control characters");
    }
    Ok(())
}

fn validate_optional_field(field: &str, value: Option<&str>, maximum: usize) -> Result<()> {
    if let Some(value) = value {
        if value.len() > maximum {
            bail!("{field} must contain at most {maximum} bytes");
        }
        if value.chars().any(char::is_control) {
            bail!("{field} cannot contain control characters");
        }
    }
    Ok(())
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
        "events" => decode_project_query(params, |query: &mut EventQuery| &mut query.project_id)
            .map(ServiceRequest::Events),
        "sessions" => {
            decode_project_query(params, |query: &mut SessionQuery| &mut query.project_id)
                .map(ServiceRequest::Sessions)
        }
        "claims" => decode_project_query(params, |query: &mut ClaimQuery| &mut query.project_id)
            .map(ServiceRequest::Claims),
        "conflicts" => {
            decode_project_query(params, |query: &mut ConflictQuery| &mut query.project_id)
                .map(ServiceRequest::Conflicts)
        }
        "memory_status" => decode(params).map(ServiceRequest::MemoryStatus),
        "memory_context" => decode(params).map(ServiceRequest::MemoryContext),
        "memory_search" => {
            let query: MemorySearchQuery = decode(params)?;
            if query.regex.len() > MAX_MEMORY_SEARCH_REGEX_BYTES {
                bail!(
                    "memory search regex must contain at most {} bytes",
                    MAX_MEMORY_SEARCH_REGEX_BYTES
                );
            }
            Ok(ServiceRequest::MemorySearch(query))
        }
        "memory_expand" => decode(params).map(ServiceRequest::MemoryExpand),
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn decode_command(method: &str, params: Value) -> Result<ServiceRequest> {
    match method {
        "analytics" => analytics::decode_analytics(params),
        "resolve" => decode(params).map(ServiceRequest::Resolve),
        "release" => decode(params).map(ServiceRequest::Release),
        "analyze" => decode(params).map(ServiceRequest::Analyze),
        "memory_add" => decode(params).map(ServiceRequest::MemoryAdd),
        "memory_invalidate" => decode(params).map(ServiceRequest::MemoryInvalidate),
        "memory_consolidate" => Ok(ServiceRequest::MemoryConsolidate),
        "papercuts" => {
            let mut command: PapercutCommand = decode(params)?;
            if let PapercutCommand::List { project_id, .. } = &mut command {
                normalize_read_project(project_id)?;
            }
            Ok(ServiceRequest::Papercuts(command))
        }
        "sos" => {
            let command: SosCommand = decode(params)?;
            validate_hook_context(command.context())?;
            if matches!(&command, SosCommand::Report { context, .. } if context.model.as_deref().is_none_or(|model| model.trim().is_empty()))
            {
                bail!("SOS model must contain 1 to 256 bytes");
            }
            Ok(ServiceRequest::Sos(command))
        }
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn decode_report(method: &str, params: Value) -> Result<ServiceRequest> {
    match method {
        "projects" => Ok(ServiceRequest::Projects),
        "dashboard" => {
            decode_project_query(params, |query: &mut DashboardQuery| &mut query.project_id)
                .map(ServiceRequest::Dashboard)
        }
        "usage" => decode_project_query(params, |query: &mut UsageQuery| &mut query.project_id)
            .map(ServiceRequest::Usage),
        "config" => Ok(ServiceRequest::Config),
        _ => unreachable!("method family was classified before decoding"),
    }
}

fn decode_project_query<T, F>(params: Value, project_id: F) -> Result<T>
where
    T: DeserializeOwned,
    F: FnOnce(&mut T) -> &mut Option<String>,
{
    let mut query = decode(params)?;
    normalize_read_project(project_id(&mut query))?;
    Ok(query)
}

fn normalize_read_project(project_id: &mut Option<String>) -> Result<()> {
    if let Some(raw) = project_id.as_deref() {
        let canonical = crate::config::canonical_project_id(raw).map_err(|_| {
            anyhow::anyhow!("project_id must contain 1 to 256 bytes and no control characters")
        })?;
        if canonical != raw {
            *project_id = Some(canonical.to_owned());
        }
    }
    Ok(())
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
        | ServiceRequest::Conflicts(_)
        | ServiceRequest::MemoryStatus(_)
        | ServiceRequest::MemoryContext(_)
        | ServiceRequest::MemorySearch(_)
        | ServiceRequest::MemoryExpand(_) => RequestFamily::Read,
        ServiceRequest::Resolve(_)
        | ServiceRequest::Release(_)
        | ServiceRequest::Analyze(_)
        | ServiceRequest::MemoryAdd(_)
        | ServiceRequest::MemoryInvalidate(_)
        | ServiceRequest::MemoryConsolidate
        | ServiceRequest::Papercuts(_)
        | ServiceRequest::Sos(_)
        | ServiceRequest::Analytics(_) => RequestFamily::Command,
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
        ServiceRequest::MemoryStatus(query) => encode("memory_status", query),
        ServiceRequest::MemoryContext(query) => encode("memory_context", query),
        ServiceRequest::MemorySearch(query) => encode("memory_search", query),
        ServiceRequest::MemoryExpand(query) => encode("memory_expand", query),
        _ => unreachable!("request family was selected before encoding"),
    }
}

fn command_wire_parts(request: &ServiceRequest) -> Result<(&'static str, Value)> {
    match request {
        ServiceRequest::Resolve(command) => encode("resolve", command),
        ServiceRequest::Release(command) => encode("release", command),
        ServiceRequest::Analyze(command) => encode("analyze", command),
        ServiceRequest::MemoryAdd(command) => encode("memory_add", command),
        ServiceRequest::MemoryInvalidate(command) => encode("memory_invalidate", command),
        ServiceRequest::MemoryConsolidate => Ok(empty("memory_consolidate")),
        ServiceRequest::Papercuts(command) => encode("papercuts", command),
        ServiceRequest::Sos(command) => encode("sos", command),
        ServiceRequest::Analytics(command) => encode("analytics", command),
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
const READ_METHODS: &[&str] = &[
    "status",
    "events",
    "sessions",
    "claims",
    "conflicts",
    "memory_status",
    "memory_context",
    "memory_search",
    "memory_expand",
];
const COMMAND_METHODS: &[&str] = &[
    "analytics",
    "resolve",
    "release",
    "analyze",
    "memory_add",
    "memory_invalidate",
    "memory_consolidate",
    "papercuts",
    "sos",
];
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
        self.limit.clamp(1, MAX_EVENT_RESULTS)
    }
}

fn default_event_limit() -> usize {
    MAX_EVENT_RESULTS
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionQuery {
    #[serde(default)]
    pub project_id: Option<String>,
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

#[cfg(test)]
mod tests;
