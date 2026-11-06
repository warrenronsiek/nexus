// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
// @boundary disposable-postgres: PostgreSQL builds use only NEXUS_TEST_DATABASE_URL in a rolled-back transaction.
use super::*;
use crate::coordination::domain::{
    Advisory, Claim, ClaimRelease, ConflictScope, Operation, PathIntent,
};
use crate::persistence::models::NewAdvisory;

/// One isolated project with two sessions, on either supported backend.
struct History {
    store: Store,
    project: String,
}

#[cfg(feature = "sqlite")]
fn open_store() -> Store {
    Store::open_memory().unwrap()
}

#[cfg(feature = "postgres")]
fn open_store() -> Store {
    let url = std::env::var("NEXUS_TEST_DATABASE_URL").expect("disposable PostgreSQL URL required");
    let mut store = Store::connect(&url).unwrap();
    store.connection.begin_test_transaction().unwrap();
    store
}

impl History {
    fn new() -> Self {
        let mut history = Self {
            store: open_store(),
            project: format!("history-{}", uuid::Uuid::new_v4()),
        };
        for (session, agent) in [("session-one", "codex"), ("session-two", "claude")] {
            history.session(session, agent);
        }
        history
    }

    fn session(&mut self, session: &str, agent: &str) {
        self.store
            .touch_session(&self.project, session, agent, None, "config")
            .unwrap();
    }

    fn claim(&mut self, session: &str, tool: &str, path: &str, ttl_seconds: i64) -> Claim {
        let intent = PathIntent {
            path: path.into(),
            operation: Operation::Write,
            line_start: None,
            line_end: None,
        };
        self.store
            .insert_claim(&self.project, session, tool, &intent, ttl_seconds)
            .unwrap()
    }

    /// Records the overlap the way a pre-tool hook does: the requester owns the left claim.
    fn overlap(&mut self, requester: &Claim, existing: &Claim) -> String {
        let advisory = Advisory {
            id: uuid::Uuid::new_v4().to_string(),
            severity: Severity::Warning,
            kind: "path_overlap".into(),
            message: format!("named counterpart has an active claim on {}", existing.path),
            path: existing.path.clone(),
            other_session_id: existing.session_id.clone(),
            conflict_id: None,
        };
        let tool = requester.tool_use_id.clone();
        self.store
            .record_conflict(&self.project, requester, existing, advisory, &tool)
            .unwrap()
            .conflict_id
            .unwrap()
    }

    fn timeline(&mut self, session: &str, limit: usize) -> SessionTimeline {
        self.store
            .session_timeline(&self.project, session, limit)
            .unwrap()
    }

    fn release(&mut self, session: &str) {
        self.store
            .release_claims(&self.project, session, &ClaimRelease::Session)
            .unwrap();
    }

    fn end_stale(&mut self) -> usize {
        self.store.end_stale_conflicts(Some(&self.project)).unwrap()
    }

    fn conflict(&mut self, id: &str) -> crate::coordination::domain::ConflictRecord {
        self.store.conflict_by_id(id).unwrap().unwrap()
    }

    fn open_conflicts(&mut self) -> usize {
        self.store
            .list_conflicts(Some(&self.project), ConflictScope::Open)
            .unwrap()
            .len()
    }

    /// The left claim belongs to `requester`, which therefore receives the advisory.
    fn overlap_on(&mut self, path: &str, requester: &str, existing: &str) -> String {
        let owner = self.claim(existing, &format!("{existing}-{path}"), path, 600);
        let notified = self.claim(requester, &format!("{requester}-{path}"), path, 600);
        self.overlap(&notified, &owner)
    }
}

type Shape = (TimelineEntryKind, TimelineDirection, Option<String>);

fn shape(timeline: &SessionTimeline) -> Vec<Shape> {
    timeline
        .entries
        .iter()
        .map(|entry| {
            let counterpart = entry.counterpart.as_ref();
            (
                entry.kind,
                entry.direction,
                counterpart.map(|counterpart| counterpart.session_id.clone()),
            )
        })
        .collect()
}

