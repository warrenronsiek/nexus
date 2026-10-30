// @feature work-history
// @spec docs/features/work-history.md
// @boundary historical-host-fixture: a closed synthetic report checks retained provenance without rewriting source evidence.
use super::{host_work_identity, WorkEvent, WorkObservation, WorkSubject};

impl WorkObservation {
    /// Resolve declared host ownership, including the exact legacy companion identity.
    pub(crate) fn host_root_owner(&self) -> Option<&str> {
        let WorkSubject::Session { id } = &self.input.subject else {
            return None;
        };
        let WorkEvent::SessionOpened { context, .. } = &self.input.body else {
            return None;
        };
        if let Some(host) = context.labels.get("nexus.capture.host") {
            return (host != "nexus-cli").then_some(host.as_str());
        }
        let actor = self
            .source
            .as_ref()
            .map(|source| &source.provenance)
            .unwrap_or(&self.provenance);
        let session = context
            .host_session_id
            .as_deref()
            .unwrap_or(&actor.session_id);
        (self.input.producer.name == "nexus-companion"
            && matches!(actor.agent.as_str(), "codex" | "claude")
            && *id == host_work_identity(&actor.agent, session))
        .then_some(actor.agent.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_history::{WorkActor, WorkObservationInput, WorkSource};

    #[test]
    fn legacy_origin_uses_the_retained_actor_and_exact_host_root_identity() {
        let input: WorkObservationInput = serde_json::from_value(serde_json::json!({
            "id":"opened", "subject":{"scope":"session","id":"58448194a44a55a3e6f7e0e780fdcfe223da3649b0c53a4ca1e0b85eadd89329"},
            "occurred_at":"2026-10-24T12:00:00Z", "producer":{"name":"nexus-companion","version":"0.1.0","instance_id":"old-process"},
            "method":"companion_callback_wall_clock", "body":{"event":"session_opened","title":"Legacy harness","context":{"host_session_id":"legacy-native"}}
        })).unwrap();
        let mut observation = WorkObservation {
            input,
            project_id: "fixture".into(),
            recorded_at: "2026-10-24T12:00:00Z".parse().unwrap(),
            provenance: WorkActor {
                agent: "codex".into(),
                session_id: "legacy-native".into(),
                model_id: None,
                config_hash: "fixture".into(),
            },
            source: None,
        };
        assert_eq!(observation.host_root_owner(), Some("codex"));
        observation.source = Some(WorkSource {
            project_id: observation.project_id.clone(),
            recorded_at: observation.recorded_at,
            provenance: observation.provenance.clone(),
        });
        observation.provenance.agent = "importer".into();
        assert_eq!(observation.host_root_owner(), Some("codex"));
        observation.input.subject = WorkSubject::Session {
            id: "caller-owned-root".into(),
        };
        assert_eq!(observation.host_root_owner(), None);
    }
}
