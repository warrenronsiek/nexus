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
    let request = ServiceRequest::SessionStop(SessionStopInput {
        context: HookContext {
            session_id: input.session_id,
            project_root: input.cwd,
            agent: agent.to_owned(),
            turn_id: None,
            model: None,
        },
    });
    NexusClient::new(loaded, explicit_config)
        .lifecycle_request(&request)
        .await;
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
