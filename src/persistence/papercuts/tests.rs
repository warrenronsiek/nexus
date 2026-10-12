// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
use super::super::schema::papercuts;
use super::super::Store;
use super::records::reporter_token_hash;
use super::{PapercutDuplicateProposal, PapercutReviewAttemptRecord};
use crate::papercuts::{
    DuplicateReviewStatus, PapercutActor, PapercutReviewAttemptOutcome, PapercutReviewDecision,
};
use diesel::prelude::*;

fn actor(session_id: &str) -> PapercutActor {
    PapercutActor {
        agent: "codex".to_owned(),
        session_id: session_id.to_owned(),
        model_id: Some("test-model".to_owned()),
        config_hash: "test-config".to_owned(),
    }
}

#[test]
fn new_report_returns_token_but_persists_and_exposes_only_its_hash() {
    let mut store = Store::open_memory().unwrap();
    let first = store
        .report_papercut("project", "Flaky test", "", &actor("reporter-a"))
        .unwrap();
    let token = first.reporter_token.as_deref().unwrap();
    let stored_hash = papercuts::table
        .find(&first.papercut.id)
        .select(papercuts::reporter_token_hash)
        .first::<String>(&mut store.connection)
        .unwrap();

    assert_ne!(stored_hash, token);
    assert_eq!(stored_hash, reporter_token_hash(token));
    assert!(serde_json::to_value(&first.papercut)
        .unwrap()
        .get("reporter_token_hash")
        .is_none());

    let exact = store
        .report_papercut("project", " flaky TEST ", "", &actor("reporter-b"))
        .unwrap();
    assert!(!exact.created);
    assert!(exact.reporter_token.is_none());
}

#[test]
fn duplicate_response_is_authorized_by_source_reporter_token() {
    let mut store = Store::open_memory().unwrap();
    let canonical = store
        .report_papercut("project", "Socket test hangs", "", &actor("reporter-a"))
        .unwrap();
    let source = store
        .report_papercut(
            "project",
            "Validation never completes",
            "",
            &actor("reporter-b"),
        )
        .unwrap();
    let reviewer = actor("reviewer");
    let review = store
        .propose_papercut_duplicate(PapercutDuplicateProposal {
            project_id: "project",
            papercut_id: &source.papercut.id,
            canonical_id: &canonical.papercut.id,
            rationale: "Same hang",
            question: "Is this the socket test?",
            reviewer: &reviewer,
            provider: Some("claude"),
            model_id: Some("test-model"),
        })
        .unwrap();

    let error = store
        .respond_to_papercut_review(
            &review.id,
            PapercutReviewDecision::Approve,
            "Yes",
            "wrong-token",
            &actor("reporter-b"),
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("invalid duplicate review reporter token"));

    let approved = store
        .respond_to_papercut_review(
            &review.id,
            PapercutReviewDecision::Approve,
            "Yes",
            source.reporter_token.as_deref().unwrap(),
            &actor("replacement-session"),
        )
        .unwrap();
    assert_eq!(approved.status, DuplicateReviewStatus::Approved);
}

#[test]
fn automated_review_attempt_and_event_are_persisted_together() {
    let mut store = Store::open_memory().unwrap();
    let reported = store
        .report_papercut("project", "Slow checkout", "", &actor("reporter"))
        .unwrap();

    store
        .record_papercut_review_event(PapercutReviewAttemptRecord {
            project_id: "project",
            papercut_id: &reported.papercut.id,
            outcome: PapercutReviewAttemptOutcome::Distinct,
            provider: "claude",
            model_id: Some("test-model"),
            detail: "No matching issue",
            config_hash: "test-config",
        })
        .unwrap();

    let registry = store.papercut_registry(Some("project")).unwrap();
    assert_eq!(registry.review_attempts.len(), 1);
    assert_eq!(
        registry.review_attempts[0].outcome,
        PapercutReviewAttemptOutcome::Distinct
    );
    assert!(store
        .list_events(Some("project"), 10)
        .unwrap()
        .iter()
        .any(|event| event.kind == "papercut_reviewed"));
}
