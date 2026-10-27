// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
#![cfg(feature = "sqlite")]
use nexus::persistence::Store;
use serde_json::json;
#[path = "support/work_accounting.rs"]
mod fixture;
use fixture::{at, command, interval};

#[test]
fn source_monotonic_boundaries_can_cross_producer_processes_but_not_clock_domains() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("source-clock.db")).unwrap();
    let mut observations = interval("measured", None, None, 2, 10);
    observations[1]["source_clock"] =
        json!({"clock_id":"os-monotonic:boot-fixture","nanoseconds":"100"});
    observations[2]["source_clock"] =
        json!({"clock_id":"os-monotonic:boot-fixture","nanoseconds":"800"});
    observations[2]["producer"]["instance_id"] = json!("next-process");
    observations[2]["body"]
        .as_object_mut()
        .unwrap()
        .remove("elapsed");
    command(
        &mut store,
        json!({"action":"record","observations":observations}),
    );
    let result = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"measured"}}),
    );
    assert_eq!(result["record"]["summary"]["elapsed"]["nanoseconds"], "700");
    assert_eq!(
        result["record"]["summary"]["elapsed"]["method"],
        "os_monotonic_callback_interval"
    );
    assert!(!result["record"]["summary"]["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("clock_domain_mismatch")));
}

#[test]
fn conflicting_source_clocks_never_fall_back_to_a_plausible_wall_interval() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("conflicting-clock.db")).unwrap();
    for (id, finish_clock, finish_ns, gap) in [
        ("cross-domain", "other-boot", "800", "clock_domain_mismatch"),
        ("reversed", "boot", "99", "source_clock_anomaly"),
    ] {
        let mut observations = interval(id, None, None, 2, 10);
        observations[1]["source_clock"] = json!({"clock_id":"boot","nanoseconds":"100"});
        observations[2]["source_clock"] = json!({"clock_id":finish_clock,"nanoseconds":finish_ns});
        observations[2]["body"]
            .as_object_mut()
            .unwrap()
            .remove("elapsed");
        command(
            &mut store,
            json!({"action":"record","observations":observations}),
        );
        let result = command(
            &mut store,
            json!({"action":"get","subject":{"scope":"activity","id":id}}),
        );
        assert!(result["record"]["summary"]["elapsed"].is_null(), "{id}");
        assert!(
            result["record"]["summary"]["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!(gap)),
            "{id}"
        );
    }
}

#[test]
fn same_utc_boundaries_with_competing_monotonic_evidence_stay_conflicted() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("competing-clock.db")).unwrap();
    for terminal in [false, true] {
        let id = if terminal { "finish" } else { "start" };
        let mut observations = interval(id, None, None, 2, 10);
        observations[1]["source_clock"] = json!({"clock_id":"boot","nanoseconds":"100"});
        observations[2]["source_clock"] = json!({"clock_id":"boot","nanoseconds":"800"});
        observations[2]["body"]
            .as_object_mut()
            .unwrap()
            .remove("elapsed");
        let mut competing = observations[if terminal { 2 } else { 1 }].clone();
        competing["id"] = json!(format!("competing-{id}"));
        competing["source_clock"]["nanoseconds"] = json!(if terminal { "801" } else { "101" });
        observations.push(competing);
        command(
            &mut store,
            json!({"action":"record","observations":observations}),
        );
        let result = command(
            &mut store,
            json!({"action":"get","subject":{"scope":"activity","id":id}}),
        );
        let conflict = if terminal {
            "conflicting_finishes"
        } else {
            "conflicting_starts"
        };
        assert!(result["record"]["summary"]["conflicts"]
            .as_array()
            .unwrap()
            .contains(&json!(conflict)));
        assert!(result["record"]["summary"]["elapsed"].is_null());
    }
}

