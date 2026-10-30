// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary capture-reliability-fixtures: decode closed callback envelopes and assertion-only metadata JSON; no workload payloads are retained.
use super::*;
use crate::work_history::{WorkEvent, WorkSubject};

fn fixture(temp: &Path) -> LoadedConfig {
    let mut config = crate::Config::default();
    config.storage.database_path = temp.join("nexus.db");
    config.runtime.socket_path = temp.join("nexus.sock");
    LoadedConfig {
        config,
        hash: "test".into(),
        sources: Vec::new(),
    }
}

fn callback(id: &str) -> WorkCommand {
    WorkCommand {
        context: HookContext {
            session_id: "source-session".into(),
            agent: "fixture".into(),
            project_root: None,
            turn_id: None,
            model: None,
        },
        command: WorkAction::Record {
            observations: vec![lifecycle::event(
                WorkSubject::Activity { id: id.into() },
                "started",
                WorkEvent::Started,
            )],
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_capture_admits_siblings_and_reports_bounded_overflow() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = Arc::new(fixture(temp.path()));
    let path = directory(&loaded).unwrap();
    let holder = Outbox::writer(&path).unwrap();
    let handoff = CaptureHandoff::new(2);
    for id in ["first", "second"] {
        let receipt = handoff
            .capture(loaded.clone(), callback(id), Duration::from_millis(10))
            .await;
        assert_eq!(receipt.status, CaptureStatus::Pending);
        assert_eq!(receipt.lost, 0);
    }
    let receipt = handoff
        .capture(
            loaded.clone(),
            callback("overflow"),
            Duration::from_millis(10),
        )
        .await;
    assert_eq!(receipt.status, CaptureStatus::Lost);
    assert_eq!(receipt.reason, Some("admission_capacity"));
    assert_eq!(receipt.lost, 1);
    drop(holder);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if health(&loaded).is_ok_and(|health| {
                health.queued == 2 && health.capture.is_some_and(|capture| capture.offered == 3)
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let queue = Outbox::writer(&path).unwrap();
    assert_eq!(queue.pending(16).unwrap().len(), 2);
    let counters = queue.health().unwrap().capture.unwrap();
    assert_eq!(counters.offered, 3);
    assert_eq!(counters.durably_queued, 2);
    assert_eq!(counters.loss_reasons.get("admission_capacity"), Some(&1));
}

#[test]
fn repeated_quarantined_callbacks_report_rejection_instead_of_durable_capture() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    let first = persist_lifecycle(&loaded, callback("rejected-tool"));
    assert_eq!(first.status, CaptureStatus::Durable);
    let queue = Outbox::writer(&directory(&loaded).unwrap()).unwrap();
    queue
        .reject(&queue.pending(16).unwrap().pop().unwrap(), "invalid")
        .unwrap();
    drop(queue);
    let repeat = persist_lifecycle(&loaded, callback("rejected-tool"));
    assert_eq!(repeat.status, CaptureStatus::Rejected);
    assert_eq!(repeat.rejected, 1);
    assert_eq!(repeat.duplicates, 0);
}

#[test]
fn synchronous_capture_does_not_wait_for_a_slow_writer() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = Arc::new(fixture(temp.path()));
    let path = directory(&loaded).unwrap();
    let holder = Outbox::writer(&path).unwrap();
    let handoff = CaptureHandoff::new(2);
    let started = Instant::now();
    let receipt =
        handoff.capture_blocking(loaded.clone(), callback("sync"), Duration::from_millis(10));
    assert_eq!(receipt.status, CaptureStatus::Pending);
    assert!(started.elapsed() < Duration::from_millis(200));
    drop(holder);
    for _ in 0..500 {
        if health(&loaded).is_ok_and(|value| value.queued == 1) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the deferred observation was lost");
}

#[test]
fn a_producer_exit_fence_waits_for_admitted_capture_with_a_bounded_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = Arc::new(fixture(temp.path()));
    let path = directory(&loaded).unwrap();
    let holder = Outbox::writer(&path).unwrap();
    let handoff = CaptureHandoff::new(2);
    let receipt = handoff.capture_blocking(
        loaded,
        callback("short-lived-producer"),
        Duration::from_millis(1),
    );
    assert_eq!(receipt.status, CaptureStatus::Pending);
    let started = Instant::now();
    assert!(!handoff.drain_pending(Duration::from_millis(5)));
    assert!(started.elapsed() < Duration::from_millis(200));
    drop(holder);
    assert!(handoff.drain_pending(Duration::from_secs(1)));
    assert_eq!(Outbox::open(&path).unwrap().pending(16).unwrap().len(), 1);
}

#[test]
fn callback_replay_matches_only_regenerated_transport_metadata() {
    let WorkAction::Record { observations } = callback("immutable").command else {
        unreachable!()
    };
    let accepted = &observations[0];
    let mut replay = accepted.clone();
    replay.occurred_at += chrono::Duration::seconds(60);
    replay.producer.instance_id = "restarted-companion".into();
    assert!(callback_replay_matches(accepted, &replay));
    replay.body = WorkEvent::Queued;
    assert!(!callback_replay_matches(accepted, &replay));
    replay = accepted.clone();
    replay.producer.name = "caller-owned".into();
    assert!(!callback_replay_matches(accepted, &replay));
}

#[test]
fn a_full_outbox_accounts_for_every_offered_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    let path = directory(&loaded).unwrap();
    let queue = Outbox::open(&path).unwrap();
    for index in 0..256 {
        std::fs::write(path.join(format!("event-{index}.json")), b"metadata").unwrap();
    }
    drop(queue);
    let mut command = callback("overflow-one");
    let WorkAction::Record { observations } = &mut command.command else {
        unreachable!()
    };
    observations.push(lifecycle::event(
        WorkSubject::Activity {
            id: "overflow-two".into(),
        },
        "started",
        WorkEvent::Started,
    ));
    let receipt = persist_lifecycle(&loaded, command);
    assert_eq!(receipt.offered, 2);
    assert_eq!(receipt.lost, 2);
    assert_eq!(receipt.reason, Some("capacity"));
    let health = health(&loaded).unwrap();
    assert_eq!(health.queued, 256);
    let capture = health.capture.unwrap();
    assert_eq!(capture.offered, 2);
    assert_eq!(capture.loss_reasons.get("capacity"), Some(&2));
}

#[test]
fn exact_host_bridge_keeps_an_explicit_work_root_across_later_callbacks() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    let command = host_event("claude",br#"{"session_id":"shared-host","hook_event_name":"SessionStart","work_session_id":"shared-work","parent_activity_id":"caller"}"#).unwrap();
    persist_lifecycle(&loaded, command);
    let request = crate::coordination::api::ServiceRequest::decode("pre_tool_use",serde_json::json!({"session_id":"shared-host","agent":"claude","tool_use_id":"one-tool","tool_name":"Bash"})).unwrap();
    persist_lifecycle(&loaded, prepare_lifecycle(&request).unwrap());
    let active = context_bridge::active_host_context(&loaded, "claude", "shared-host").unwrap();
    assert_eq!(active.work_session_id, "shared-work");
    let queue = Outbox::writer(&directory(&loaded).unwrap()).unwrap();
    let commands: Vec<WorkCommand> = queue
        .pending(16)
        .unwrap()
        .into_iter()
        .map(|entry| serde_json::from_slice(&entry.bytes).unwrap())
        .collect();
    let declarations = commands
        .iter()
        .flat_map(|command| match &command.command {
            WorkAction::Record { observations } => observations.as_slice(),
            _ => &[],
        })
        .filter_map(|observation| match &observation.body {
            WorkEvent::ActivityDeclared {
                kind,
                work_session_id,
                ..
            } if kind == "tool" => Some(work_session_id.as_deref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(declarations, vec![Some("shared-work")]);
}

#[test]
fn parallel_child_agents_have_distinct_executors_even_when_their_types_match() {
    let mut executors = Vec::new();
    for agent in ["child-a", "child-b"] {
        let bytes = serde_json::to_vec(&serde_json::json!({"session_id":"parent","hook_event_name":"SubagentStart","agent_id":agent,"agent_type":"general-purpose"})).unwrap();
        let WorkAction::Record { observations } = host_event("claude", &bytes).unwrap().command
        else {
            unreachable!()
        };
        let executor = observations
            .iter()
            .find_map(|observation| match &observation.body {
                WorkEvent::ActivityDeclared { kind, executor, .. } if kind == "agent.task" => {
                    executor.clone()
                }
                _ => None,
            })
            .unwrap();
        executors.push(executor);
    }
    assert_ne!(executors[0], executors[1]);
}

#[test]
fn mixed_enqueue_failures_keep_each_reason_count() {
    let mut receipt = CaptureReceipt::lost(0, "unused");
    account_enqueue(&mut receipt, Err(outbox::CapacityExceeded.into()));
    account_enqueue(
        &mut receipt,
        Err(std::io::Error::other("fixture write failure").into()),
    );
    let receipt = serde_json::to_value(receipt).unwrap();
    assert_eq!(receipt["loss_reasons"]["capacity"], 1);
    assert_eq!(receipt["loss_reasons"]["enqueue_io"], 1);
}

#[test]
fn a_continuation_stops_the_original_request_instead_of_an_orphan_request() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    for value in [
        serde_json::json!({"session_id":"continuation-host","hook_event_name":"RequestStart","turn_id":"first","request_id":"human-request"}),
        serde_json::json!({"session_id":"continuation-host","hook_event_name":"TurnFinish","turn_id":"first"}),
        serde_json::json!({"session_id":"continuation-host","hook_event_name":"TurnStart","turn_id":"continuation"}),
        serde_json::json!({"session_id":"continuation-host","hook_event_name":"Stop","turn_id":"continuation"}),
    ] {
        persist_lifecycle(
            &loaded,
            host_event("claude", &serde_json::to_vec(&value).unwrap()).unwrap(),
        );
    }
    let request = stable_id(&["request", "claude", "continuation-host", "human-request"]);
    let queue = Outbox::writer(&directory(&loaded).unwrap()).unwrap();
    let finished = queue.pending(16).unwrap().into_iter().any(|entry| {
        let command: WorkCommand = serde_json::from_slice(&entry.bytes).unwrap();
        let WorkAction::Record { observations } = command.command else {
            return false;
        };
        observations.into_iter().any(|event| {
            event.subject.id() == request && matches!(event.body, WorkEvent::Finished { .. })
        })
    });
    assert!(
        finished,
        "the active request must have a terminal boundary after its continuation"
    );
}

#[test]
fn an_evicted_old_turn_never_finishes_an_unrelated_active_request() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    let offered = |name: &str, turn: &str, request: Option<&str>| {
        let bytes=serde_json::to_vec(&serde_json::json!({"session_id":"bounded-host","hook_event_name":name,"turn_id":turn,"request_id":request})).unwrap();
        let mut command = host_event("claude", &bytes).unwrap();
        context_bridge::enrich(&loaded, &mut command);
    };
    offered("RequestStart", "old-turn", Some("old-request"));
    for index in 0..17 {
        let id = format!("intervening-{index}");
        offered("RequestStart", &id, Some(&id));
        offered("Stop", &id, Some(&id));
    }
    offered("RequestStart", "current-turn", Some("current-request"));
    offered("Stop", "old-turn", None);
    let active = context_bridge::active_host_context(&loaded, "claude", "bounded-host").unwrap();
    assert_eq!(
        active.request_activity_id,
        Some(stable_id(&[
            "request",
            "claude",
            "bounded-host",
            "current-request"
        ]))
    );
}

#[test]
fn invalid_adapter_callbacks_have_durable_metadata_only_rejection_health() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    let receipt = capture_adapter_rejection(&loaded, "claude", Duration::from_secs(1));
    assert_eq!(receipt.status, CaptureStatus::Rejected);
    let health = health(&loaded).unwrap();
    assert_eq!(health.queued, 0);
    let counters = serde_json::to_value(health.capture.unwrap()).unwrap();
    assert_eq!(counters["transport_rejected"], 1);
    assert_eq!(
        counters["transport_rejection_reasons"]["invalid_host_event"],
        1
    );
    assert_eq!(counters["last_adapter_host"], "claude");
}

fn finished_callback(outcome: crate::work_history::WorkOutcome) -> WorkCommand {
    let mut command = callback("cached-terminal");
    let WorkAction::Record { observations } = &mut command.command else {
        unreachable!()
    };
    observations[0].body = WorkEvent::Finished {
        outcome,
        elapsed: None,
        exit_code: None,
        signal: None,
        assessment: None,
    };
    command
}

#[test]
fn changed_cached_outcomes_are_rejected_without_replacing_the_original_bytes() {
    for phase in ["json", "seen", "callback"] {
        let temp = tempfile::tempdir().unwrap();
        let loaded = fixture(temp.path());
        persist_lifecycle(
            &loaded,
            finished_callback(crate::work_history::WorkOutcome::Succeeded),
        );
        let queue = Outbox::writer(&directory(&loaded).unwrap()).unwrap();
        let original = queue.pending(16).unwrap().pop().unwrap();
        match phase {
            "seen" => queue.acknowledge(&original).unwrap(),
            "callback" => queue.defer_callback(&original).unwrap(),
            _ => {}
        }
        let path = original.path.with_extension(phase);
        drop(queue);
        let receipt = persist_lifecycle(
            &loaded,
            finished_callback(crate::work_history::WorkOutcome::Failed),
        );
        assert_eq!(receipt.status, CaptureStatus::Rejected);
        assert_eq!(receipt.reason, Some("conflict"));
        assert_eq!(receipt.duplicates, 0);
        assert_eq!(std::fs::read(path).unwrap(), original.bytes);
        assert_eq!(
            health(&loaded)
                .unwrap()
                .capture
                .unwrap()
                .rejection_reasons
                .get("conflict"),
            Some(&1)
        );
    }
}

#[test]
fn repeated_native_containers_preserve_the_first_boundary_without_false_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    for (name, at) in [
        ("SessionStart", "2026-10-27T12:00:00Z"),
        ("SessionEnd", "2026-10-27T12:01:00Z"),
    ] {
        let bytes=serde_json::to_vec(&serde_json::json!({"session_id":"canonical-container","hook_event_name":name,"occurred_at":at})).unwrap();
        let receipt = persist_lifecycle(&loaded, host_event("claude", &bytes).unwrap());
        assert_eq!(receipt.status, CaptureStatus::Durable);
        assert_eq!(receipt.rejected, 0);
    }
}

