// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::publication::{Publication, WriteContext};
use crate::code_concerns::*;
use crate::persistence::Store;
use anyhow::Result;

impl Store {
    pub(super) fn record_concerns(
        &mut self,
        project: &str,
        actor: &ConcernActor,
        inputs: &[ConcernObservationInput],
        imports: &[ConcernObservation],
        token: Option<&String>,
    ) -> Result<ConcernResponse> {
        let context = WriteContext {
            project,
            actor,
            imports,
            token: token.map(String::as_str),
        };
        let publication = Publication::prepare(context, inputs)?;
        self.transaction(|connection| publication.publish(connection))
    }
}
pub(super) use super::declarations::declare;
pub(super) use super::indexing::index;
