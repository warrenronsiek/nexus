// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @entrypoint Store::work_command
mod corrections;
mod dashboard;
mod ingestion;
mod lineage;
mod paging;
mod projection;
mod queries;
mod rows;
mod selection;
mod snapshot;
mod timeline;
mod timing;
use crate::persistence::Store;
use crate::work_history::*;
use anyhow::Result;

impl Store {
    pub fn work_command(
        &mut self,
        project: &str,
        actor: &WorkActor,
        action: &WorkAction,
    ) -> Result<WorkResponse> {
        match action {
            WorkAction::Record { observations } => {
                self.record_work(project, actor, observations, &[])
            }
            WorkAction::Import { observations } => {
                let inputs = observations
                    .iter()
                    .map(|item| item.input.clone())
                    .collect::<Vec<_>>();
                self.record_work(project, actor, &inputs, observations)
            }
            action => read_command(&mut self.connection, project, action),
        }
    }
}
fn read_command(
    connection: &mut crate::persistence::backend::DbConnection,
    project: &str,
    action: &WorkAction,
) -> Result<WorkResponse> {
    match action {
        WorkAction::List { query, .. } => {
            queries::subjects(connection, project, query, queries::SubjectScope::Sessions)
        }
        WorkAction::Activities { query, .. } => queries::subjects(
            connection,
            project,
            query,
            queries::SubjectScope::Activities,
        ),
        WorkAction::Dashboard {
            work_session_id, ..
        } => dashboard::dashboard(connection, project, work_session_id.as_deref()),
        WorkAction::Get { subject, .. } => Ok(WorkResponse::Detail {
            record: projection::detail(connection, project, subject)?.map(Box::new),
        }),
        WorkAction::Timeline {
            work_session_id,
            parent_activity_id,
            after,
            limit,
            ..
        } => timeline::timeline(
            connection,
            project,
            timeline::TimelineRequest {
                session: work_session_id,
                parent: parent_activity_id.as_deref(),
                after: after.as_deref(),
                limit: *limit,
            },
        ),
        WorkAction::Events {
            subject,
            after,
            limit,
            ..
        } => queries::events(connection, project, Some(subject), after.as_deref(), *limit),
        WorkAction::Export { after, limit, .. } => {
            queries::events(connection, project, None, after.as_deref(), *limit)
        }
        _ => unreachable!("recording dispatched before reads"),
    }
}

pub(super) use snapshot::rebuild_snapshot;
