// @feature papercuts
// @spec docs/features/papercuts.md
mod domain;

pub use domain::{
    DuplicateReview, DuplicateReviewStatus, Papercut, PapercutActor, PapercutLifecycleEvent,
    PapercutLifecycleTransition, PapercutRegistry, PapercutReportResult, PapercutReviewAttempt,
    PapercutReviewAttemptOutcome, PapercutReviewDecision, PapercutStatus, PapercutUpvote,
};
