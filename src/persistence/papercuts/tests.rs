// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
use super::super::schema::papercuts;
use super::super::Store;
use super::records::reporter_token_hash;
use super::{PapercutDuplicateProposal, PapercutReviewAttemptRecord};
use crate::papercuts::{
    DuplicateReviewStatus, PapercutActor, PapercutLifecycleTransition,
    PapercutReviewAttemptOutcome, PapercutReviewDecision,
};
use crate::persistence::test_support::{actor, capture_queries};
use diesel::prelude::*;

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
fn report_rejects_unbounded_actor_metadata() {
    let mut store = Store::open_memory().unwrap();
    let mut oversized: PapercutActor = actor("reporter");
    oversized.agent = "a".repeat(super::records::MAX_METADATA_BYTES + 1);

    let error = store
        .report_papercut("project", "Bound actor", "", &oversized)
        .unwrap_err();

    assert!(error.to_string().contains("256-byte limit"));
}

#[test]
fn report_rejects_oversized_raw_whitespace_before_normalizing() {
    let mut store = Store::open_memory().unwrap();
    let oversized = " ".repeat(super::records::MAX_TEXT_BYTES + 1);

    let error = store
        .report_papercut(
            "project",
            "Bound normalization",
            &oversized,
            &actor("reporter"),
        )
        .unwrap_err();

    assert!(error.to_string().contains("8192-byte limit"));
}

#[test]
fn ordinary_open_list_aggregates_votes_without_loading_history_rows() {
    let mut store = Store::open_memory().unwrap();
    let active = store
        .report_papercut("project", "Active friction", "", &actor("active-a"))
        .unwrap();
    store
        .upvote_papercut(
            "project",
            &active.papercut.id,
            Some("Still happening"),
            &actor("active-b"),
        )
        .unwrap();
    let resolved = store
        .report_papercut("project", "Historical friction", "", &actor("resolved-a"))
        .unwrap();
    store
        .remove_papercut(
            "project",
            &resolved.papercut.id,
            "Fixed",
            &actor("resolver"),
        )
        .unwrap();

    let queries = capture_queries(&mut store.connection);

    let listed = store.open_papercuts("project").unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, active.papercut.id);
    assert_eq!(listed[0].upvote_count, 2);
    assert!(listed[0].last_upvoted_at.is_some());
    let upvote_queries = queries
        .lock()
        .unwrap()
        .iter()
        .map(|query| query.to_ascii_lowercase())
        .filter(|query| query.contains("papercut_upvotes"))
        .collect::<Vec<_>>();
    assert!(
        upvote_queries
            .iter()
            .any(|query| query.contains("count(") && query.contains("max(")),
        "ordinary list did not aggregate upvotes in the database: {upvote_queries:#?}"
    );
    assert!(
        upvote_queries.iter().all(|query| !query.contains("note")
            && !query.contains("reporter_agent")
            && !query.contains("reporter_session_id")),
        "ordinary list loaded full historical upvote rows: {upvote_queries:#?}"
    );

    let registry = store.papercut_registry(Some("project")).unwrap();
    assert_eq!(registry.papercuts.len(), 2);
    assert_eq!(registry.upvotes.len(), 3);
}

#[test]
fn ordinary_open_list_has_a_recent_activity_cap_but_dump_remains_complete() {
    let mut store = Store::open_memory().unwrap();
    for index in 0..33 {
        store
            .report_papercut(
                "project",
                &format!("Friction {index:03}"),
                "Still cumbersome",
                &actor(&format!("reporter-{index:03}")),
            )
            .unwrap();
    }

    let listed = store.open_papercuts_window("project", None).unwrap();
    let registry = store.papercut_registry(Some("project")).unwrap();

    assert_eq!(listed.items.len(), 32);
    assert!(listed.truncated);
    assert_eq!(registry.papercuts.len(), 33);
    assert_eq!(registry.upvotes.len(), 33);
}

#[test]
fn open_list_cursor_reaches_every_older_report_in_bounded_pages() {
    let mut store = Store::open_memory().unwrap();
    for index in 0..65 {
        store
            .report_papercut(
                "project",
                &format!("Friction {index:03}"),
                "Still cumbersome",
                &actor(&format!("reporter-{index:03}")),
            )
            .unwrap();
    }
    diesel::update(papercuts::table.filter(papercuts::project_id.eq("project")))
        .set(papercuts::updated_at.eq("2026-10-19T12:00:00+00:00"))
        .execute(&mut store.connection)
        .unwrap();

    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = store
            .open_papercuts_window("project", after.as_ref())
            .unwrap();
        assert!(page.items.len() <= 32);
        all.extend(page.items.iter().map(|cut| cut.id.clone()));
        if !page.truncated {
            break;
        }
        after = page
            .items
            .last()
            .map(crate::coordination::api::PapercutListCursor::after);
    }

    let unique = all.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(all.len(), 65);
    assert_eq!(unique.len(), 65);
}

