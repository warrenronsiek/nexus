// @feature papercuts
// @feature runtime
// @spec docs/features/papercuts.md
// @spec docs/features/runtime.md
// @boundary dynamic-json
mod support;

use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Harness;

fn papercut<'a>(registry: &'a Value, id: &str) -> &'a Value {
    registry["papercuts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|papercut| papercut["id"] == id)
        .unwrap()
}

fn write_fake_reviewer(path: &Path, decision: Value) {
    let envelope = json!({"result": decision.to_string()}).to_string();
    std::fs::write(
        path,
        format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{envelope}'\n"),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn write_delayed_failing_reviewer(path: &Path, started: &Path) {
    std::fs::write(
        path,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf started > '{}'\nsleep 1\nexit 1\n",
            started.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn report(
    harness: &mut Harness,
    session_id: &str,
    agent: &str,
    title: &str,
    description: &str,
) -> Value {
    harness.call(
        "nexus_papercuts",
        json!({
            "action":"report",
            "session_id":session_id,
            "agent":agent,
            "title":title,
            "description":description
        }),
    )
}

fn dump(harness: &mut Harness) -> Value {
    harness.call(
        "nexus_papercuts",
        json!({"action":"dump","scope":"project"}),
    )
}

#[derive(Clone, Copy)]
enum ReviewRecordKind {
    Attempt,
    Proposal,
}

impl ReviewRecordKind {
    fn collection(self) -> &'static str {
        match self {
            Self::Attempt => "review_attempts",
            Self::Proposal => "duplicate_reviews",
        }
    }
}

fn wait_for_review_record(
    harness: &mut Harness,
    papercut_id: &str,
    kind: ReviewRecordKind,
    failure_context: &str,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let registry = dump(harness);
        if let Some(records) = registry["registry"][kind.collection()].as_array() {
            for record in records {
                if record["papercut_id"] == papercut_id {
                    return record.clone();
                }
            }
        }
        assert!(Instant::now() < deadline, "{failure_context}: {registry}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_failed_review_attempt(harness: &Harness, project_id: &str, papercut_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut store = harness.store();
    loop {
        let registry = store
            .papercut_registry(Some(project_id))
            .expect("read original project registry after its worktree moved");
        if let Some(attempt) = registry
            .review_attempts
            .iter()
            .find(|attempt| attempt.papercut_id == papercut_id)
        {
            return serde_json::to_value(attempt).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "failed review attempt was not persisted for project {project_id}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn respond_to_duplicate(
    harness: &mut Harness,
    review_id: &str,
    reporter_token: &str,
    decision: &str,
    response: &str,
) -> Value {
    harness.call(
        "nexus_papercuts",
        json!({
            "action":"respond",
            "session_id":"reporter-b",
            "agent":"codex",
            "review_id":review_id,
            "reporter_token":reporter_token,
            "decision":decision,
            "response":response
        }),
    )
}

fn assert_registry_history(
    dump: &Value,
    papercut_id: &str,
    bound_session_id: &str,
    bound_agent: &str,
) {
    assert_eq!(dump["ok"], true);
    assert_eq!(dump["action"], "dumped");
    assert_eq!(dump["registry"]["truncated"], false);
    assert_eq!(dump["registry"]["papercuts"].as_array().unwrap().len(), 1);
    assert_eq!(dump["registry"]["upvotes"].as_array().unwrap().len(), 3);
    assert_eq!(
        dump["registry"]["duplicate_reviews"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let dumped = papercut(&dump["registry"], papercut_id);
    assert_eq!(dumped["upvote_count"], 3);
    assert!(dumped["created_at"].is_string());
    assert!(dumped["updated_at"].is_string());
    assert!(dumped["last_upvoted_at"].is_string());

    let upvotes = dump["registry"]["upvotes"].as_array().unwrap();
    assert!(upvotes
        .iter()
        .all(|upvote| upvote["created_at"].is_string()));
    assert!(upvotes
        .iter()
        .all(|upvote| upvote["reporter_session_id"] == bound_session_id));
    assert!(upvotes
        .iter()
        .all(|upvote| upvote["reporter_agent"] == bound_agent));

    let lifecycle = dump["registry"]["lifecycle_events"]
        .as_array()
        .expect("complete dumps include typed lifecycle history");
    assert_eq!(lifecycle.len(), 2);
    assert_eq!(lifecycle[0]["transition"], "resolved");
    assert_eq!(lifecycle[0]["papercut_id"], papercut_id);
    assert_eq!(
        lifecycle[0]["resolution"],
        "Stopped sharing the fixed TCP port."
    );
    assert_eq!(lifecycle[0]["actor"]["agent"], bound_agent);
    assert_eq!(lifecycle[0]["actor"]["session_id"], bound_session_id);
    assert!(lifecycle[0]["created_at"].is_string());
    assert_eq!(lifecycle[1]["transition"], "reopened");
    assert_eq!(lifecycle[1]["papercut_id"], papercut_id);
    assert_eq!(lifecycle[1]["actor"]["agent"], bound_agent);
    assert_eq!(lifecycle[1]["actor"]["session_id"], bound_session_id);
    assert!(lifecycle[1]["created_at"].is_string());
}

fn assert_consolidation_history(
    dump: &Value,
    source_id: &str,
    canonical_id: &str,
    bound_session_id: &str,
    bound_agent: &str,
) {
    let consolidation = dump["registry"]["lifecycle_events"]
        .as_array()
        .expect("complete dumps include consolidation history")
        .iter()
        .find(|event| event["transition"] == "consolidated")
        .expect("approved duplicate records a consolidation transition");
    assert_eq!(consolidation["papercut_id"], source_id);
    assert_eq!(consolidation["canonical_id"], canonical_id);
    assert_eq!(
        consolidation["rationale"],
        "Both reports appear to describe the same hanging test process."
    );
    assert_eq!(consolidation["actor"]["agent"], bound_agent);
    assert_eq!(consolidation["actor"]["session_id"], bound_session_id);
    assert!(consolidation["created_at"].is_string());
}

#[test]
fn registry_collapses_exact_reports_keeps_vote_times_and_reopens_solved_issues() {
    let mut harness = Harness::start();
    harness.initialize_git();

    let first = report(
        &mut harness,
        "reporter-a",
        "codex",
        "Flaky integration test",
        "The socket test intermittently times out.",
    );
    assert_eq!(first["ok"], true);
    assert_eq!(first["action"], "reported");
    assert_eq!(first["created"], true);
    assert!(first["reporter_token"].is_string());
    let id = first["papercut"]["id"].as_str().unwrap().to_owned();
    let bound_session_id = first["papercut"]["creator_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let bound_agent = first["papercut"]["creator_agent"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(bound_session_id, "reporter-a");
    assert_ne!(bound_agent, "codex");

    let duplicate = report(
        &mut harness,
        "reporter-b",
        "claude",
        "  flaky   INTEGRATION test  ",
        " The socket test intermittently times out. ",
    );
    assert_eq!(duplicate["ok"], true);
    assert_eq!(duplicate["action"], "reported");
    assert_eq!(duplicate["created"], false);
    assert!(duplicate["reporter_token"].is_null());
    assert_eq!(duplicate["papercut"]["id"], id);
    assert_eq!(duplicate["papercut"]["upvote_count"], 2);

    let solved = harness.call(
        "nexus_papercuts",
        json!({
            "action":"remove",
            "session_id":"reporter-a",
            "agent":"codex",
            "papercut_id":id,
            "resolution":"Stopped sharing the fixed TCP port."
        }),
    );
    assert_eq!(solved["papercut"]["status"], "resolved");
    assert!(solved["papercut"]["resolved_at"].is_string());

    let recurring = harness.call(
        "nexus_papercuts",
        json!({
            "action":"upvote",
            "session_id":"reporter-c",
            "agent":"codex",
            "papercut_id":id,
            "note":"It failed again on a clean checkout."
        }),
    );
    assert_eq!(recurring["papercut"]["status"], "open");
    assert_eq!(recurring["papercut"]["upvote_count"], 3);
    assert!(recurring["papercut"]["resolved_at"].is_null());

    let dump = dump(&mut harness);
    assert_registry_history(&dump, &id, &bound_session_id, &bound_agent);
}

#[test]
fn duplicate_proposal_can_ask_the_reporter_before_consolidating() {
    let temporary = tempfile::tempdir().unwrap();
    let reviewer = temporary.path().join("fake-claude");
    write_fake_reviewer(
        &reviewer,
        json!({
            "decision":"distinct",
            "rationale":"There is no earlier issue to consolidate."
        }),
    );
    let mut harness = Harness::start_with_analyst(Some(&reviewer), 120);
    harness.initialize_git();
    let canonical = report(
        &mut harness,
        "reporter-a",
        "codex",
        "Tests sometimes hang",
        "Observed while running the full validation suite.",
    );
    assert_eq!(canonical["review_requested"], true);
    let canonical_id = canonical["papercut"]["id"].as_str().unwrap().to_owned();
    let bound_session_id = canonical["papercut"]["creator_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let bound_agent = canonical["papercut"]["creator_agent"]
        .as_str()
        .unwrap()
        .to_owned();
    let first_review = wait_for_review_record(
        &mut harness,
        &canonical_id,
        ReviewRecordKind::Attempt,
        "first reviewer invocation did not complete",
    );
    assert_eq!(first_review["outcome"], "distinct");
    assert!(first_review["created_at"].is_string());

    write_fake_reviewer(
        &reviewer,
        json!({
            "decision":"possible_duplicate",
            "canonical_id":canonical_id,
            "rationale":"Both reports appear to describe the same hanging test process.",
            "question":"Does your hang occur after the socket test starts?"
        }),
    );
    let duplicate = report(
        &mut harness,
        "reporter-b",
        "codex",
        "Validation never completes",
        "Observed while running the full validation suite.",
    );
    let duplicate_id = duplicate["papercut"]["id"].as_str().unwrap().to_owned();
    let reporter_token = duplicate["reporter_token"].as_str().unwrap().to_owned();
    let proposal = wait_for_review_record(
        &mut harness,
        &duplicate_id,
        ReviewRecordKind::Proposal,
        "reviewer did not persist its question",
    );
    assert_eq!(proposal["status"], "pending_reporter");
    assert_eq!(proposal["reporter_session_id"], bound_session_id);
    let review_id = proposal["id"].as_str().unwrap().to_owned();

    let unauthorized = respond_to_duplicate(
        &mut harness,
        &review_id,
        "not-the-original-report-token",
        "approve",
        "A different caller must not be able to approve this merge.",
    );
    assert_eq!(unauthorized["ok"], false);

    let approved = respond_to_duplicate(
        &mut harness,
        &review_id,
        &reporter_token,
        "approve",
        "Yes, it is the same socket-test hang.",
    );
    assert_eq!(approved["review"]["status"], "approved");
    assert!(approved["review"]["responded_at"].is_string());

    let dump = dump(&mut harness);
    assert_eq!(
        papercut(&dump["registry"], &duplicate_id)["status"],
        "consolidated"
    );
    assert_eq!(
        papercut(&dump["registry"], &duplicate_id)["consolidated_into"],
        canonical_id
    );
    assert_eq!(
        papercut(&dump["registry"], &canonical_id)["upvote_count"],
        2
    );
    assert_eq!(dump["registry"]["upvotes"].as_array().unwrap().len(), 2);
    assert!(dump["registry"]["upvotes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|upvote| upvote["papercut_id"] == canonical_id));
    assert_consolidation_history(
        &dump,
        &duplicate_id,
        &canonical_id,
        &bound_session_id,
        &bound_agent,
    );
}

#[test]
fn failed_background_review_keeps_resolved_identity_after_worktree_moves() {
    let temporary = tempfile::tempdir().unwrap();
    let reviewer = temporary.path().join("failing-claude");
    let started = temporary.path().join("review-started");
    write_delayed_failing_reviewer(&reviewer, &started);

    let mut harness = Harness::start_with_analyst(Some(&reviewer), 120);
    harness.initialize_git();
    let reported = report(
        &mut harness,
        "reporter-a",
        "codex",
        "Reviewer fails after worktree cleanup",
        "The failure record must retain the report's resolved project identity.",
    );
    assert_eq!(reported["review_requested"], true);
    assert!(reported.get("review_job").is_none());
    assert!(reported.get("reviewer_worktree").is_none());
    let papercut_id = reported["papercut"]["id"].as_str().unwrap().to_owned();
    let project_id = reported["papercut"]["project_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(
            Instant::now() < deadline,
            "background reviewer did not start"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let moved_worktree = harness.root.with_extension("moved");
    std::fs::rename(&harness.root, &moved_worktree).unwrap();

    let attempt = wait_for_failed_review_attempt(&harness, &project_id, &papercut_id);
    assert_eq!(attempt["project_id"], project_id);
    assert_eq!(attempt["papercut_id"], papercut_id);
    assert_eq!(attempt["outcome"], "failed");
    assert_eq!(attempt["detail"], "review workflow failed");
}
