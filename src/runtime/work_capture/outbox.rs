// @feature work-history
// @spec docs/features/work-history.md
// @boundary cached-work-envelopes: compare bounded typed local capture envelopes to preserve canonical callback receipts and reject changed event bodies.
use crate::coordination::{api::WorkCommand, domain::HookContext};
use crate::work_history::{WorkAction, WorkEvent, WorkObservationInput};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static WRITER: Mutex<()> = Mutex::new(());

mod retry;
pub use retry::RetryInvalidReport;

#[cfg(test)]
mod retry_tests;

#[derive(Debug, thiserror::Error)]
#[error("work outbox capacity exceeded")]
pub(super) struct CapacityExceeded;

#[derive(Debug, Default, Serialize)]
pub struct CaptureHealth {
    pub queued: usize,
    pub rejected: usize,
    pub bytes: u64,
    pub dropped: u64,
    pub rejected_evicted: u64,
    pub oldest_queued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub recent_delivered: usize,
    pub capture: Option<super::health::CaptureCounters>,
    pub admission_lost_process: u64,
    pub worker_failed_process: u64,
    pub health_persistence_failed_process: u64,
    pub pending_callbacks_process: u64,
}

pub(super) struct Pending {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingPhase {
    Record,
    ReconcileCallback,
}

impl Pending {
    pub fn phase(&self) -> PendingPhase {
        if self
            .path
            .extension()
            .is_some_and(|extension| extension == "callback")
        {
            PendingPhase::ReconcileCallback
        } else {
            PendingPhase::Record
        }
    }
}

fn is_pending(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json" || extension == "callback")
}

pub(super) enum Enqueued {
    New,
    Duplicate,
    Rejected,
    Conflict,
}

fn single_observation(bytes: &[u8]) -> Option<(HookContext, WorkObservationInput)> {
    let command: WorkCommand = serde_json::from_slice(bytes).ok()?;
    let WorkAction::Record { observations } = command.command else {
        return None;
    };
    let [observation]: [WorkObservationInput; 1] = observations.try_into().ok()?;
    Some((command.context, observation))
}

fn same_cached_boundary(cached: &[u8], incoming: &[u8]) -> bool {
    if cached == incoming {
        return true;
    }
    let Some((context, known)) = single_observation(cached) else {
        return false;
    };
    let Some((_, offered)) = single_observation(incoming) else {
        return false;
    };
    super::callback_replay_matches(&known, &offered)
        || same_stable_container(&context, &known, &offered)
}

fn same_stable_container(
    context: &HookContext,
    known: &WorkObservationInput,
    offered: &WorkObservationInput,
) -> bool {
    let container = match known.body {
        WorkEvent::SessionOpened { .. } | WorkEvent::ActivityDeclared { .. } => true,
        WorkEvent::Started => {
            known.subject.id()
                == crate::work_history::host_agent_identity(&context.agent, &context.session_id)
        }
        _ => false,
    };
    container && super::same_companion_body(known, offered)
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
            _lock: crate::private_state::lock_file(directory, ".lock")?,
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

    pub fn enqueue_once(&self, id: &str, bytes: &[u8]) -> Result<Enqueued> {
        for extension in [
            "json",
            "callback",
            "seen",
            "rejected-invalid",
            "rejected-conflict",
            "rejected-corrupt",
        ] {
            let existing = self.directory.join(format!("{id}.{extension}"));
            if existing.exists() {
                return Ok(if extension.starts_with("rejected-") {
                    Enqueued::Rejected
                } else if same_cached_boundary(&read_entry(&existing)?, bytes) {
                    Enqueued::Duplicate
                } else {
                    Enqueued::Conflict
                });
            }
        }
        self.enqueue(id, bytes)?;
        Ok(Enqueued::New)
    }

    pub fn pending(&self) -> Result<Vec<Pending>> {
        let mut paths = self.entries()?;
        paths.retain(|path| is_pending(path));
        paths.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
        let mut entries = Vec::new();
        for path in paths.into_iter().take(16) {
            match read_entry(&path) {
                Ok(bytes) => entries.push(Pending { path, bytes }),
                Err(_) => {
                    self.reject(
                        &Pending {
                            path,
                            bytes: Vec::new(),
                        },
                        "corrupt",
                    )?;
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
        File::open(&self.directory)?.sync_all()?;
        super::health::settled(&self.directory, Some(reason));
        Ok(())
    }

    pub fn defer_callback(&self, pending: &Pending) -> Result<()> {
        if pending.path.exists() && pending.phase() == PendingPhase::Record {
            std::fs::rename(&pending.path, pending.path.with_extension("callback"))?;
            File::open(&self.directory)?.sync_all()?;
        }
        Ok(())
    }

    pub fn acknowledge(&self, pending: &Pending) -> Result<()> {
        if pending.path.exists() {
            std::fs::rename(&pending.path, pending.path.with_extension("seen"))?;
            File::open(&self.directory)?.sync_all()?;
            super::health::settled(&self.directory, None);
        }
        Ok(())
    }

    pub fn health(&self) -> Result<CaptureHealth> {
        let (admission_lost_process, worker_failed_process, pending_callbacks_process) =
            super::handoff::process_losses();
        let mut health = CaptureHealth {
            dropped: counter(&self.directory, ".loss"),
            rejected_evicted: counter(&self.directory, ".rejected-evicted"),
            capture: super::health::read(&self.directory).ok(),
            admission_lost_process,
            worker_failed_process,
            pending_callbacks_process,
            health_persistence_failed_process: super::health::persistence_failures(),
            ..CaptureHealth::default()
        };
        for path in self.entries()? {
            let metadata = std::fs::symlink_metadata(&path)?;
            health.bytes = health.bytes.saturating_add(metadata.len());
            if is_pending(&path) {
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
        for entry in std::fs::read_dir(&self.directory)?.take(MAX_ENTRIES + 16) {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
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
            .filter(|path| !is_pending(path))
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
            return Err(CapacityExceeded.into());
        }
        Ok(())
    }

    fn publish(&self, destination: &Path, bytes: &[u8]) -> Result<()> {
        crate::private_state::temporary_in(&self.directory, bytes)?
            .persist_noclobber(destination)?;
        File::open(&self.directory)?.sync_all().map_err(Into::into)
    }
}

pub(super) fn record_capture(
    directory: &Path,
    context: &super::HookContext,
    receipt: &super::CaptureReceipt,
) {
    super::health::capture(directory, context, receipt);
}

fn counter(directory: &Path, name: &str) -> u64 {
    read_entry(&directory.join(name))
        .ok()
        .and_then(|bytes| std::str::from_utf8(&bytes).ok()?.parse().ok())
        .unwrap_or(0)
}

fn increment_counter(directory: &Path, name: &str) -> Result<()> {
    let value = counter(directory, name).saturating_add(1).to_string();
    crate::private_state::write(&directory.join(name), value.as_bytes())
}

fn read_entry(path: &Path) -> Result<Vec<u8>> {
    crate::private_state::read(path, MAX_ENTRY_BYTES as u64)?
        .context("missing local metadata entry")
}

#[cfg(test)]
mod tests;
