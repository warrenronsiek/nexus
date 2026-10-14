// @feature persistence
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
mod backend;
mod conflicts;
mod dashboard;
mod memory;
mod migrations;
mod models;
mod papercuts;
#[cfg(feature = "postgres")]
mod readiness;
mod releases;
mod schema;
mod sos;
mod store;
#[cfg(test)]
mod test_support;
mod transaction;
mod transfer;
mod usage;
mod window;

pub(crate) use papercuts::{PapercutDuplicateProposal, PapercutReviewAttemptRecord};
#[cfg(feature = "postgres")]
pub(crate) use readiness::DatabaseReadinessProbe;
pub(crate) use sos::{SosTriageApply, SosTriageFailure};
pub use store::Store;
#[cfg(feature = "sqlite")]
pub use transfer::{backup_sqlite_database, snapshot_sqlite_database};
pub use transfer::{ImportOutcome, PortableSnapshot};
pub use usage::{
    CapabilityEvidence, CapabilityKind, CapabilityObservation, CapabilityOutcome, CapabilitySource,
    UsageCount, UsageSummary,
};
