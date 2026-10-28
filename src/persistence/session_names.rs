// @feature coordination
// @feature persistence
// @feature work-history
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @boundary fixture-json: source title fixtures decode through the closed work schema.

use crate::coordination::domain::{RecordScope, RecordWindow, SessionRecord};
use crate::persistence::Store;
use crate::work_history::{host_agent_identity, WorkSubject};
use anyhow::Result;

impl Store {
    pub(crate) fn list_sessions_window(
        &mut self,
        project_key: Option<&str>,
        scope: RecordScope,
        activity_window_seconds: i64,
        limit: usize,
    ) -> Result<RecordWindow<SessionRecord>> {
        let mut records =
            self.read_sessions_window(project_key, scope, activity_window_seconds, limit)?;
        for record in &mut records.items {
            record.display_name = self.host_session_display_name(record)?;
        }
        Ok(records)
    }
    fn host_session_display_name(&mut self, record: &SessionRecord) -> Result<Option<String>> {
        host_title(
            &mut self.connection,
            &record.project_id,
            &record.agent,
            &record.session_id,
        )
    }
}

pub(super) fn host_title(
    connection: &mut super::backend::DbConnection,
    project: &str,
    agent: &str,
    session: &str,
) -> Result<Option<String>> {
    let subject = WorkSubject::Activity {
        id: host_agent_identity(agent, session),
    };
    let Some(detail) = super::work_history::session_title(connection, project, &subject)? else {
        return Ok(None);
    };
    if detail.context.host_session_id.as_deref() != Some(session)
        || detail
            .context
            .labels
            .get("nexus.capture.host")
            .map(String::as_str)
            != Some(agent)
    {
        return Ok(None);
    }
    Ok(detail.context.labels.get("nexus.session_title").cloned())
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use crate::coordination::domain::RecordScope;
    use crate::persistence::Store;
    use crate::work_history::{WorkAction, WorkActor};
    use serde_json::{json, Value};
    fn identity(agent: &str, session: &str) -> String {
        let mut hash = blake3::Hasher::new();
        for part in ["agent", agent, session] {
            hash.update(&(part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        hash.finalize().to_hex().to_string()
    }
    fn title(store: &mut Store, project: &str, agent: &str, session: &str, title: &str) {
        let id = identity(agent, session);
        let observation = |key: &str, body: Value| json!({"id":format!("{key}-{id}"),"subject":{"scope":"activity","id":id},"occurred_at":"2026-10-27T12:00:00Z","producer":{"name":"fixture","version":"1","instance_id":"fixture"},"method":"host_hook_boundary","body":body});
        let action: WorkAction=serde_json::from_value(json!({"action":"record","observations":[
            observation("declared",json!({"event":"activity_declared","name":"Host session","kind":"host.session","work_session_id":"shared-root","context":{"host_session_id":session,"labels":{"nexus.capture.host":agent}}})),
            observation("title",json!({"event":"milestone","name":"Host session title updated","context":{"labels":{"nexus.session_title":title}}}))
        ]})).unwrap();
        store
            .work_command(
                project,
                &WorkActor {
                    agent: agent.into(),
                    session_id: session.into(),
                    model_id: None,
                    config_hash: "fixture".into(),
                },
                &action,
            )
            .unwrap();
    }
    #[test]
    fn coordination_display_names_use_exact_project_agent_and_full_host_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temporary.path().join("names.db")).unwrap();
        store
            .ensure_session("project-a", "same-session", "codex", "fixture")
            .unwrap();
        store
            .ensure_session("project-b", "same-session", "codex", "fixture")
            .unwrap();
        store
            .ensure_session("project-a", "unmatched", "claude", "fixture")
            .unwrap();
        title(
            &mut store,
            "project-a",
            "codex",
            "same-session",
            "Implement accounting",
        );
        title(
            &mut store,
            "project-b",
            "codex",
            "same-session",
            "Other project",
        );
        title(
            &mut store,
            "project-a",
            "claude",
            "same-session",
            "Other host",
        );
        title(&mut store, "project-a", "codex", "unmatched", "Wrong agent");
        let sessions =
            serde_json::to_value(store.list_sessions(RecordScope::All, 60).unwrap()).unwrap();
        let rows = sessions.as_array().unwrap();
        for row in rows {
            let expected = match (
                row["project_id"].as_str().unwrap(),
                row["session_id"].as_str().unwrap(),
            ) {
                ("project-a", "same-session") => json!("Implement accounting"),
                ("project-b", "same-session") => json!("Other project"),
                _ => Value::Null,
            };
            assert_eq!(row["display_name"], expected);
        }
    }
}
