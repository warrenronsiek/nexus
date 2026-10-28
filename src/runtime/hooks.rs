// @feature runtime
// @spec docs/features/runtime.md
// @entrypoint session_end
// @boundary dynamic-json
use super::bounded_io;
use super::client::NexusClient;
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
    let Some(input) = decode_session_end_input(&mut std::io::stdin().lock()) else {
        return;
    };
    let context = HookContext {
        session_id: input.session_id,
        project_root: input.cwd,
        agent: agent.to_owned(),
        turn_id: None,
        model: None,
    };
    let _ = stop_visited(loaded, explicit_config, &context).await;
}

pub(crate) async fn stop_visited(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    context: &HookContext,
) -> serde_json::Value {
    let (mut cleanup, complete_plan) = checkout_stops(loaded, explicit_config, context);
    let primary = NexusClient::new(loaded, explicit_config);
    let request = ServiceRequest::SessionStop(SessionStopInput {
        context: context.clone(),
    });
    let complete = async {
        let result = primary.lifecycle_request(&request).await;
        anyhow::ensure!(
            result.get("recorded").and_then(serde_json::Value::as_bool) == Some(true),
            "primary stop not recorded"
        );
        while let Some(result) = cleanup.join_next().await {
            anyhow::ensure!(
                result?.get("recorded").and_then(serde_json::Value::as_bool) == Some(true),
                "checkout stop not recorded"
            );
        }
        Ok::<_, anyhow::Error>(result)
    };
    let completed = tokio::time::timeout(super::client::LIFECYCLE_RESPONSE_BUDGET, complete).await;
    if let Ok(Ok(result)) = completed {
        if complete_plan && super::checkout_visits::clear(loaded, context).is_ok() {
            return result;
        }
    }
    serde_json::to_value(crate::coordination::domain::HookResponse::fail_open(
        "checkout cleanup incomplete",
    ))
    .unwrap()
}

fn checkout_stops(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    context: &HookContext,
) -> (tokio::task::JoinSet<serde_json::Value>, bool) {
    let mut cleanup = tokio::task::JoinSet::new();
    let visits = super::checkout_visits::observed(loaded, context).and_then(|visits| {
        visits
            .iter()
            .map(|visit| visit.restore(loaded, context))
            .collect::<anyhow::Result<Vec<_>>>()
    });
    let complete_plan = visits.is_ok();
    for (target, context) in visits.unwrap_or_default() {
        let explicit = explicit_config.map(Path::to_path_buf);
        cleanup.spawn(async move {
            NexusClient::new(&target, explicit.as_deref())
                .lifecycle_request(&ServiceRequest::SessionStop(SessionStopInput { context }))
                .await
        });
    }
    (cleanup, complete_plan)
}

fn decode_session_end_input(reader: &mut impl Read) -> Option<SessionEndHookInput> {
    let input = bounded_io::read_json_document(reader).ok()?;
    serde_json::from_slice(&input).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn oversized_nonterminated_session_end_input_is_rejected_at_the_sentinel() {
        let mut input = Cursor::new(vec![
            b'x';
            super::super::bounded_io::MAX_JSON_FRAME_BYTES + 2
        ]);

        assert!(decode_session_end_input(&mut input).is_none());
        assert_eq!(
            input.position(),
            (super::super::bounded_io::MAX_JSON_FRAME_BYTES + 1) as u64
        );
    }
}
