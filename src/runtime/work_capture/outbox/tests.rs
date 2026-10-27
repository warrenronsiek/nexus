// @feature work-history
// @spec docs/features/work-history.md
// @boundary outbox-fixtures: inspect bounded local metadata files to verify durable capture and corruption handling.
use super::*;
use std::os::unix::fs::PermissionsExt;

// Ordering tests supply distinct times instead of relying on filesystem clock precision.
fn set_fixture_time(queue: &Outbox, id: &str, seconds: u64) {
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    File::open(queue.directory.join(format!("{id}.json")))
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
}

#[test]
fn concurrent_producers_survive_a_slow_writer_without_losing_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let holder = Outbox::writer(temp.path()).unwrap();
    let ready = std::sync::Barrier::new(3);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|worker| {
                let ready = &ready;
                let path = temp.path();
                scope.spawn(move || {
                    ready.wait();
                    let queue = Outbox::writer(path)?;
                    for boundary in ["declared", "started", "finished"] {
                        let identity = format!("worker-{worker}-{boundary}");
                        queue.enqueue(&identity, identity.as_bytes())?;
                    }
                    anyhow::Ok(())
                })
            })
            .collect();
        ready.wait();
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(holder);
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
    });
    let queue = Outbox::open(temp.path()).unwrap();
    let mut bytes: Vec<_> = queue
        .pending()
        .unwrap()
        .into_iter()
        .map(|entry| entry.bytes)
        .collect();
    bytes.sort();
    assert_eq!(bytes.len(), 6);
    for (worker, entries) in bytes.as_chunks::<3>().0.iter().enumerate() {
        let expected: Vec<_> = ["declared", "finished", "started"]
            .map(|boundary| format!("worker-{worker}-{boundary}").into_bytes())
            .into();
        assert_eq!(entries.as_slice(), expected);
    }
}

#[test]
fn observers_do_not_wait_behind_a_slow_process_writer() {
    let temp = tempfile::tempdir().unwrap();
    let _holder = Outbox::writer(temp.path()).unwrap();
    let started = std::time::Instant::now();
    assert!(Outbox::open(temp.path()).is_err());
    assert!(started.elapsed() < std::time::Duration::from_millis(100));
}

#[test]
fn a_persistent_external_owner_retains_the_bounded_failure_path() {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let temp = tempfile::tempdir().unwrap();
    let mut external = Command::new("python3").args(["-u", "-c",
        "import fcntl,sys; f=open(sys.argv[1], 'w'); fcntl.flock(f, fcntl.LOCK_EX); print('ready'); sys.stdin.read()"])
        .arg(temp.path().join(".lock")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut ready = String::new();
    std::io::BufReader::new(external.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "ready\n");
    let started = std::time::Instant::now();
    let result = Outbox::writer(temp.path());
    let elapsed = started.elapsed();
    drop(external.stdin.take());
    assert!(external.wait().unwrap().success());
    assert!(result.is_err());
    assert!(elapsed < std::time::Duration::from_secs(1));
}

#[test]
fn a_panicked_writer_does_not_disable_later_capture() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().to_path_buf();
    assert!(std::thread::spawn(move || {
        let _holder = Outbox::writer(&path).unwrap();
        panic!("interrupted producer");
    })
    .join()
    .is_err());
    let queue = Outbox::writer(temp.path()).unwrap();
    queue.enqueue("recovered", b"original metadata").unwrap();
    assert_eq!(queue.pending().unwrap()[0].bytes, b"original metadata");
}

#[test]
fn saturated_quarantine_recovers_valid_delivery_without_discarding_pending() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Outbox::open(temp.path()).unwrap();
    fill_quarantine(&queue, MAX_ENTRIES);
    drop(queue);
    let queue = Outbox::open(temp.path()).unwrap();
    queue.enqueue("valid-one", b"valid one").unwrap();
    queue.enqueue("valid-two", b"valid two").unwrap();
    set_fixture_time(&queue, "valid-one", 1);
    set_fixture_time(&queue, "valid-two", 2);
    let pending = queue.pending().unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].bytes, b"valid one");
    assert_eq!(pending[1].bytes, b"valid two");
    queue.acknowledge(&pending[0]).unwrap();
    queue.enqueue("valid-three", b"valid three").unwrap();
    let pending = queue.pending().unwrap();
    assert_eq!(pending[0].bytes, b"valid two");
    assert_eq!(pending[1].bytes, b"valid three");
    assert_recovered_health(&queue);
    drop(queue);
    let queue = Outbox::open(temp.path()).unwrap();
    for entry in queue.pending().unwrap() {
        queue.acknowledge(&entry).unwrap();
    }
    assert_eq!(queue.health().unwrap().queued, 0);
    assert_eq!(queue.health().unwrap().rejected_evicted, 2);
}

fn fill_quarantine(queue: &Outbox, count: usize) {
    for index in 0..count {
        queue.enqueue(&format!("bad-{index}"), b"bad").unwrap();
        queue
            .reject(&queue.pending().unwrap().pop().unwrap(), "invalid")
            .unwrap();
    }
}

