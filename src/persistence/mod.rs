// @feature persistence
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
mod conflicts;
mod dashboard;
mod memory;
mod migrations;
mod models;
mod papercuts;
mod schema;
mod store;
mod transaction;
mod usage;

pub(crate) use papercuts::{PapercutDuplicateProposal, PapercutReviewAttemptRecord};
pub use store::Store;
pub use usage::{
    CapabilityEvidence, CapabilityKind, CapabilityObservation, CapabilityOutcome, CapabilitySource,
    UsageCount, UsageSummary,
};
