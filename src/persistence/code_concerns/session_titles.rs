// @feature code-concerns
// @feature persistence
// @feature work-history
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::code_concerns::{ConcernObservation, ConcernResponse, ConcernSessionTitle};
use crate::persistence::{backend::DbConnection, session_names, work_history};
use crate::work_history::WorkSubject;
use anyhow::Result;
use std::collections::BTreeSet;

/// Current session names are read-time metadata, never changes to recorded evidence.
pub(super) fn enrich(
    connection: &mut DbConnection,
    project: &str,
    mut response: ConcernResponse,
) -> Result<ConcernResponse> {
    match &mut response {
        ConcernResponse::Detail {
            concern: Some(detail),
        } => detail.session_titles = titles(connection, project, &detail.history)?,
        ConcernResponse::History {
            observations,
            session_titles,
            ..
        } => *session_titles = titles(connection, project, observations)?,
        _ => {}
    }
    Ok(response)
}
fn titles(
    connection: &mut DbConnection,
    project: &str,
    observations: &[ConcernObservation],
) -> Result<Vec<ConcernSessionTitle>> {
    let identities = observations
        .iter()
        .map(|observation| {
            let actor = observation
                .source
                .as_ref()
                .map_or(&observation.provenance, |source| &source.provenance);
            let context = &observation.input.context;
            (
                actor.agent.clone(),
                context.host_session_id.clone(),
                context.work_session_id.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    identities
        .into_iter()
        .map(|(agent, host_session_id, work_session_id)| {
            let host_title = host_session_id
                .as_deref()
                .map(|id| session_names::host_title(connection, project, &agent, id))
                .transpose()?
                .flatten();
            let work_title = work_session_id
                .as_ref()
                .map(|id| {
                    work_history::session_title(
                        connection,
                        project,
                        &WorkSubject::Session { id: id.clone() },
                    )
                })
                .transpose()?
                .flatten()
                .map(|summary| summary.title);
            Ok(ConcernSessionTitle {
                agent,
                host_session_id,
                work_session_id,
                host_title,
                work_title,
            })
        })
        .collect()
}
