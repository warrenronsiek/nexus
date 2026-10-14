// @feature sos
// @spec docs/features/sos.md
mod domain;

pub(crate) use domain::{
    required_text, validate_actor, SosTriageCandidate, SosTriageDecision, MAX_SOS_IDENTITY_BYTES,
    MAX_SOS_TEXT_BYTES, MAX_SOS_TITLE_BYTES,
};
pub use domain::{
    SosActor, SosIssue, SosIssueDetail, SosIssueLifecycleEvent, SosIssueStatus, SosIssueTransition,
    SosIssueWindow, SosMetadata, SosRegistry, SosReport, SosReportDraft, SosTriageAttempt,
    SosTriageOutcome,
};
