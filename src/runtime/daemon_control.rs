// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @entrypoint inspect
// @boundary private-daemon-control: same-user local sockets exchange exact managed process configuration, never remote MCP
use crate::{Config, LoadedConfig};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod client;
mod lifecycle;
mod state;
pub use client::{inspect, stop};
pub use lifecycle::{restart, resume};
pub(super) use state::DaemonControl;

const METHOD: &str = "_update_control";
const MAX_SNAPSHOT: u64 = 1024 * 1024;

/// Contains private resolved configuration; deliberately has no Debug implementation.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonSnapshot {
    pub instance_id: String,
    pub config: Config,
    pub sources: Vec<PathBuf>,
    pub config_hash: String,
    pub directory: PathBuf,
    pub executable: PathBuf,
}
impl DaemonSnapshot {
    pub fn loaded(&self) -> Result<LoadedConfig> {
        self.config
            .validate()
            .map_err(|_| anyhow::anyhow!("managed daemon configuration is invalid"))?;
        ensure!(
            !self.instance_id.is_empty() && self.instance_id.len() <= 128,
            "invalid managed daemon identity"
        );
        ensure!(
            !self.config_hash.is_empty() && self.config_hash.len() <= 512,
            "invalid managed configuration identity"
        );
        ensure!(
            self.directory.is_absolute() && self.executable.is_absolute(),
            "managed daemon paths must be absolute"
        );
        Ok(LoadedConfig {
            config: self.config.clone(),
            sources: self.sources.clone(),
            hash: self.config_hash.clone(),
        })
    }
    pub(super) fn capture(loaded: &LoadedConfig) -> Result<Self> {
        Ok(Self {
            instance_id: uuid::Uuid::new_v4().to_string(),
            config: loaded.config.clone(),
            sources: loaded.sources.clone(),
            config_hash: loaded.hash.clone(),
            directory: std::env::current_dir()?,
            executable: std::env::current_exe()?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ControlRequest {
    Inspect,
    Stop { instance_id: String },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum ControlResponse {
    Snapshot { snapshot: Box<DaemonSnapshot> },
    Stopping { instance_id: String },
    Rejected { reason: String },
}

pub(super) fn decode_snapshot(path: &Path) -> Result<DaemonSnapshot> {
    let bytes = crate::private_state::read(path, MAX_SNAPSHOT)?
        .context("managed daemon snapshot is missing")?;
    let snapshot: DaemonSnapshot = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid managed daemon snapshot"))?;
    snapshot.loaded()?;
    Ok(snapshot)
}
