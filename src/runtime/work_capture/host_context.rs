// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary explicit-work-context: decode only bounded caller-supplied identity handoffs; never guess from checkout, PID, or time.
use super::{lifecycle::stable_id, CaptureContext};
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
    #[serde(default, deserialize_with = "crate::model_hint::deserialize")]
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

impl WorkHandoff {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 {
            bail!("work context exceeds 16 KiB");
        }
        let value: Self = serde_json::from_slice(bytes)?;
        if value.format_version != 1 {
            bail!("unsupported work context format");
        }
        for text in [
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
        {
            if text.trim().is_empty() || text.len() > 512 || text.chars().any(char::is_control) {
                bail!("invalid work context identity");
            }
        }
        Ok(value)
    }

    /// Explicit individual environment values and CLI options take precedence over this default handoff.
    pub fn apply_defaults(&self, context: &mut CaptureContext) {
        if context.work_session_id.is_none() {
            context.work_session_id = Some(self.work_session_id.clone());
        }
        if context.parent_activity_id.is_none() {
            context.parent_activity_id = self.parent_activity_id.clone();
        }
        if std::env::var_os("NEXUS_HOST_SESSION_ID").is_none() {
            context.hook.session_id = self.host_session_id.clone();
        }
        if std::env::var_os("NEXUS_AGENT").is_none() {
            context.hook.agent = self.agent.clone();
        }
        if context.hook.turn_id.is_none() {
            context.hook.turn_id = self.turn_id.clone();
        }
        if context.hook.model.is_none() {
            context.hook.model = self.model.clone();
        }
    }
}
