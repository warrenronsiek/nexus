// @feature work-history
// @spec docs/features/work-history.md
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static WRITER: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize)]
pub struct CaptureHealth {
    pub queued: usize,
    pub rejected: usize,
    pub bytes: u64,
    pub dropped: u64,
    pub rejected_evicted: u64,
    pub oldest_queued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub recent_delivered: usize,
}

pub(super) struct Pending {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

pub(super) struct Outbox {
    directory: PathBuf,
    _lock: File,
    _writer: MutexGuard<'static, ()>,
}

impl Outbox {
    pub fn open(directory: &Path) -> Result<Self> {
        let owner = match WRITER.try_lock() {
            Ok(owner) => owner,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => bail!("work outbox is busy"),
        };
        Self::open_owned(directory, owner)
    }

    pub fn writer(directory: &Path) -> Result<Self> {
        // Durable file syncs can exceed the external-owner deadline. Queue this
        // process's producers before starting that deadline; observers never wait
        // on this lane. The mutex protects ownership only, so poison is recoverable.
        Self::open_owned(
            directory,
            WRITER.lock().unwrap_or_else(PoisonError::into_inner),
        )
    }

    fn open_owned(directory: &Path, owner: MutexGuard<'static, ()>) -> Result<Self> {
        Ok(Self {
            directory: directory.into(),
            _lock: lock_directory(directory)?,
            _writer: owner,
        })
    }

    pub fn enqueue(&self, id: &str, bytes: &[u8]) -> Result<()> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            bail!("invalid local event identity");
        }
        let destination = self.directory.join(format!("{id}.json"));
        if destination.exists() {
            if read_entry(&destination)? == bytes {
                return Ok(());
            }
            bail!("conflicting local event identity");
        }
        self.ensure_capacity(bytes.len())?;
        self.publish(&destination, bytes)
    }

    pub fn enqueue_once(&self, id: &str, bytes: &[u8]) -> Result<()> {
        if [
            "json",
            "seen",
            "rejected-invalid",
            "rejected-conflict",
            "rejected-corrupt",
        ]
        .iter()
        .any(|extension| self.directory.join(format!("{id}.{extension}")).exists())
        {
            return Ok(());
        }
        self.enqueue(id, bytes)
    }

    pub fn pending(&self) -> Result<Vec<Pending>> {
        let mut paths = self.entries()?;
        paths.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
        paths.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
        let mut entries = Vec::new();
        for path in paths.into_iter().take(16) {
            match read_entry(&path) {
                Ok(bytes) => entries.push(Pending { path, bytes }),
                Err(_) => {
                    std::fs::rename(&path, path.with_extension("rejected-corrupt"))?;
                }
            }
        }
        Ok(entries)
    }

    pub fn reject(&self, pending: &Pending, reason: &str) -> Result<()> {
        let reason = match reason {
            "invalid" => "invalid",
            "conflict" => "conflict",
            _ => "corrupt",
        };
        std::fs::rename(
            &pending.path,
            pending.path.with_extension(format!("rejected-{reason}")),
        )?;
        Ok(())
    }

    pub fn acknowledge(&self, pending: &Pending) -> Result<()> {
        if pending.path.exists() {
            std::fs::rename(&pending.path, pending.path.with_extension("seen"))?;
        }
        Ok(())
    }

    pub fn health(&self) -> Result<CaptureHealth> {
        let mut health = CaptureHealth {
            dropped: counter(&self.directory, ".loss"),
            rejected_evicted: counter(&self.directory, ".rejected-evicted"),
            ..CaptureHealth::default()
        };
        for path in self.entries()? {
            let metadata = std::fs::symlink_metadata(&path)?;
            health.bytes = health.bytes.saturating_add(metadata.len());
            if path.extension().is_some_and(|ext| ext == "json") {
                health.queued += 1;
                let time = metadata
                    .modified()
                    .ok()
                    .map(chrono::DateTime::<chrono::Utc>::from);
                health.oldest_queued_at = health.oldest_queued_at.into_iter().chain(time).min();
            } else if path.extension().is_some_and(|ext| ext == "seen") {
                health.recent_delivered += 1;
            } else {
                health.rejected += 1;
            }
        }
        Ok(health)
    }

    fn entries(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&self.directory)?.take(MAX_ENTRIES + 3) {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
            {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    fn evict_settled(&self, incoming: u64) -> Result<()> {
        let mut health = self.health()?;
        let mut settled: Vec<_> = self
            .entries()?
            .into_iter()
            .filter(|path| !path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        settled.sort_by_key(|path| {
            (
                !path.extension().is_some_and(|ext| ext == "seen"),
                std::fs::metadata(path).and_then(|m| m.modified()).ok(),
            )
        });
        for path in settled {
            if health.queued + health.rejected + health.recent_delivered < MAX_ENTRIES
                && health.bytes.saturating_add(incoming) <= MAX_BYTES
            {
                break;
            }
            let bytes = std::fs::metadata(&path)?.len();
            std::fs::remove_file(&path)?;
            if path.extension().is_some_and(|ext| ext == "seen") {
                health.recent_delivered -= 1;
            } else {
                increment_counter(&self.directory, ".rejected-evicted")?;
                health.rejected -= 1;
            }
            health.bytes = health.bytes.saturating_sub(bytes);
        }
        Ok(())
    }
    fn ensure_capacity(&self, incoming: usize) -> Result<()> {
        if incoming <= MAX_ENTRY_BYTES {
            self.evict_settled(incoming as u64)?;
        }
        let health = self.health()?;
        if incoming > MAX_ENTRY_BYTES
            || health.queued + health.rejected + health.recent_delivered >= MAX_ENTRIES
            || health.bytes.saturating_add(incoming as u64) > MAX_BYTES
        {
            let _ = increment_counter(&self.directory, ".loss");
            bail!("work outbox capacity exceeded");
        }
        Ok(())
    }

    fn publish(&self, destination: &Path, bytes: &[u8]) -> Result<()> {
        write_temporary(&self.directory, bytes)?.persist_noclobber(destination)?;
        File::open(&self.directory)?.sync_all().map_err(Into::into)
    }
}

fn lock_directory(directory: &Path) -> Result<File> {
    std::fs::create_dir_all(directory)?;
    let metadata = std::fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        bail!("work outbox must be a private owned directory");
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(".lock"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => return Err(error).context("work outbox is busy"),
        }
    }
    Ok(lock)
}

fn counter(directory: &Path, name: &str) -> u64 {
    read_entry(&directory.join(name))
        .ok()
        .and_then(|bytes| std::str::from_utf8(&bytes).ok()?.parse().ok())
        .unwrap_or(0)
}

fn increment_counter(directory: &Path, name: &str) -> Result<()> {
    let value = counter(directory, name).saturating_add(1).to_string();
    write_temporary(directory, value.as_bytes())?.persist(directory.join(name))?;
    File::open(directory)?.sync_all().map_err(Into::into)
}

fn write_temporary(directory: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("pending-")
        .suffix(".tmp")
        .tempfile_in(directory)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}

fn read_entry(path: &Path) -> Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES as u64 {
        bail!("invalid local metadata entry");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_ENTRY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ENTRY_BYTES {
        bail!("oversized local metadata entry");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
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
}
