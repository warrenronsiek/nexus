// @feature persistence
// @feature work-history
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @spec docs/features/usage-analytics.md
// @boundary native-operation-correlation: only exact project/host/session/invocation identities join existing privacy-safe analytics; receipt timestamps never repair source execution boundaries.
use super::*;
use crate::persistence::schema::{
    analytics_actions as actions, analytics_calls as calls, analytics_categories as categories,
};
use std::collections::BTreeMap;

const ROW_LIMIT: i64 = 8192;
#[derive(Default)]
pub(super) struct OperationCache {
    session: Option<String>,
    operations: BTreeMap<String, Vec<WorkToolOperation>>,
    partial: bool,
}
#[derive(Queryable)]
struct OperationRow {
    invocation: String,
    name: String,
    category_id: String,
    category_name: String,
    description: String,
    evidence: String,
}
impl OperationCache {
    pub(super) fn enrich(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        summary: &mut WorkSummary,
        events: &[&WorkObservation],
    ) -> Result<()> {
        let Some(declaration) = native_declaration(summary, events) else {
            return Ok(());
        };
        // These are synchronous host-tool invocation callbacks, including launcher
        // results. This says nothing about completion of a launched background worker.
        if !summary.context.labels.contains_key("nexus.flow.mode")
            && !summary
                .conflicts
                .iter()
                .any(|gap| gap == "conflicting_invocation_modes")
        {
            summary
                .context
                .labels
                .insert("nexus.flow.mode".into(), "synchronous".into());
        }
        let Some(host) = summary.context.host_session_id.as_deref() else {
            return Ok(());
        };
        let agent = &declaration.provenance.agent;
        let session = crate::analytics::identity(&[project, agent, host]);
        if self.session.as_deref() != Some(&session) {
            self.load(connection, project, agent, host, &session)?;
        }
        if let Some(operations) = self.operations.get(summary.subject.id()) {
            summary.operations = operations.clone();
            let names: Vec<_> = operations
                .iter()
                .take(3)
                .map(|operation| operation.name.as_str())
                .collect();
            summary.title = names.join("; ");
            if operations.len() > 3 {
                summary
                    .title
                    .push_str(&format!(" (+{} operations)", operations.len() - 3));
            }
        } else if self.partial {
            summary.gaps.push("tool_operation_projection_limit".into());
        }
        Ok(())
    }
    fn load(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        agent: &str,
        host: &str,
        session: &str,
    ) -> Result<()> {
        let mut rows = operation_rows(connection, project, session)?;
        self.session = Some(session.into());
        self.operations.clear();
        self.partial = rows.len() > ROW_LIMIT as usize;
        if self.partial {
            let cut = rows.pop().unwrap().invocation;
            rows.retain(|row| row.invocation != cut);
        }
        let mut bytes = 0;
        for row in rows {
            let id = stable_work_id(&[agent, host, &row.invocation]);
            let operation = WorkToolOperation {
                name: row.name,
                category_id: row.category_id,
                category_name: row.category_name,
                description: row.description,
                evidence: row.evidence,
            };
            bytes += serde_json::to_vec(&operation)?.len() + id.len();
            if bytes > 4 * 1024 * 1024 {
                self.operations.remove(&id);
                self.partial = true;
                break;
            }
            self.operations.entry(id).or_default().push(operation);
        }
        Ok(())
    }
}
fn native_declaration<'a>(
    summary: &WorkSummary,
    events: &[&'a WorkObservation],
) -> Option<&'a WorkObservation> {
    if summary.kind != "tool" {
        return None;
    }
    events.iter().copied().find(|event| {
        event.input.producer.name == "nexus-companion"
            && event.input.method == "companion_callback_wall_clock"
            && matches!(event.input.body, WorkEvent::ActivityDeclared { ref kind, .. } if kind == "tool")
    })
}

fn operation_rows(
    connection: &mut DbConnection,
    project: &str,
    session: &str,
) -> Result<Vec<OperationRow>> {
    let rows = actions::table
        .inner_join(calls::table.on(actions::call_id.eq(calls::id)))
        .inner_join(categories::table.on(actions::category_id.eq(categories::id)))
        .filter(calls::project_id.eq(project))
        .filter(actions::project_id.eq(project))
        .filter(calls::session_key.eq(session))
        .order((calls::invocation_id.asc(), actions::id.asc()))
        .select((
            calls::invocation_id,
            actions::name,
            actions::category_id,
            categories::name,
            actions::description,
            actions::evidence,
        ))
        .limit(ROW_LIMIT + 1)
        .load::<OperationRow>(connection)?;
    Ok(rows)
}
