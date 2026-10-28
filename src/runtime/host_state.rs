// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @boundary private-host-state: typed bounded per-host metadata shares owned-file, locking, capacity, and atomic publication policy.
use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

pub(crate) struct HostStateFile<T> {
    path: PathBuf,
    limit: u64,
    _lock: File,
    _type: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned> HostStateFile<T> {
    pub(crate) fn open_locked(path: &Path, limit: u64) -> Result<Self> {
        let directory = path.parent().context("host state directory unavailable")?;
        Ok(Self {
            path: path.to_owned(),
            limit,
            _lock: crate::private_state::lock_directory(directory)?,
            _type: PhantomData,
        })
    }

    pub(crate) fn read(path: &Path, limit: u64) -> Result<Option<T>> {
        crate::private_state::read(path, limit)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn publish(&self, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        anyhow::ensure!(
            bytes.len() as u64 <= self.limit,
            "host state exceeds its byte limit"
        );
        check_capacity(&self.path)?;
        crate::private_state::write(&self.path, &bytes)
    }

    pub(crate) fn remove(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn check_capacity(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let directory = path.parent().context("host state directory unavailable")?;
    let count = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "json")
        })
        .take(4096)
        .count();
    anyhow::ensure!(count < 4096, "host state capacity exhausted");
    Ok(())
}
