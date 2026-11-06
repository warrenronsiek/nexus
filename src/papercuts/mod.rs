// @feature papercuts
// @spec docs/features/papercuts.md
mod domain;

pub(crate) use domain::PapercutReviewCandidate;
pub use domain::{
    DuplicateReview, DuplicateReviewStatus, Papercut, PapercutActor, PapercutDetail,
    PapercutLifecycleEvent, PapercutLifecycleTransition, PapercutRegistry, PapercutReport,
    PapercutReportKind, PapercutReportResult, PapercutReviewAttempt, PapercutReviewAttemptOutcome,
    PapercutReviewDecision, PapercutStatus, PapercutUpvote,
};
