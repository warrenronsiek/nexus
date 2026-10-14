// @feature papercuts
// @spec docs/features/papercuts.md
mod domain;

pub(crate) use domain::PapercutReviewCandidate;
pub use domain::{
    DuplicateReview, DuplicateReviewStatus, Papercut, PapercutActor, PapercutLifecycleEvent,
    PapercutLifecycleTransition, PapercutRegistry, PapercutReportResult, PapercutReviewAttempt,
    PapercutReviewAttemptOutcome, PapercutReviewDecision, PapercutStatus, PapercutUpvote,
};
