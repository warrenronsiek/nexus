// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;

#[test]
fn compaction_attempts_apply_provider_cooldown_and_report_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance())
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance())
        .unwrap();
    let started_at = Utc::now();
    let job = store
        .ready_memory_compactions(&["codex"], started_at, 8)
        .unwrap()
        .remove(0);

    store
        .record_memory_compaction_attempt(&job, &failed_attempt("codex", false, started_at))
        .unwrap();
    assert!(!store
        .memory_provider_can_attempt(&job, "codex", started_at)
        .unwrap());
    assert!(store
        .memory_provider_can_attempt(&job, "codex", started_at + chrono::Duration::seconds(15),)
        .unwrap());

    store
        .record_memory_compaction_attempt(
            &job,
            &successful_attempt("claude", true, started_at + chrono::Duration::seconds(1)),
        )
        .unwrap();
    let health = store
        .memory_health(Some("project-a"), started_at + chrono::Duration::seconds(2))
        .unwrap();
    assert_eq!(health.pending_summaries, 1);
    assert_eq!(health.failed_attempts, 1);
    assert_eq!(health.cooling_down_attempts, 1);
    assert_eq!(health.fallback_uses, 1);
    assert!(!health.degraded);
    assert_eq!(health.providers.len(), 2);
    let codex = health
        .providers
        .iter()
        .find(|provider| provider.provider == "codex")
        .unwrap();
    assert_eq!(codex.consecutive_failures, 1);
    assert!(codex.cooling_down);
    assert!(codex.next_retry_at.is_some());
    let claude = health
        .providers
        .iter()
        .find(|provider| provider.provider == "claude")
        .unwrap();
    assert!(!claude.cooling_down);
    assert_eq!(claude.next_retry_at, None);
}

#[test]
fn completed_fallback_clears_active_cooldown_health() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance())
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance())
        .unwrap();
    let started_at = Utc::now();
    let job = store
        .ready_memory_compactions(&["codex", "claude"], started_at, 8)
        .unwrap()
        .remove(0);

    store
        .record_memory_compaction_attempt(&job, &failed_attempt("codex", false, started_at))
        .unwrap();
    assert!(matches!(
        store
            .insert_memory_summary(&job, "fallback summary", "claude", None)
            .unwrap(),
        MemorySummaryInsert::Inserted(_)
    ));
    store
        .record_memory_compaction_attempt(
            &job,
            &successful_attempt("claude", true, started_at + chrono::Duration::seconds(1)),
        )
        .unwrap();

    let health = store
        .memory_health(Some("project-a"), started_at + chrono::Duration::seconds(2))
        .unwrap();
    assert_eq!(health.pending_summaries, 0);
    assert_eq!(health.failed_attempts, 1);
    assert_eq!(health.cooling_down_attempts, 0);
    assert_eq!(health.fallback_uses, 1);
    assert!(!health.degraded);
    let codex = health
        .providers
        .iter()
        .find(|provider| provider.provider == "codex")
        .unwrap();
    assert_eq!(codex.next_retry_at, None);
    assert!(!codex.cooling_down);
}

#[test]
fn health_keeps_a_dual_failed_job_degraded_when_another_job_recovers() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    for ordinal in 0..4 {
        store
            .add_memory(&scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }
    let now = Utc::now();
    let jobs = store
        .ready_memory_compactions(&["codex", "claude"], now, 8)
        .unwrap();
    assert_eq!(jobs.len(), 2);

    for provider in ["codex", "claude"] {
        store
            .record_memory_compaction_attempt(
                &jobs[0],
                &failed_attempt(provider, provider == "claude", now),
            )
            .unwrap();
    }
    store
        .record_memory_compaction_attempt(
            &jobs[1],
            &failed_attempt("codex", false, now + chrono::Duration::seconds(1)),
        )
        .unwrap();
    store
        .record_memory_compaction_attempt(
            &jobs[1],
            &successful_attempt("claude", true, now + chrono::Duration::seconds(2)),
        )
        .unwrap();

    assert!(
        store
            .memory_health(Some("project-a"), now + chrono::Duration::seconds(3))
            .unwrap()
            .degraded
    );
}

#[test]
fn expired_dual_failure_remains_degraded_but_is_not_cooling_down() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance())
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance())
        .unwrap();
    let started_at = Utc::now();
    let job = store
        .ready_memory_compactions(&["codex", "claude"], started_at, 8)
        .unwrap()
        .remove(0);
    for provider in ["codex", "claude"] {
        store
            .record_memory_compaction_attempt(
                &job,
                &failed_attempt(provider, provider == "claude", started_at),
            )
            .unwrap();
    }

    let health = store
        .memory_health(
            Some("project-a"),
            started_at + chrono::Duration::seconds(15),
        )
        .unwrap();
    assert!(health.degraded);
    assert_eq!(health.pending_summaries, 1);
    assert_eq!(health.cooling_down_attempts, 0);
    assert!(health
        .providers
        .iter()
        .all(|provider| !provider.cooling_down));
}