fn expected(direction: TimelineDirection, counterpart: &str) -> Vec<Shape> {
    [
        TimelineEntryKind::ConflictDetected,
        TimelineEntryKind::Advisory,
    ]
    .into_iter()
    .map(|kind| (kind, direction, Some(counterpart.to_owned())))
    .collect()
}

#[test]
fn timeline_shows_both_directions_with_named_counterparts() {
    let mut history = History::new();
    let conflict_id = history.overlap_on("src/a.rs", "session-two", "session-one");

    let notified = history.timeline("session-two", 500);
    let agent = notified
        .session
        .as_ref()
        .map(|session| session.agent.as_str());
    assert_eq!(agent, Some("claude"));
    assert_eq!(
        shape(&notified),
        expected(TimelineDirection::Received, "session-one")
    );
    let delivered = &notified.entries[1];
    assert_eq!(delivered.counterpart.as_ref().unwrap().agent, "codex");
    assert_eq!(delivered.conflict_id.as_deref(), Some(conflict_id.as_str()));
    assert_eq!(delivered.status, Some(ConflictStatus::Open));
    assert_eq!(delivered.path, "src/a.rs");
    assert!(!notified.truncated);

    let owner = history.timeline("session-one", 500);
    assert_eq!(
        shape(&owner),
        expected(TimelineDirection::Sent, "session-two")
    );
    let counterpart = owner.entries[1].counterpart.as_ref().unwrap();
    assert_eq!(counterpart.agent, "claude");
}

#[test]
fn missing_sessions_have_an_empty_timeline() {
    let mut history = History::new();
    let absent = history.timeline("absent", 500);
    assert!(absent.session.is_none());
    assert!(absent.entries.is_empty());
    let elsewhere = history
        .store
        .session_timeline("another-project", "session-one", 500)
        .unwrap();
    assert!(elsewhere.session.is_none());
}

#[test]
fn released_claims_end_conflicts_and_add_a_resolution_entry() {
    let mut history = History::new();
    let conflict_id = history.overlap_on("src/a.rs", "session-two", "session-one");
    assert_eq!(history.end_stale(), 0);

    history.release("session-one");
    assert_eq!(history.end_stale(), 1);
    assert_eq!(history.end_stale(), 0);

    let conflict = history.conflict(&conflict_id);
    assert_eq!(conflict.status, ConflictStatus::Ended);
    assert!(conflict.updated_at >= conflict.created_at);
    assert_eq!(history.open_conflicts(), 0);
    let timeline = history.timeline("session-two", 500);
    let last = timeline.entries.last().unwrap();
    assert_eq!(last.kind, TimelineEntryKind::ConflictResolved);
    assert_eq!(last.at, conflict.updated_at);
    assert_eq!(last.status, Some(ConflictStatus::Ended));
    assert_eq!(last.counterpart.as_ref().unwrap().session_id, "session-one");
    assert!(last.message.contains("src/a.rs"), "{}", last.message);
}

#[test]
fn expired_claims_end_conflicts_without_any_release() {
    let mut history = History::new();
    let stale = history.claim("session-one", "tool-one", "src/a.rs", -60);
    let live = history.claim("session-two", "tool-two", "src/a.rs", 600);
    let conflict_id = history.overlap(&live, &stale);
    let observed = history.conflict(&conflict_id);

    assert_eq!(history.end_stale(), 1);

    let ended = history.conflict(&conflict_id);
    assert_eq!(ended.status, ConflictStatus::Ended);
    // The lease had already expired, so the overlap ended no later than its observation.
    assert_eq!(ended.updated_at, observed.updated_at);
}

