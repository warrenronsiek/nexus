// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use nexus::memory::{
    MemoryContextStatus, MemoryScopeHealth, DEFAULT_MEMORY_MAX_BYTES, DEFAULT_MEMORY_MAX_ITEMS,
    MAX_MEMORY_FRONTIER_OMISSIONS,
};

mod snapshot_schema {
    diesel::table! {
        memory_spaces (id) {
            id -> Text,
            scope -> Text,
            project_id -> Nullable<Text>,
            next_ordinal -> BigInt,
            revision -> BigInt,
            created_at -> Text,
            updated_at -> Text,
        }
    }

    diesel::table! {
        memory_frontier_snapshots (space_id) {
            space_id -> Text,
            revision -> BigInt,
            node_refs_json -> Text,
            omissions_json -> Text,
            item_count -> Integer,
            omission_count -> Integer,
            byte_count -> Integer,
            built_at -> Text,
        }
    }
}

#[test]
fn bounded_cached_frontier_matches_a_full_rebuild_for_every_tighter_budget() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("nexus.db");
    let mut store = Store::open(&path).unwrap();
    let concrete_scope = MemoryScope::project("project-a");
    let read_scope = MemoryReadScope::project("project-a");
    for ordinal in 0..96 {
        store
            .add_memory(
                &concrete_scope,
                &format!("bounded memory {ordinal:02}"),
                &provenance(),
            )
            .unwrap();
    }
    let first_pair = consolidate_all(&mut store)
        .into_iter()
        .find(|summary| summary.level == 1 && summary.start_ordinal == 0)
        .unwrap();
    assert!(store.invalidate_memory_summary(&first_pair.id).unwrap() > 1);
    for ordinal in 96..112 {
        store
            .add_memory(
                &concrete_scope,
                &format!("bounded memory {ordinal:02}"),
                &provenance(),
            )
            .unwrap();
    }

    let limits = [
        MemoryContextLimits::default(),
        MemoryContextLimits {
            max_items: 17,
            max_bytes: DEFAULT_MEMORY_MAX_BYTES,
        },
        MemoryContextLimits {
            max_items: 3,
            max_bytes: DEFAULT_MEMORY_MAX_BYTES,
        },
        MemoryContextLimits {
            max_items: DEFAULT_MEMORY_MAX_ITEMS,
            max_bytes: 96,
        },
    ];
    let cached = contexts(&mut store, &read_scope, &limits);
    assert!(cached[0]
        .nodes
        .iter()
        .any(|node| matches!(node, MemoryNode::Summary(_))));
    assert!(!cached[0].omitted.is_empty());
    assert_snapshot_is_hard_bounded(&path, "project-a");

    bump_project_revision(&path, "project-a");
    assert!(store.repair_memory_frontier(&concrete_scope).unwrap());
    let rebuilt = contexts(&mut store, &read_scope, &limits);

    assert_eq!(cached, rebuilt);
    assert_snapshot_is_hard_bounded(&path, "project-a");
}

#[test]
fn stale_frontier_activation_is_retried_after_explicit_context_repairs_it() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let concrete_scope = MemoryScope::project("project-a");
    let read_scope = MemoryReadScope::project("project-a");
    for ordinal in 0..2 {
        store
            .add_memory(&concrete_scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }
    let job = store
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    store
        .insert_memory_summary(&job, "pair", "codex", None)
        .unwrap();

    assert!(store
        .activate_memory_context(
            &read_scope,
            "codex",
            "same-session",
            MemoryContextLimits::default(),
        )
        .unwrap()
        .is_none());

    let repaired = store
        .memory_context(&read_scope, MemoryContextLimits::default())
        .unwrap();
    assert_eq!(repaired.nodes.len(), 2);
    assert!(store
        .activate_memory_context(
            &read_scope,
            "codex",
            "same-session",
            MemoryContextLimits::default(),
        )
        .unwrap()
        .is_some());
}