#[test]
fn logical_root_timing_uses_the_open_boundary_and_an_explicit_shared_clock() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("root-clock.db")).unwrap();
    for (id, clock) in [("matched", "boot"), ("mismatched", "other-boot")] {
        let mut opened = at(
            &format!("{id}-open"),
            "session",
            id,
            0,
            json!({"event":"session_opened","title":id}),
        );
        let mut finished = at(
            &format!("{id}-finish"),
            "session",
            id,
            20,
            json!({"event":"finished","outcome":"unknown"}),
        );
        opened["source_clock"] = json!({"clock_id":"boot","nanoseconds":"100"});
        finished["source_clock"] = json!({"clock_id":clock,"nanoseconds":"800"});
        finished["producer"]["instance_id"] = json!("other-process");
        command(
            &mut store,
            json!({"action":"record","observations":[opened,finished]}),
        );
        let result = command(
            &mut store,
            json!({"action":"dashboard","work_session_id":id}),
        );
        if id == "matched" {
            assert_eq!(result["selected_session"]["elapsed"]["nanoseconds"], "700");
            assert_eq!(result["accounting"]["session_wall"]["seconds"], 20.0);
        } else {
            assert!(result["selected_session"]["elapsed"].is_null());
            assert!(result["accounting"]["session_wall"].is_null());
        }
    }
}

#[test]
fn qualified_queue_wait_is_attributed_separately_from_worker_and_execution_time() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("queue.db")).unwrap();
    let mut observations = vec![
        at(
            "open",
            "session",
            "root",
            0,
            json!({"event":"session_opened","title":"Root"}),
        ),
        at(
            "end",
            "session",
            "root",
            100,
            json!({"event":"finished","outcome":"unknown"}),
        ),
    ];
    let mut execution = interval("job", None, Some("runner-1"), 30, 70);
    execution[0]["body"]["context"] = json!({"labels":{"nexus.accounting.role":"container"}});
    execution[1]["source_clock"] = json!({"clock_id":"boot","nanoseconds":"30000000000"});
    execution[1]["producer"]["instance_id"] = json!("start-process");
    let mut queued = at(
        "job-queue",
        "activity",
        "job",
        10,
        json!({"event":"queued"}),
    );
    queued["source_clock"] = json!({"clock_id":"boot","nanoseconds":"10000000000"});
    observations.extend(execution);
    observations.push(queued);
    let mut step = interval("step", Some("job"), Some("runner-1"), 35, 60);
    step[0]["body"]["context"] = json!({"labels":{"nexus.accounting.role":"execution"}});
    observations.extend(step);
    command(
        &mut store,
        json!({"action":"record","observations":observations}),
    );
    let result = command(
        &mut store,
        json!({"action":"dashboard","work_session_id":"root"}),
    );
    let job = result["intervals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "job")
        .unwrap();
    assert_eq!(job["queued_at"], "2026-10-27T12:00:10Z");
    assert_eq!(job["queue_elapsed"]["nanoseconds"], "20000000000");
    assert_eq!(
        job["queue_elapsed"]["method"],
        "os_monotonic_callback_interval"
    );
    assert_eq!(result["accounting"]["queue_wait"]["seconds"], 20.0);
    assert_eq!(result["accounting"]["occupied_wall"]["seconds"], 45.0);
    assert_eq!(result["accounting"]["worker_occupancy"]["seconds"], 40.0);
    assert_eq!(result["accounting"]["unobserved"]["seconds"], 55.0);
    assert_eq!(result["accounting"]["coverage"]["queue_count"], 1);
    assert_eq!(result["accounting"]["coverage"]["known_queue_count"], 1);
}

#[test]
fn unknown_queue_clock_does_not_erase_a_known_execution_interval() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("queue-unknown.db")).unwrap();
    let mut observations = interval("task", None, Some("runner"), 30, 70);
    observations[0]["body"]["context"] = json!({"labels":{"nexus.accounting.role":"execution"}});
    observations[1]["source_clock"] = json!({"clock_id":"new-boot","nanoseconds":"30000000000"});
    let mut queued = at("queue", "activity", "task", 10, json!({"event":"queued"}));
    queued["source_clock"] = json!({"clock_id":"old-boot","nanoseconds":"10000000000"});
    observations.push(queued);
    command(
        &mut store,
        json!({"action":"record","observations":observations}),
    );
    let result = command(
        &mut store,
        json!({"action":"get","subject":{"scope":"activity","id":"task"}}),
    );
    assert!(result["record"]["summary"]["queue_elapsed"].is_null());
    assert_eq!(
        result["record"]["summary"]["elapsed"]["nanoseconds"],
        "40000000000"
    );
    assert!(result["record"]["summary"]["gaps"]
        .as_array()
        .unwrap()
        .contains(&json!("queue_clock_domain_mismatch")));
}
