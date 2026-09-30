// @feature runtime
// @spec docs/features/runtime.md
// @entrypoint session_end
// @boundary dynamic-json
use super::daemon;
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::coordination::domain::{HookContext, SessionStopInput};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct SessionEndHookInput {
    session_id: String,
    #[serde(default)]
    cwd: Option<String>,
}

pub async fn session_end(loaded: &LoadedConfig, explicit_config: Option<&Path>, agent: &str) {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(input) = serde_json::from_str::<SessionEndHookInput>(&input) else {
        return;
    };
    let request = ServiceRequest::SessionStop(SessionStopInput {
        context: HookContext {
            session_id: input.session_id,
            project_root: input.cwd,
            agent: agent.to_owned(),
        },
    });
    daemon::lifecycle_request(loaded, explicit_config, &request).await;
}
