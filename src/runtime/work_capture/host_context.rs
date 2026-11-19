// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary explicit-work-context: decode only bounded caller-supplied identity handoffs; never guess from checkout, PID, or time.
use super::lifecycle::stable_id;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkHandoff {
    pub format_version: u8,
    pub work_session_id: String,
    pub parent_activity_id: Option<String>,
    #[serde(default)]
    pub request_activity_id: Option<String>,
    pub host_session_id: String,
    pub agent: String,
    pub turn_id: Option<String>,
    #[serde(default, deserialize_with = "crate::optional_hint::deserialize")]
    pub model: Option<String>,
}

pub fn host_handoff(
    agent: &str,
    session: &str,
    turn: Option<&str>,
    model: Option<&str>,
) -> WorkHandoff {
    WorkHandoff {
        format_version: 1,
        work_session_id: stable_id(&["work", agent, session]),
        parent_activity_id: Some(turn.map_or_else(
            || stable_id(&["agent", agent, session]),
            |turn| stable_id(&["turn", agent, session, turn]),
        )),
        request_activity_id: turn.map(|turn| stable_id(&["request", agent, session, turn])),
        host_session_id: session.into(),
        agent: agent.into(),
        turn_id: turn.map(str::to_owned),
        model: model.map(str::to_owned),
    }
}

pub(super) fn environment_handoff() -> Result<Option<WorkHandoff>> {
    std::env::var("NEXUS_WORK_CONTEXT")
        .ok()
        .map(|value| WorkHandoff::decode(value.as_bytes()))
        .transpose()
}

pub(super) struct ProcessHostIdentity {
    pub(super) agent: Option<String>,
    pub(super) session_id: Option<String>,
}

pub(super) fn process_host_identity(inherited: Option<&WorkHandoff>) -> ProcessHostIdentity {
    ProcessHostIdentity {
        agent: identity_value("NEXUS_AGENT", inherited.map(|handoff| &handoff.agent)),
        session_id: identity_value(
            "NEXUS_HOST_SESSION_ID",
            inherited.map(|handoff| &handoff.host_session_id),
        ),
    }
}

fn identity_value(variable: &str, inherited: Option<&String>) -> Option<String> {
    match std::env::var(variable) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => inherited.cloned(),
        Err(std::env::VarError::NotUnicode(_)) => None,
    }
}

pub(crate) fn companion_host_session(agent: &str) -> Result<Option<String>> {
    let handoff = environment_handoff().ok().flatten();
    let identity = process_host_identity(handoff.as_ref());
    if identity.agent.as_deref().is_some_and(|host| host != agent) {
        return Ok(None);
    }
    if let Some(session) = &identity.session_id {
        if session.trim().is_empty() || session.len() > 512 || session.chars().any(char::is_control)
        {
            bail!("invalid MCP host session identity");
        }
    }
    Ok(identity.session_id)
}

impl WorkHandoff {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 {
            bail!("work context exceeds 16 KiB");
        }
        let value: Self = serde_json::from_slice(bytes)?;
        if value.format_version != 1 {
            bail!("unsupported work context format");
        }
        if [
            Some(value.work_session_id.as_str()),
            value.parent_activity_id.as_deref(),
            value.request_activity_id.as_deref(),
            Some(value.host_session_id.as_str()),
            Some(value.agent.as_str()),
            value.turn_id.as_deref(),
            value.model.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|text| {
            text.trim().is_empty() || text.len() > 512 || text.chars().any(char::is_control)
        }) {
            bail!("invalid work context identity");
        }
        Ok(value)
    }
}
