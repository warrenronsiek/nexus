// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
mod mutations;
mod records;
mod registry;
mod reviews;

#[cfg(test)]
mod tests;

use crate::papercuts::{PapercutActor, PapercutReviewAttemptOutcome};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PapercutDuplicateProposal<'a> {
    pub project_id: &'a str,
    pub papercut_id: &'a str,
    pub canonical_id: &'a str,
    pub rationale: &'a str,
    pub question: &'a str,
    pub reviewer: &'a PapercutActor,
    pub provider: Option<&'a str>,
    pub model_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PapercutReviewAttemptRecord<'a> {
    pub project_id: &'a str,
    pub papercut_id: &'a str,
    pub outcome: PapercutReviewAttemptOutcome,
    pub provider: &'a str,
    pub model_id: Option<&'a str>,
    pub detail: &'a str,
    pub config_hash: &'a str,
}