#[test]
fn compaction_queue_survives_reopen_and_promotes_completed_siblings() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("nexus.db");
    let scope = MemoryScope::project("project-a");
    let first_level = {
        let mut store = Store::open(&path).unwrap();
        for ordinal in 0..4 {
            store
                .add_memory(&scope, &format!("memory {ordinal}"), &provenance())
                .unwrap();
        }
        store
            .ready_memory_compactions(&["codex"], Utc::now(), 8)
            .unwrap()
    };
    assert_eq!(first_level.len(), 2);

    let mut reopened = Store::open(&path).unwrap();
    let after_reopen = reopened
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap();
    assert_eq!(job_keys(&after_reopen), job_keys(&first_level));
    for job in after_reopen {
        reopened
            .insert_memory_summary(&job, "pair", "codex", None)
            .unwrap();
    }
    drop(reopened);

    let mut reopened = Store::open(&path).unwrap();
    let promoted = reopened
        .ready_memory_compactions(&["codex"], Utc::now(), 8)
        .unwrap();
    assert_eq!(promoted.len(), 1);
    assert_eq!(promoted[0].level, 2);
    assert_eq!((promoted[0].start_ordinal, promoted[0].end_ordinal), (0, 4));
}

#[test]
fn cooled_early_jobs_do_not_starve_later_ready_jobs_before_the_batch_limit() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let scope = MemoryScope::project("project-a");
    for ordinal in 0..40 {
        store
            .add_memory(&scope, &format!("memory {ordinal}"), &provenance())
            .unwrap();
    }
    let now = Utc::now();
    let cooled = store.ready_memory_compactions(&["codex"], now, 8).unwrap();
    assert_eq!(cooled.len(), 8);
    for job in &cooled {
        store
            .record_memory_compaction_attempt(job, &failed_attempt("codex", false, now))
            .unwrap();
    }

    let ready = store.ready_memory_compactions(&["codex"], now, 8).unwrap();
    assert_eq!(ready.len(), 8);
    assert!(ready.iter().all(|job| job.level == 1));
    assert!(ready.iter().all(|job| job.start_ordinal >= 16));
    assert!(ready.iter().all(|job| cooled
        .iter()
        .all(|cooled_job| cooled_job.job_id != job.job_id)));
}

#[test]
fn health_always_includes_explicit_global_and_selected_project_rows() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();

    let fresh = store.memory_health(Some("project-a"), Utc::now()).unwrap();
    assert_scope_health(&fresh.scopes, &MemoryScope::Global, 0, 0);
    assert_scope_health(&fresh.scopes, &MemoryScope::project("project-a"), 0, 0);
    assert_eq!(fresh.scopes.len(), 2);

    store
        .add_memory(
            &MemoryScope::project("project-a"),
            "project-only memory",
            &provenance(),
        )
        .unwrap();
    let one_scope = store.memory_health(Some("project-a"), Utc::now()).unwrap();
    assert_scope_health(&one_scope.scopes, &MemoryScope::Global, 0, 0);
    assert_scope_health(&one_scope.scopes, &MemoryScope::project("project-a"), 1, 0);
    assert_eq!(one_scope.scopes.len(), 2);
}

#[test]
fn absent_streams_return_empty_context_and_activate_once_without_creating_spaces() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("nexus.db");
    let mut store = Store::open(&path).unwrap();
    let scopes = [
        MemoryReadScope::Global,
        MemoryReadScope::project("project-a"),
        MemoryReadScope::layered("project-a"),
    ];

    for (index, scope) in scopes.into_iter().enumerate() {
        let context = store
            .memory_context(&scope, MemoryContextLimits::default())
            .unwrap();
        assert_eq!(context.scope, scope);
        assert!(context.nodes.is_empty());
        assert!(context.omitted.is_empty());
        assert_eq!(context.item_count, 0);
        assert_eq!(context.byte_count, 0);

        let session_id = format!("empty-session-{index}");
        let activated = store
            .activate_memory_context(&scope, "codex", &session_id, MemoryContextLimits::default())
            .unwrap()
            .expect("an absent stream still has an empty context to deliver");
        assert!(activated.nodes.is_empty());
        assert!(store
            .activate_memory_context(&scope, "codex", &session_id, MemoryContextLimits::default(),)
            .unwrap()
            .is_none());
    }

    assert_eq!(memory_space_count(&path), 0);
}

