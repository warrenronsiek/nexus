// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;

#[test]
fn complete_dyadic_frontiers_cover_histories_without_overlap_across_budgets() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    for history_len in 1..=24_u64 {
        let project_id = format!("history-{history_len}");
        let scope = MemoryScope::project(&project_id);
        for ordinal in 0..history_len {
            store
                .add_memory(&scope, &format!("m{ordinal}"), &provenance())
                .unwrap();
        }
        consolidate_all(&mut store);

        for max_items in 1..=history_len.min(8) as usize {
            assert_frontier_invariants(
                &mut store,
                &project_id,
                history_len,
                MemoryContextLimits {
                    max_items,
                    max_bytes: 16 * 1024,
                },
            );
        }
        for max_bytes in [3, 4, 8, 16] {
            assert_frontier_invariants(
                &mut store,
                &project_id,
                history_len,
                MemoryContextLimits {
                    max_items: 64,
                    max_bytes,
                },
            );
        }
    }
}

#[test]
fn compaction_batches_are_bounded_to_eight_jobs_from_one_level() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    for ordinal in 0..40 {
        store
            .add_memory(&scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }

    let first = store
        .ready_memory_compactions(&["codex"], Utc::now(), 100)
        .unwrap();
    assert_eq!(first.len(), 8);
    assert!(first.iter().all(|job| job.level == 1));
    for job in &first {
        store
            .insert_memory_summary(job, "pair", "codex", None)
            .unwrap();
    }
    let second = store
        .ready_memory_compactions(&["codex"], Utc::now(), 100)
        .unwrap();
    assert_eq!(second.len(), 8);
    assert!(second.iter().all(|job| job.level == 1));
}

#[test]
fn summary_insert_rejects_a_stale_child_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    for ordinal in 0..4 {
        store
            .add_memory(&scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }
    let jobs = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap();
    let mut children = Vec::new();
    for job in &jobs {
        let MemorySummaryInsert::Inserted(summary) = store
            .insert_memory_summary(job, "original", "codex", None)
            .unwrap()
        else {
            panic!("expected child summary")
        };
        children.push(summary);
    }
    let stale_parent = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);

    assert_eq!(store.invalidate_memory_summary(&children[0].id).unwrap(), 1);
    let replacement = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    store
        .insert_memory_summary(&replacement, "replacement", "codex", None)
        .unwrap();

    assert_eq!(
        store
            .insert_memory_summary(&stale_parent, "must not persist", "codex", None)
            .unwrap(),
        MemorySummaryInsert::Stale
    );
}

#[test]
fn malformed_compaction_ranges_return_an_error_instead_of_panicking() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance())
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance())
        .unwrap();
    let mut malformed = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    malformed.start_ordinal = 2;
    malformed.end_ordinal = 1;

    assert!(store
        .insert_memory_summary(&malformed, "invalid", "codex", None)
        .is_err());
}

#[test]
fn provider_cooldown_doubles_and_caps_at_one_hour() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    store
        .add_memory(&scope, "memory zero", &provenance())
        .unwrap();
    store
        .add_memory(&scope, "memory one", &provenance())
        .unwrap();
    let mut attempted_at = Utc::now();
    let job = store
        .ready_memory_compactions(&["codex"], attempted_at, 8)
        .unwrap()
        .remove(0);

    for failure_index in 0..10_u32 {
        store
            .record_memory_compaction_attempt(&job, &failed_attempt("codex", false, attempted_at))
            .unwrap();
        let delay = 15_i64
            .saturating_mul(2_i64.saturating_pow(failure_index))
            .min(3_600);
        assert!(!store
            .memory_provider_can_attempt(
                &job,
                "codex",
                attempted_at + chrono::Duration::seconds(delay - 1),
            )
            .unwrap());
        attempted_at += chrono::Duration::seconds(delay);
        assert!(store
            .memory_provider_can_attempt(&job, "codex", attempted_at)
            .unwrap());
    }
}

fn consolidate_all(store: &mut Store) {
    loop {
        let jobs = store
            .ready_memory_compactions(&["codex"], Utc::now(), 8)
            .unwrap();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            store
                .insert_memory_summary(&job, "s", "codex", None)
                .unwrap();
        }
    }
}

fn assert_frontier_invariants(
    store: &mut Store,
    project_id: &str,
    history_len: u64,
    limits: MemoryContextLimits,
) {
    let scope = MemoryReadScope::project(project_id);
    let context = store.memory_context(&scope, limits).unwrap();
    assert_eq!(context, store.memory_context(&scope, limits).unwrap());
    assert!(context.item_count <= limits.max_items);
    assert!(context.byte_count <= limits.max_bytes);
    assert!(context
        .nodes
        .windows(2)
        .all(|pair| pair[0].end_ordinal() <= pair[1].start_ordinal()));
    assert!(context
        .nodes
        .windows(2)
        .all(|pair| pair[0].level() >= pair[1].level()));
    assert_eq!(context.nodes.last().unwrap().end_ordinal(), history_len);

    let mut ranges = context
        .nodes
        .iter()
        .map(|node| (node.start_ordinal(), node.end_ordinal()))
        .chain(
            context
                .omitted
                .iter()
                .map(|omission| (omission.start_ordinal, omission.end_ordinal)),
        )
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    assert_eq!(ranges.first().unwrap().0, 0);
    assert_eq!(ranges.last().unwrap().1, history_len);
    assert!(ranges.windows(2).all(|pair| pair[0].1 == pair[1].0));
}
