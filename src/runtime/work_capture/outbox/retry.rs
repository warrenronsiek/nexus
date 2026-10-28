// @feature work-history
// @spec docs/features/work-history.md
// @boundary explicit-invalid-capture-retry: requeue only owned bounded payloads in the configured outbox, preserving their original bytes and identities.
use super::{read_entry, Outbox, MAX_ENTRIES};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct RetryInvalidReport {
    pub dry_run: bool,
    pub eligible: usize,
    pub requeued: usize,
}

impl Outbox {
    pub fn retry_invalid(&self, limit: usize, dry_run: bool) -> Result<RetryInvalidReport> {
        ensure!((1..=MAX_ENTRIES).contains(&limit), "invalid retry limit");
        let paths = invalid_candidates(&self.directory)?;
        let eligible = paths.len();
        let mut requeued = 0;
        if !dry_run {
            for path in paths.into_iter().take(limit) {
                std::fs::rename(&path, path.with_extension("json"))?;
                requeued += 1;
            }
            File::open(&self.directory)?.sync_all()?;
        }
        Ok(RetryInvalidReport {
            dry_run,
            eligible,
            requeued,
        })
    }
}

fn invalid_candidates(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)?.take(MAX_ENTRIES + 16) {
        let path = entry?.path();
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
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}
