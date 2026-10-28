// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @entrypoint Store::concern_command
mod counts;
mod declarations;
mod duplicates;
mod indexing;
mod ingestion;
mod projection;
mod publication;
mod queries;
mod rows;
mod session_titles;
mod snapshot;
use crate::code_concerns::*;
use crate::persistence::Store;
use anyhow::Result;

impl Store {
    pub fn concern_command(
        &mut self,
        project: &str,
        actor: &ConcernActor,
        action: &ConcernAction,
    ) -> Result<ConcernResponse> {
        action
            .validate_read()
            .map_err(|error| ConcernRejection::Invalid(error.to_string()))?;
        match action {
            ConcernAction::Record { observations } => {
                self.record_concerns(project, actor, observations, &[], None)
            }
            ConcernAction::Import { observations } => {
                let inputs = observations
                    .iter()
                    .map(|v| v.input.clone())
                    .collect::<Vec<_>>();
                self.record_concerns(project, actor, &inputs, observations, None)
            }
            ConcernAction::Respond {
                observation,
                reporter_token,
            } => self.record_concerns(
                project,
                actor,
                std::slice::from_ref(observation.as_ref()),
                &[],
                Some(reporter_token),
            ),
            _ => {
                let response = queries::read(&mut self.connection, project, action)?;
                session_titles::enrich(&mut self.connection, project, response)
            }
        }
    }
}
pub(super) use snapshot::rebuild_snapshot;