#[test]
fn direct_layered_context_clamps_unbounded_caller_limits() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let project_scope = MemoryScope::project("project-a");
    for ordinal in 0..80 {
        store
            .add_memory(
                &MemoryScope::Global,
                &format!("global-{ordinal:03}-{}", "g".repeat(380)),
                &provenance(),
            )
            .unwrap();
        store
            .add_memory(
                &project_scope,
                &format!("project-{ordinal:03}-{}", "p".repeat(380)),
                &provenance(),
            )
            .unwrap();
    }

    let context = store
        .memory_context(
            &MemoryReadScope::layered("project-a"),
            MemoryContextLimits {
                max_items: usize::MAX,
                max_bytes: usize::MAX,
            },
        )
        .unwrap();

    assert!(context.item_count <= DEFAULT_MEMORY_MAX_ITEMS);
    assert!(context.byte_count <= DEFAULT_MEMORY_MAX_BYTES);
    assert_eq!(context.item_count, context.nodes.len());
    assert_eq!(
        context.byte_count,
        context
            .nodes
            .iter()
            .map(|node| node.content().len())
            .sum::<usize>()
    );
    assert!(context
        .nodes
        .iter()
        .any(|node| node.scope() == &MemoryScope::Global));
    assert!(context
        .nodes
        .iter()
        .any(|node| node.scope() == &project_scope));
}

fn contexts(
    store: &mut Store,
    scope: &MemoryReadScope,
    limits: &[MemoryContextLimits],
) -> Vec<MemoryContextStatus> {
    limits
        .iter()
        .map(|limits| store.memory_context(scope, *limits).unwrap())
        .collect()
}

fn consolidate_all(store: &mut Store) -> Vec<nexus::memory::MemorySummary> {
    let mut summaries = Vec::new();
    loop {
        let jobs = store
            .ready_memory_compactions(&["codex"], Utc::now(), 8)
            .unwrap();
        if jobs.is_empty() {
            return summaries;
        }
        for job in jobs {
            if let MemorySummaryInsert::Inserted(summary) = store
                .insert_memory_summary(&job, "summary", "codex", None)
                .unwrap()
            {
                summaries.push(summary);
            }
        }
    }
}

fn bump_project_revision(path: &Path, project_id: &str) {
    use snapshot_schema::memory_spaces;
    let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    diesel::update(
        memory_spaces::table
            .filter(memory_spaces::scope.eq("project"))
            .filter(memory_spaces::project_id.eq(Some(project_id))),
    )
    .set(memory_spaces::revision.eq(memory_spaces::revision + 1_i64))
    .execute(&mut connection)
    .unwrap();
}

fn assert_snapshot_is_hard_bounded(path: &Path, project_id: &str) {
    use snapshot_schema::{memory_frontier_snapshots, memory_spaces};
    let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    let space_id = memory_spaces::table
        .filter(memory_spaces::scope.eq("project"))
        .filter(memory_spaces::project_id.eq(Some(project_id)))
        .select(memory_spaces::id)
        .first::<String>(&mut connection)
        .unwrap();
    let (items, omissions, bytes) = memory_frontier_snapshots::table
        .find(space_id)
        .select((
            memory_frontier_snapshots::item_count,
            memory_frontier_snapshots::omission_count,
            memory_frontier_snapshots::byte_count,
        ))
        .first::<(i32, i32, i32)>(&mut connection)
        .unwrap();
    assert!(usize::try_from(items).unwrap() <= DEFAULT_MEMORY_MAX_ITEMS);
    assert!(usize::try_from(omissions).unwrap() <= MAX_MEMORY_FRONTIER_OMISSIONS);
    assert!(usize::try_from(bytes).unwrap() <= DEFAULT_MEMORY_MAX_BYTES);
}

fn memory_space_count(path: &Path) -> i64 {
    use snapshot_schema::memory_spaces;
    let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    memory_spaces::table
        .count()
        .get_result(&mut connection)
        .unwrap()
}

fn job_keys(jobs: &[nexus::memory::MemoryCompactionJob]) -> Vec<(u32, u64, u64, String)> {
    jobs.iter()
        .map(|job| {
            (
                job.level,
                job.start_ordinal,
                job.end_ordinal,
                job.source_hash.clone(),
            )
        })
        .collect()
}

fn assert_scope_health(
    scopes: &[MemoryScopeHealth],
    expected_scope: &MemoryScope,
    raw_entries: i64,
    summaries: i64,
) {
    let health = scopes
        .iter()
        .find(|health| &health.scope == expected_scope)
        .unwrap_or_else(|| panic!("missing health row for {expected_scope:?}"));
    assert_eq!(health.raw_entries, raw_entries);
    assert_eq!(health.summaries, summaries);
}
