// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use chrono::Utc;
use nexus::memory::{
    MemoryCompactionAttempt, MemoryCompactionOutcome, MemoryContextLimits, MemoryNode,
    MemoryProvenance, MemoryReadScope, MemoryScope, MemorySummaryInsert, MAX_MEMORY_SEARCH_RESULTS,
};
use nexus::persistence::Store;
use std::path::Path;
use std::sync::{Arc, Barrier};

#[path = "memory_persistence/frontier.rs"]
mod frontier;
#[path = "memory_persistence/health.rs"]
mod health;
#[path = "memory_persistence/snapshot_queue.rs"]
mod snapshot_queue;

fn provenance() -> MemoryProvenance {
    MemoryProvenance {
        agent: "codex".to_owned(),
        session_id: "session-a".to_owned(),
        model_id: Some("test-model".to_owned()),
        config_hash: "config-a".to_owned(),
    }
}

fn failed_attempt(
    provider: &str,
    was_fallback: bool,
    attempted_at: chrono::DateTime<Utc>,
) -> MemoryCompactionAttempt {
    MemoryCompactionAttempt {
        provider: provider.to_owned(),
        model_id: None,
        outcome: MemoryCompactionOutcome::Failed,
        diagnostic: Some("provider failed with details".to_owned()),
        was_fallback,
        attempted_at,
    }
}

fn successful_attempt(
    provider: &str,
    was_fallback: bool,
    attempted_at: chrono::DateTime<Utc>,
) -> MemoryCompactionAttempt {
    MemoryCompactionAttempt {
        provider: provider.to_owned(),
        model_id: None,
        outcome: MemoryCompactionOutcome::Succeeded,
        diagnostic: None,
        was_fallback,
        attempted_at,
    }
}

fn run_concurrent_writes(
    path: &Path,
    contents: Vec<String>,
) -> Vec<nexus::memory::MemoryWriteResult> {
    let barrier = Arc::new(Barrier::new(contents.len()));
    let stores = (0..contents.len())
        .map(|_| Store::open(path).unwrap())
        .collect::<Vec<_>>();
    contents
        .into_iter()
        .zip(stores)
        .map(|(content, mut store)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .add_memory(&MemoryScope::project("project-a"), &content, &provenance())
                    .unwrap()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect()
}

#[test]
fn adding_memory_normalizes_and_deduplicates_within_its_stream() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");

    let inserted = store
        .add_memory(&scope, "  Prefer   typed APIs.  ", &provenance())
        .unwrap();
    let duplicate = store
        .add_memory(&scope, "Prefer typed APIs.", &provenance())
        .unwrap();

    assert!(inserted.inserted);
    assert!(!duplicate.inserted);
    assert_eq!(inserted.entry.id, duplicate.entry.id);
    assert_eq!(inserted.entry.ordinal, 0);
    assert_eq!(inserted.entry.content, "Prefer typed APIs.");
}

#[test]
fn raw_memory_validation_enforces_single_line_utf8_byte_and_provenance_bounds() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::Global;

    for invalid in ["", "   ", "two\nlines", "control\tcharacter", "nul\0byte"] {
        assert!(store.add_memory(&scope, invalid, &provenance()).is_err());
    }
    let exact_limit = "é".repeat(256);
    assert_eq!(exact_limit.len(), 512);
    assert!(store
        .add_memory(&scope, &exact_limit, &provenance())
        .is_ok());
    assert!(store
        .add_memory(&scope, &format!("{exact_limit}a"), &provenance())
        .is_err());

    let mut missing_agent = provenance();
    missing_agent.agent.clear();
    assert!(store
        .add_memory(&scope, "valid content", &missing_agent)
        .is_err());
}

#[test]
fn exact_deduplication_is_local_to_each_memory_stream() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let writes = [
        MemoryScope::Global,
        MemoryScope::project("project-a"),
        MemoryScope::project("project-b"),
    ]
    .map(|scope| {
        store
            .add_memory(&scope, "  same   normalized note  ", &provenance())
            .unwrap()
    });

    assert!(writes.iter().all(|write| write.inserted));
    assert_eq!(
        writes
            .iter()
            .map(|write| write.entry.id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
}

#[test]
fn raw_search_never_exceeds_the_exported_deterministic_bound() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    for ordinal in 0..(MAX_MEMORY_SEARCH_RESULTS + 4) {
        store
            .add_memory(&scope, &format!("bounded note {ordinal}"), &provenance())
            .unwrap();
    }

    let found = store
        .search_memories(
            &MemoryReadScope::project("project-a"),
            "bounded",
            usize::MAX,
        )
        .unwrap();
    assert_eq!(found.len(), MAX_MEMORY_SEARCH_RESULTS);
    assert_eq!(found.first().unwrap().ordinal, 259);
    assert_eq!(found.last().unwrap().ordinal, 4);
}