#[test]
fn complete_registry_reconstructs_a_maximum_length_resolution() {
    let mut store = Store::open_memory().unwrap();
    let reported = store
        .report_papercut("project", "Maximum resolution", "", &actor("reporter"))
        .unwrap();
    let resolution = "r".repeat(super::records::MAX_TEXT_BYTES);

    store
        .remove_papercut(
            "project",
            &reported.papercut.id,
            &resolution,
            &actor("resolver"),
        )
        .unwrap();
    let registry = store.papercut_registry(Some("project")).unwrap();

    assert_eq!(registry.papercuts.len(), 1);
    assert_eq!(
        registry.papercuts[0].resolution.as_deref(),
        Some(resolution.as_str())
    );
    assert_eq!(registry.lifecycle_events.len(), 1);
    assert_eq!(
        registry.lifecycle_events[0].papercut_id,
        reported.papercut.id
    );
    assert_eq!(
        registry.lifecycle_events[0].transition,
        PapercutLifecycleTransition::Resolved { resolution }
    );
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

#[test]
fn automated_review_candidates_are_bounded_and_exclude_the_source() {
    let mut store = Store::open_memory().unwrap();
    let mut ids = Vec::new();
    for ordinal in 0..=super::registry::MAX_PAPERCUT_REVIEW_CANDIDATES {
        let reported = store
            .report_papercut(
                "project",
                &format!("Distinct issue {ordinal}"),
                "Bounded candidate selection",
                &actor("reporter"),
            )
            .unwrap();
        ids.push(reported.papercut.id);
    }
    let source_id = ids.last().unwrap();
    diesel::update(papercuts::table.find(&ids[0]))
        .set(papercuts::creator_agent.eq("legacy-agent".repeat(128 * 1024)))
        .execute(&mut store.connection)
        .unwrap();

    let candidates = store
        .papercut_review_candidates("project", source_id)
        .unwrap();

    assert_eq!(
        candidates.len(),
        super::registry::MAX_PAPERCUT_REVIEW_CANDIDATES
    );
    assert!(candidates
        .iter()
        .all(|candidate| candidate.id != *source_id));
    let encoded = serde_json::to_vec(&candidates).unwrap();
    assert!(encoded.len() < 128 * 1024);
    assert!(!String::from_utf8(encoded).unwrap().contains("legacy-agent"));
}

fn report_with_encounters(store: &mut Store, title: &str, encounters: usize) {
    let cut = store
        .report_papercut("project", title, "", &actor(&format!("{title} reporter")))
        .unwrap()
        .papercut;
    for vote in 0..encounters {
        let voter = actor(&format!("{title} voter {vote}"));
        store
            .upvote_papercut("project", &cut.id, None, &voter)
            .unwrap();
    }
}

fn every_open_page(store: &mut Store) -> Vec<crate::papercuts::Papercut> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = store
            .open_papercuts_window("project", after.as_ref())
            .unwrap();
        assert!(page.items.len() <= 32);
        all.extend(page.items.iter().cloned());
        if !page.truncated {
            return all;
        }
        after = page
            .items
            .last()
            .map(crate::coordination::api::PapercutListCursor::after);
    }
}