fn assert_recovered_health(queue: &Outbox) {
    let health = queue.health().unwrap();
    assert_eq!(
        (health.queued, health.rejected, health.recent_delivered),
        (2, MAX_ENTRIES - 2, 0)
    );
    assert_eq!(health.bytes, ((MAX_ENTRIES - 2) * 3 + 20) as u64);
    assert_eq!(health.rejected_evicted, 2);
    assert_eq!(health.dropped, 0);
    let retained = queue
        .entries()
        .unwrap()
        .into_iter()
        .find(|path| {
            path.extension()
                .is_some_and(|ext| ext == "rejected-invalid")
        })
        .unwrap();
    assert_eq!(
        std::fs::metadata(retained).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn oversized_observations_do_not_erase_retained_quarantine() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Outbox::open(temp.path()).unwrap();
    fill_quarantine(&queue, MAX_ENTRIES);
    assert!(queue
        .enqueue("oversized", &vec![0; MAX_ENTRY_BYTES + 1])
        .is_err());
    let health = queue.health().unwrap();
    assert_eq!(health.rejected, MAX_ENTRIES);
    assert_eq!(health.rejected_evicted, 0);
    assert_eq!(health.dropped, 1);
}

#[test]
fn context_bridge_directories_do_not_consume_payload_capacity() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Outbox::open(temp.path()).unwrap();
    std::fs::create_dir(temp.path().join("contexts")).unwrap();
    std::fs::write(temp.path().join("contexts/metadata.json"), b"context").unwrap();
    assert_eq!(queue.health().unwrap().rejected, 0);
    for index in 0..MAX_ENTRIES {
        queue
            .enqueue(&format!("event-{index}"), b"boundary")
            .unwrap();
    }
    assert_eq!(queue.health().unwrap().queued, MAX_ENTRIES);
    assert!(temp.path().join("contexts/metadata.json").exists());
}

#[test]
fn corrupt_health_counters_do_not_disable_durable_capture() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Outbox::open(temp.path()).unwrap();
    std::fs::write(temp.path().join(".capture-health"), b"not valid metadata").unwrap();
    assert!(queue.enqueue("still-captured", b"boundary").is_ok());
    let health = serde_json::to_value(queue.health().unwrap()).unwrap();
    assert!(health["capture"].is_null());
    assert_eq!(health["queued"], 1);
}

#[test]
fn a_late_conflict_after_recent_journal_eviction_is_not_requeued_forever() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Outbox::open(temp.path()).unwrap();
    queue.enqueue_once("root", b"original boundary").unwrap();
    let original = queue.pending().unwrap().pop().unwrap();
    queue.acknowledge(&original).unwrap();
    for index in 0..MAX_ENTRIES {
        queue
            .enqueue(&format!("call-{index}"), b"tool boundary")
            .unwrap();
    }
    assert!(!temp.path().join("root.seen").exists());
    let old = queue.pending().unwrap().remove(0);
    queue.acknowledge(&old).unwrap();
    queue.enqueue_once("root", b"new callback time").unwrap();
    queue
        .reject(
            &Pending {
                path: temp.path().join("root.json"),
                bytes: Vec::new(),
            },
            "conflict",
        )
        .unwrap();
    let old = queue.pending().unwrap().remove(0);
    queue.acknowledge(&old).unwrap();
    queue
        .enqueue_once("root", b"another callback time")
        .unwrap();
    assert!(!temp.path().join("root.json").exists());
    assert_eq!(queue.health().unwrap().rejected, 1);
}

#[test]
fn pending_metadata_survives_restart_and_rejections_do_not_block_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("queue");
    let queue = Outbox::open(&path).unwrap();
    queue.enqueue("first", b"first-observation").unwrap();
    queue.enqueue("second", b"second-observation").unwrap();
    set_fixture_time(&queue, "first", 1);
    set_fixture_time(&queue, "second", 2);
    drop(queue);

    let queue = Outbox::open(&path).unwrap();
    let entries = queue.pending().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].bytes, b"first-observation");
    queue.reject(&entries[0], "invalid").unwrap();
    let second = queue.pending().unwrap().pop().unwrap();
    assert_eq!(second.bytes, b"second-observation");
    queue.acknowledge(&second).unwrap();
    let health = queue.health().unwrap();
    assert_eq!((health.queued, health.rejected), (0, 1));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(path.join("first.rejected-invalid"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn exact_replay_is_idempotent_and_the_total_queue_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Outbox::open(temp.path()).unwrap();
    queue.enqueue("same", b"original").unwrap();
    queue.enqueue("same", b"original").unwrap();
    assert!(queue.enqueue("same", b"changed").is_err());
    assert_eq!(queue.health().unwrap().queued, 1);
    assert!(queue
        .enqueue("large", &vec![0; MAX_ENTRY_BYTES + 1])
        .is_err());
    for index in 1..MAX_ENTRIES {
        queue
            .enqueue(&format!("event-{index}"), b"metadata")
            .unwrap();
    }
    assert!(queue.enqueue("overflow", b"metadata").is_err());
    assert_eq!(queue.health().unwrap().queued, MAX_ENTRIES);
}