#[test]
fn raw_search_is_case_insensitive_newest_first_and_scope_bounded() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let project_a = MemoryScope::project("project-a");
    let project_b = MemoryScope::project("project-b");
    store
        .add_memory(&project_a, "Prefer typed APIs.", &provenance())
        .unwrap();
    store
        .add_memory(
            &project_a,
            "TYPED errors are easier to inspect.",
            &provenance(),
        )
        .unwrap();
    store
        .add_memory(&project_b, "typed but isolated", &provenance())
        .unwrap();

    let found = store
        .search_memories(&MemoryReadScope::project("project-a"), "typed", 10)
        .unwrap();

    assert_eq!(found.len(), 2);
    assert_eq!(found[0].ordinal, 1);
    assert_eq!(found[1].ordinal, 0);
    assert!(store
        .search_memories(&MemoryReadScope::project("project-a"), "[", 10)
        .is_err());
}

#[test]
fn dyadic_context_omits_an_old_missing_range_before_using_newer_summaries() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    for ordinal in 0..6 {
        store
            .add_memory(&scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }

    let jobs = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap();
    assert_eq!(jobs.len(), 3);
    for job in jobs.iter().skip(1) {
        assert!(matches!(
            store
                .insert_memory_summary(job, "recent pair", "codex", Some("test-model"))
                .unwrap(),
            MemorySummaryInsert::Inserted(_)
        ));
    }

    let context = store
        .memory_context(
            &MemoryReadScope::project("project-a"),
            MemoryContextLimits {
                max_items: 3,
                max_bytes: 16 * 1024,
            },
        )
        .unwrap();

    assert_eq!(context.nodes.len(), 3);
    assert_eq!(context.omitted.len(), 1);
    assert_eq!(context.omitted[0].start_ordinal, 0);
    assert_eq!(context.omitted[0].end_ordinal, 2);
    assert!(
        matches!(&context.nodes[0], MemoryNode::Summary(summary) if summary.start_ordinal == 2 && summary.end_ordinal == 4)
    );
    assert_eq!(context.nodes[1].start_ordinal(), 4);
    assert_eq!(context.nodes[2].start_ordinal(), 5);
    assert!(context
        .nodes
        .windows(2)
        .all(|pair| pair[0].level() >= pair[1].level()));
}

#[test]
fn layered_context_donates_actual_unused_global_capacity_to_project() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    for ordinal in 0..4 {
        store
            .add_memory(
                &MemoryScope::Global,
                &format!("global{ordinal}"),
                &provenance(),
            )
            .unwrap();
    }
    let project = MemoryScope::project("project-a");
    for ordinal in 0..6 {
        store
            .add_memory(&project, &format!("proj-{ordinal:02}"), &provenance())
            .unwrap();
    }
    let jobs = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap();
    for job in jobs.iter().filter(|job| job.scope == MemoryScope::Global) {
        store
            .insert_memory_summary(job, "g", "codex", None)
            .unwrap();
    }

    let context = store
        .memory_context(
            &MemoryReadScope::layered("project-a"),
            MemoryContextLimits {
                max_items: 64,
                max_bytes: 60,
            },
        )
        .unwrap();

    assert_eq!(
        context
            .nodes
            .iter()
            .filter(|node| node.scope() == &project)
            .count(),
        6
    );
    assert!(context
        .omitted
        .iter()
        .all(|omission| omission.scope != project));
    assert!(context.byte_count <= 60);
}

#[test]
fn layered_context_donates_actual_unused_project_capacity_to_global() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    for ordinal in 0..3 {
        store
            .add_memory(
                &MemoryScope::Global,
                &format!("global{ordinal}"),
                &provenance(),
            )
            .unwrap();
    }
    let project = MemoryScope::project("project-a");
    store
        .add_memory(&project, "project", &provenance())
        .unwrap();

    let context = store
        .memory_context(
            &MemoryReadScope::layered("project-a"),
            MemoryContextLimits {
                max_items: 64,
                max_bytes: 60,
            },
        )
        .unwrap();

    assert_eq!(
        context
            .nodes
            .iter()
            .filter(|node| node.scope() == &MemoryScope::Global)
            .count(),
        3
    );
    assert!(context
        .omitted
        .iter()
        .all(|omission| omission.scope != MemoryScope::Global));
    assert!(context.byte_count <= 60);
}

