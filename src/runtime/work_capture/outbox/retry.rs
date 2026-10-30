// @feature work-history
// @spec docs/features/work-history.md
// @boundary explicit-invalid-capture-retry: requeue only owned bounded payloads in the configured outbox, preserving their original bytes and identities.
use super::{read_entry, Outbox, MAX_ENTRIES};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::fs::File;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
pub enum InvalidRetry {
    Preview,
    Requeue { limit: usize },
}

#[derive(Debug, Serialize)]
pub struct RetryInvalidReport {
    pub dry_run: bool,
    pub eligible: usize,
    pub requeued: usize,
}

impl Outbox {
    pub fn retry_invalid(&self, mode: InvalidRetry) -> Result<RetryInvalidReport> {
        if let InvalidRetry::Requeue { limit } = mode {
            ensure!((1..=MAX_ENTRIES).contains(&limit), "invalid retry limit");
        }
        let (eligible, paths) = self.invalid_candidates()?;
        let mut requeued = 0;
        if let InvalidRetry::Requeue { limit } = mode {
            for path in paths.into_iter().take(limit) {
                std::fs::rename(&path, path.with_extension("json"))?;
                requeued += 1;
            }
            File::open(&self.directory)?.sync_all()?;
        }
        Ok(RetryInvalidReport {
            dry_run: matches!(mode, InvalidRetry::Preview),
            eligible,
            requeued,
        })
    }

    fn invalid_candidates(&self) -> Result<(usize, Vec<PathBuf>)> {
        let mut paths = Vec::new();
        let mut eligible = 0;
        for path in super::inventory::entries(&self.directory)? {
            let path = path?;
            if path
                .extension()
                .is_none_or(|extension| extension != "rejected-invalid")
            {
                continue;
            }
            read_entry(&path)?;
            ensure!(
                !["json", "seen", "callback"]
                    .iter()
                    .any(|extension| path.with_extension(extension).exists()),
                "capture already has another delivery state"
            );
            eligible += 1;
            paths.push(path);
            paths.sort();
            paths.truncate(MAX_ENTRIES);
        }
        Ok((eligible, paths))
    }
}