#[test]
fn an_ended_overlap_reopens_on_any_recurrence() {
    let mut history = History::new();
    let second = history.claim("session-two", "tool-two", "src/a.rs", 600);
    let first = history.claim("session-one", "tool-one", "src/a.rs", 600);
    let conflict_id = history.overlap(&second, &first);
    history.release("session-one");
    history.end_stale();

    let renewed = history.claim("session-one", "tool-three", "src/a.rs", 600);
    assert_eq!(history.overlap(&renewed, &second), conflict_id);

    assert_eq!(history.conflict(&conflict_id).status, ConflictStatus::Open);
    assert_eq!(history.open_conflicts(), 1);
}

#[test]
fn timelines_keep_the_newest_bounded_entries_oldest_first() {
    let mut history = History::new();
    let first = history.claim("session-one", "tool-one", "src/a.rs", 600);
    for index in 0..4 {
        let tool = format!("tool-two-{index}");
        let second = history.claim("session-two", &tool, "src/a.rs", 600);
        history.overlap(&second, &first);
    }

    let timeline = history.timeline("session-two", 3);
    assert!(timeline.truncated);
    assert_eq!(timeline.entries.len(), 3);
    let entries = &timeline.entries;
    assert!(entries.windows(2).all(|pair| pair[0].at <= pair[1].at));
    assert!(entries
        .iter()
        .all(|entry| entry.kind == TimelineEntryKind::Advisory));
}

#[test]
fn session_index_ranks_activity_and_counts_each_sessions_history() {
    let mut history = History::new();
    history.session("session-quiet", "pi");
    history.overlap_on("src/a.rs", "session-two", "session-one");
    history.overlap_on("src/b.rs", "session-two", "session-one");

    let window = history.store.session_index(&history.project, 2).unwrap();
    assert!(window.truncated);
    let ranked: Vec<_> = window
        .items
        .iter()
        .map(|activity| {
            let id = activity.session.session_id.as_str();
            (id, activity.conflicts, activity.advisories)
        })
        .collect();
    // Conflict updates count as activity for the quiet owner of the existing claim.
    assert_eq!(ranked, [("session-one", 2, 0), ("session-two", 2, 2)]);
    assert!(window.items[0].last_activity_at > window.items[0].session.last_seen_at);

    let all = history.store.session_index(&history.project, 200).unwrap();
    assert!(!all.truncated);
    assert_eq!(all.items.len(), 3);
    assert_eq!(all.items[2].session.session_id, "session-quiet");
    assert_eq!(all.items[2].conflicts, 0);
}

#[test]
fn recent_counts_cover_only_the_last_seven_days() {
    let mut history = History::new();
    history.overlap_on("src/a.rs", "session-two", "session-one");
    let old = (Utc::now() - chrono::Duration::days(8)).to_rfc3339();
    diesel::update(conflicts::table.filter(conflicts::project_id.eq(&history.project)))
        .set(conflicts::created_at.eq(&old))
        .execute(&mut history.store.connection)
        .unwrap();
    let advisory_id = uuid::Uuid::new_v4().to_string();
    diesel::insert_into(advisories::table)
        .values(NewAdvisory {
            id: &advisory_id,
            project_id: &history.project,
            session_id: "session-one",
            tool_use_id: "old-tool",
            conflict_id: None,
            severity: "warning",
            kind: "path_overlap",
            path: "src/old.rs",
            message: "old",
            created_at: &old,
        })
        .execute(&mut history.store.connection)
        .unwrap();
    history.overlap_on("src/b.rs", "session-one", "session-two");

    let counts = history
        .store
        .dashboard_counts(Some(&history.project), 600)
        .unwrap();
    assert_eq!(counts.recent.recent_conflicts, 1);
    assert_eq!(counts.recent.recent_advisories, 2);
    let elsewhere = history
        .store
        .dashboard_counts(Some("another-project"), 600)
        .unwrap();
    assert_eq!(elsewhere.recent.recent_conflicts, 0);
}
