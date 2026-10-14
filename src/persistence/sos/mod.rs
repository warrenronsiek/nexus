// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
mod mutations;
mod projections;
mod records;
mod triage;

mod lifecycle;

#[cfg(test)]
mod tests;

pub(crate) use records::{SosTriageApply, SosTriageFailure};
