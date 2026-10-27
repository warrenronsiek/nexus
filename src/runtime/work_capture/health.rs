// @feature work-history
// @spec docs/features/work-history.md
// @boundary capture-health-state: decode a bounded metadata-only local counter ledger; no prompts, arguments or outputs are retained.
use super::{CaptureReceipt, HookContext};
use anyhow::{bail, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static PERSISTENCE_FAILURES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureCounters {
    pub offered: u64,
    pub durably_queued: u64,
    pub duplicates: u64,
    pub rejected_replays: u64,
    pub transport_rejected: u64,
    pub transport_rejection_reasons: BTreeMap<String, u64>,
    pub last_adapter_rejection_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_adapter_host: Option<String>,
    pub lost: u64,
    pub delivered: u64,
    pub rejection_reasons: BTreeMap<String, u64>,
    pub loss_reasons: BTreeMap<String, u64>,
    pub last_captured_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_delivered_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_source: Option<CaptureSource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSource {
    pub agent: String,
    pub host_session_id: String,
}

pub(super) fn read(directory: &Path) -> Result<CaptureCounters> {
    let path = directory.join(".capture-health");
    match super::outbox::read_metadata(&path, 16 * 1024) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(CaptureCounters::default())
        }
        Err(error) => Err(error),
    }
}

fn update(directory: &Path, change: impl FnOnce(&mut CaptureCounters)) -> Result<()> {
    prepare_directory(directory)?;
    let _lock = lock_health(directory)?;
    let mut health = read(directory)?;
    change(&mut health);
    publish_counters(directory, &health)
}

fn publish_counters(directory: &Path, health: &CaptureCounters) -> Result<()> {
    let file =
        super::outbox::temporary_metadata(directory, &serde_json::to_vec(&health)?, ".health-")?;
    file.persist(directory.join(".capture-health"))?;
    sync_directory(directory)
}

fn prepare_directory(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let metadata = std::fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        bail!("capture health requires private owned directory");
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(Into::into)
}

fn lock_health(directory: &Path) -> Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(".capture-health-lock"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(lock)
}

fn sync_directory(directory: &Path) -> Result<()> {
    std::fs::File::open(directory)?
        .sync_all()
        .map_err(Into::into)
}

fn retain(result: Result<()>) -> bool {
    if result.is_err() {
        PERSISTENCE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    true
}

pub(super) fn adapter_rejected(directory: &Path, host: &str) -> bool {
    retain(update(directory, |health| {
        health.offered = health.offered.saturating_add(1);
        health.transport_rejected = health.transport_rejected.saturating_add(1);
        increment(
            &mut health.transport_rejection_reasons,
            "invalid_host_event",
            1,
        );
        health.last_adapter_rejection_at = Some(chrono::Utc::now());
        health.last_adapter_host = Some(host.into());
    }))
}

pub(super) fn capture(directory: &Path, context: &HookContext, receipt: &CaptureReceipt) {
    retain(update(directory, |health| {
        health.offered = health.offered.saturating_add(receipt.offered as u64);
        health.durably_queued = health.durably_queued.saturating_add(receipt.queued as u64);
        health.duplicates = health.duplicates.saturating_add(receipt.duplicates as u64);
        health.rejected_replays = health
            .rejected_replays
            .saturating_add(receipt.rejected as u64);
        health.lost = health.lost.saturating_add(receipt.lost as u64);
        for (reason, count) in &receipt.loss_reasons {
            increment(&mut health.loss_reasons, reason, *count as u64);
        }
        for (reason, count) in &receipt.rejection_reasons {
            increment(&mut health.rejection_reasons, reason, *count as u64);
        }
        if receipt.queued + receipt.duplicates > 0 {
            health.last_captured_at = Some(chrono::Utc::now());
        }
        health.last_source = Some(CaptureSource {
            agent: identity(&context.agent),
            host_session_id: identity(&context.session_id),
        });
    }));
}

pub(super) fn settled(directory: &Path, reason: Option<&str>) {
    retain(update(directory, |health| match reason {
        Some(reason) => increment(&mut health.rejection_reasons, reason, 1),
        None => {
            health.delivered = health.delivered.saturating_add(1);
            health.last_delivered_at = Some(chrono::Utc::now());
        }
    }));
}

pub(super) fn admission_loss(directory: &Path, count: u64) {
    retain(update(directory, |health| {
        health.offered = health.offered.saturating_add(count);
        health.lost = health.lost.saturating_add(count);
        increment(&mut health.loss_reasons, "admission_capacity", count);
    }));
}

fn increment(counts: &mut BTreeMap<String, u64>, reason: &str, amount: u64) {
    let count = counts.entry(reason.into()).or_default();
    *count = count.saturating_add(amount);
}

fn identity(value: &str) -> String {
    if value.len() <= 256 {
        value.into()
    } else {
        format!("blake3:{}", blake3::hash(value.as_bytes()))
    }
}

pub(super) fn persistence_failures() -> u64 {
    PERSISTENCE_FAILURES.load(Ordering::Relaxed)
}