#[test]
fn open_list_ranks_by_encounters_then_recent_activity_across_stable_pages() {
    let mut store = Store::open_memory().unwrap();
    for index in 0..41 {
        report_with_encounters(&mut store, &format!("Friction {index:03}"), index % 4);
    }
    diesel::update(papercuts::table.filter(papercuts::title.eq("Friction 001")))
        .set(papercuts::updated_at.eq("2099-01-01T00:00:00+00:00"))
        .execute(&mut store.connection)
        .unwrap();

    let all = every_open_page(&mut store);

    let unique = all
        .iter()
        .map(|cut| &cut.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!((all.len(), unique.len()), (41, 41));
    let rank = |cut: &crate::papercuts::Papercut| {
        let reverse = (
            std::cmp::Reverse(cut.upvote_count),
            std::cmp::Reverse(cut.updated_at),
        );
        (reverse, cut.id.clone())
    };
    assert!(all.windows(2).all(|pair| rank(&pair[0]) < rank(&pair[1])));
    let counts = all.iter().map(|cut| cut.upvote_count);
    assert_eq!(counts.clone().max(), Some(4));
    assert_eq!(counts.min(), Some(1));
    let recent = all
        .iter()
        .position(|cut| cut.title == "Friction 001")
        .unwrap();
    assert!(all[..recent].iter().all(|cut| cut.upvote_count > 2));
    assert_eq!(all[recent].upvote_count, 2);
}

/// A canonical papercut, one encounter, then a duplicate consolidated into it and encountered.
fn consolidated_papercuts(store: &mut Store) -> (String, String) {
    let canonical = store
        .report_papercut("project", "Slow build", "Takes ages", &actor("reporter-a"))
        .unwrap()
        .papercut
        .id;
    let again = Some("Again on CI");
    store
        .upvote_papercut("project", &canonical, again, &actor("voter-b"))
        .unwrap();
    let duplicate = store
        .report_papercut(
            "project",
            "Build is slow",
            "Cache misses",
            &actor("reporter-c"),
        )
        .unwrap();
    let reviewer = actor("reviewer");
    let review = store
        .propose_papercut_duplicate(PapercutDuplicateProposal {
            project_id: "project",
            papercut_id: &duplicate.papercut.id,
            canonical_id: &canonical,
            rationale: "Same slowness",
            question: "Is this the slow build?",
            reviewer: &reviewer,
            provider: Some("claude"),
            model_id: Some("test-model"),
        })
        .unwrap();
    let token = duplicate.reporter_token.as_deref().unwrap();
    let approve = PapercutReviewDecision::Approve;
    store
        .respond_to_papercut_review(&review.id, approve, "Yes", token, &actor("reporter-c"))
        .unwrap();
    let duplicate = duplicate.papercut.id;
    store
        .upvote_papercut("project", &duplicate, Some("Still slow"), &actor("voter-d"))
        .unwrap();
    (canonical, duplicate)
}

#[test]
fn detail_lists_original_report_encounters_and_consolidated_duplicates_chronologically() {
    let mut store = Store::open_memory().unwrap();
    let (canonical, duplicate) = consolidated_papercuts(&mut store);

    let detail = store.papercut_detail("project", &duplicate).unwrap();

    assert_eq!(detail.papercut.id, canonical);
    assert_eq!(detail.papercut.upvote_count, 4);
    assert!(!detail.truncated);
    let summary = detail
        .reports
        .iter()
        .map(|report| {
            let reported = if report.reported_papercut_id == canonical {
                "canonical"
            } else {
                "duplicate"
            };
            let kind = serde_json::to_value(report.kind).unwrap();
            let text = [&report.title, &report.description, &report.note]
                .map(|value| value.as_deref().unwrap_or("-"));
            format!(
                "{} {reported} {} {}",
                kind.as_str().unwrap(),
                text.join("|"),
                report.reporter_session_id
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            "report canonical Slow build|Takes ages|- reporter-a",
            "encounter canonical -|-|Again on CI voter-b",
            "report duplicate Build is slow|Cache misses|- reporter-c",
            "encounter duplicate -|-|Still slow voter-d",
        ]
    );
    assert!(detail
        .reports
        .windows(2)
        .all(|pair| pair[0].created_at <= pair[1].created_at));
}

#[test]
fn detail_keeps_the_newest_bounded_encounters_and_discloses_truncation() {
    let maximum = super::detail::MAX_PAPERCUT_DETAIL_REPORTS;
    let mut store = Store::open_memory().unwrap();
    let cut = store
        .report_papercut("project", "Noisy friction", "", &actor("reporter"))
        .unwrap()
        .papercut;
    for vote in 0..maximum {
        store
            .upvote_papercut(
                "project",
                &cut.id,
                Some(&format!("encounter {vote:03}")),
                &actor(&format!("voter-{vote:03}")),
            )
            .unwrap();
    }

    let detail = store.papercut_detail("project", &cut.id).unwrap();

    assert!(detail.truncated);
    assert_eq!(detail.reports.len(), maximum);
    assert_eq!(detail.reports[0].note.as_deref(), Some("encounter 000"));
    let newest = format!("encounter {:03}", maximum - 1);
    assert_eq!(
        detail.reports.last().unwrap().note.as_deref(),
        Some(newest.as_str())
    );
}

#[test]
fn detail_is_bounded_after_json_escaping() {
    let mut store = Store::open_memory().unwrap();
    let cut = store
        .report_papercut("project", "Escaped friction", "", &actor("reporter"))
        .unwrap()
        .papercut;
    let escaped = "\u{1}".repeat(super::records::MAX_TEXT_BYTES);
    for vote in 0..super::detail::MAX_PAPERCUT_DETAIL_REPORTS {
        store
            .upvote_papercut(
                "project",
                &cut.id,
                Some(&escaped),
                &actor(&format!("voter-{vote:03}")),
            )
            .unwrap();
    }

    let detail = store.papercut_detail("project", &cut.id).unwrap();

    assert!(detail.truncated);
    assert!(!detail.reports.is_empty());
    assert!(serde_json::to_vec(&detail).unwrap().len() <= super::detail::MAX_PAPERCUT_DETAIL_BYTES);
}