#[test]
fn shared_sqlite_outboxes_keep_identical_observation_ids_separate_by_git_project() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    for name in ["first", "second"] {
        let root = temp.path().join(name);
        std::fs::create_dir(&root).unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success());
        let mut command = callback("same-tool-id");
        command.context.project_root = Some(root.to_string_lossy().into_owned());
        let receipt = persist_lifecycle(&loaded, command);
        assert_eq!(receipt.status, CaptureStatus::Durable);
        assert_eq!(receipt.queued, 1, "{name} was incorrectly deduplicated");
        assert_eq!(receipt.duplicates, 0);
    }
    assert_eq!(health(&loaded).unwrap().queued, 2);
}

#[tokio::test]
async fn explicit_recovery_reclaims_settled_legacy_backlog_before_new_admission() {
    let temp = tempfile::tempdir().unwrap();
    let loaded = fixture(temp.path());
    let path = directory(&loaded).unwrap();
    {
        let _queue = Outbox::open(&path).unwrap();
        for index in 0..340 {
            crate::private_state::write(
                &path.join(format!("delivered-{index}.seen")),
                b"delivered",
            )
            .unwrap();
        }
        crate::private_state::write(&path.join("rejected.rejected-invalid"), b"retained").unwrap();
    }
    let health = flush_pending(&loaded, None, 256).await.unwrap();
    assert_eq!(health.queued, 0);
    assert_eq!(health.rejected, 1);
    assert!(health.recent_delivered + health.rejected < 256);
    let queue = Outbox::open(&path).unwrap();
    queue.enqueue("fresh", b"source").unwrap();
    assert_eq!(queue.health().unwrap().queued, 1);
}