#[test]
fn layered_context_donates_spare_capacity_when_both_scopes_are_pressured() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    for ordinal in 0..22 {
        store
            .add_memory(
                &MemoryScope::Global,
                &format!("global-{ordinal:02}"),
                &provenance(),
            )
            .unwrap();
    }
    let project = MemoryScope::project("project-a");
    for ordinal in 0..44 {
        store
            .add_memory(&project, &format!("project-{ordinal:02}"), &provenance())
            .unwrap();
    }

    let context = store
        .memory_context(
            &MemoryReadScope::layered("project-a"),
            MemoryContextLimits {
                max_items: 64,
                max_bytes: 16 * 1024,
            },
        )
        .unwrap();

    assert_eq!(context.item_count, 64);
    assert_eq!(
        context
            .nodes
            .iter()
            .filter(|node| node.scope() == &project)
            .count(),
        44
    );
    assert_eq!(
        context
            .omitted
            .iter()
            .map(|omission| omission.end_ordinal - omission.start_ordinal)
            .sum::<u64>(),
        2
    );
}

#[test]
fn activation_is_once_per_agent_session_and_invalidation_preserves_raw_memory() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    let (summaries, root) = build_four_note_summary_tree(&mut store, &scope);

    let children = store.expand_memory_summary(&root.id).unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|child| child.level() == 1));

    let read_scope = MemoryReadScope::project("project-a");
    assert!(store
        .activate_memory_context(
            &read_scope,
            "codex",
            "session-a",
            MemoryContextLimits::default(),
        )
        .unwrap()
        .is_some());
    assert!(store
        .activate_memory_context(
            &read_scope,
            "codex",
            "session-a",
            MemoryContextLimits::default(),
        )
        .unwrap()
        .is_none());

    assert_eq!(
        store.invalidate_memory_summary(&summaries[0].id).unwrap(),
        2
    );
    assert_eq!(
        store
            .search_memories(&read_scope, "memory", 10)
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        store.expand_memory_summary(&summaries[1].id).unwrap().len(),
        2
    );
    assert!(store.expand_memory_summary(&root.id).is_err());
}

#[test]
fn project_health_ignores_newer_activations_from_other_projects() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let limits = MemoryContextLimits::default();
    store
        .activate_memory_context(
            &MemoryReadScope::project("project-a"),
            "codex",
            "project-a-session",
            limits,
        )
        .unwrap();
    let project_a_activation = store
        .memory_health(Some("project-a"), Utc::now())
        .unwrap()
        .last_activation_at
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(2));
    store
        .activate_memory_context(
            &MemoryReadScope::project("project-b"),
            "codex",
            "project-b-session",
            limits,
        )
        .unwrap();

    assert_eq!(
        store
            .memory_health(Some("project-a"), Utc::now())
            .unwrap()
            .last_activation_at,
        Some(project_a_activation)
    );
    assert!(
        store
            .memory_health(None, Utc::now())
            .unwrap()
            .last_activation_at
            .unwrap()
            > project_a_activation
    );
}

fn build_four_note_summary_tree(
    store: &mut Store,
    scope: &MemoryScope,
) -> (
    Vec<nexus::memory::MemorySummary>,
    nexus::memory::MemorySummary,
) {
    for ordinal in 0..4 {
        store
            .add_memory(scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }
    let first_level = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap();
    let mut summaries = Vec::new();
    for job in &first_level {
        let MemorySummaryInsert::Inserted(summary) = store
            .insert_memory_summary(job, "pair summary", "codex", None)
            .unwrap()
        else {
            panic!("expected a new summary")
        };
        summaries.push(summary);
    }
    let root_job = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    let MemorySummaryInsert::Inserted(root) = store
        .insert_memory_summary(&root_job, "root summary", "codex", None)
        .unwrap()
    else {
        panic!("expected a root summary")
    };
    assert!(store.repair_memory_frontier(scope).unwrap());
    (summaries, root)
}

#[test]
fn concurrent_writers_allocate_unique_ordinals() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("concurrent.db");
    drop(Store::open(&path).unwrap());
    let mut ordinals = run_concurrent_writes(
        &path,
        (0..8)
            .map(|index| format!("concurrent memory {index}"))
            .collect(),
    )
    .into_iter()
    .map(|write| write.entry.ordinal)
    .collect::<Vec<_>>();
    ordinals.sort_unstable();
    assert_eq!(ordinals, (0..8).collect::<Vec<_>>());
}

#[test]
fn concurrent_exact_duplicates_collapse_to_one_entry() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("concurrent.db");
    drop(Store::open(&path).unwrap());
    let duplicates = run_concurrent_writes(&path, vec!["one exact duplicate".to_owned(); 4]);
    assert_eq!(duplicates.iter().filter(|write| write.inserted).count(), 1);
    assert!(duplicates
        .iter()
        .all(|write| write.entry.id == duplicates[0].entry.id));
    assert!(duplicates.iter().all(|write| write.entry.ordinal == 0));
}
