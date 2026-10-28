// @feature runtime
// @feature coordination
// @feature configuration
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @spec docs/features/configuration.md
// @boundary private-checkout-journal: persist bounded routing metadata and credential-file paths, never credential values or tool payloads, for teardown after the MCP companion exits.
use crate::config::{ClientConfig, LoadedConfig, RuntimeConfig, StorageConfig};
use crate::coordination::domain::HookContext;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::host_state::HostStateFile;
const MAX_BYTES: u64 = 256 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Visit {
    root: String,
    hash: String,
    client: ClientConfig,
    storage: StorageConfig,
    runtime: RuntimeConfig,
}

impl Visit {
    pub(crate) fn restore(
        &self,
        base: &LoadedConfig,
        context: &HookContext,
    ) -> Result<(LoadedConfig, HookContext)> {
        let mut config = base.config.clone();
        config.client = self.client.clone();
        config.storage = self.storage.clone();
        config.runtime = self.runtime.clone();
        config.validate()?;
        let hash = blake3::hash(toml::to_string(&config)?.as_bytes())
            .to_hex()
            .to_string();
        let mut context = context.clone();
        context.project_root = Some(self.root.clone());
        Ok((
            LoadedConfig {
                config,
                hash,
                sources: base.sources.clone(),
            },
            context,
        ))
    }
}

pub(crate) fn remember(
    base: &LoadedConfig,
    target: &LoadedConfig,
    context: &HookContext,
) -> Result<()> {
    let root = context
        .project_root
        .as_ref()
        .context("checkout root missing")?;
    let path = journal_path(base, context)?;
    let contains = |visits: &[Visit]| {
        visits
            .iter()
            .any(|visit| visit.root == *root && visit.hash == target.hash)
    };
    if contains(&read(&path)?) {
        return Ok(());
    }
    update_visit(&path, target, root)
}

fn update_visit(path: &Path, target: &LoadedConfig, root: &str) -> Result<()> {
    let file = HostStateFile::<Vec<Visit>>::open_locked(path, MAX_BYTES)?;
    let mut visits = read(path)?;
    if visits
        .iter()
        .any(|visit| visit.root == root && visit.hash == target.hash)
    {
        return Ok(());
    }
    anyhow::ensure!(visits.len() < 32, "host checkout visit limit reached");
    visits.push(Visit {
        root: root.to_owned(),
        hash: target.hash.clone(),
        client: target.config.client.clone(),
        storage: target.config.storage.clone(),
        runtime: target.config.runtime.clone(),
    });
    file.publish(&visits)
}

pub(crate) fn observed(base: &LoadedConfig, context: &HookContext) -> Result<Vec<Visit>> {
    read(&journal_path(base, context)?)
}

pub(crate) fn clear(base: &LoadedConfig, context: &HookContext) -> Result<()> {
    let path = journal_path(base, context)?;
    HostStateFile::<Vec<Visit>>::open_locked(&path, MAX_BYTES)?.remove()
}

fn journal_path(base: &LoadedConfig, context: &HookContext) -> Result<PathBuf> {
    let directory = super::work_capture::directory(base)?.join("checkout-visits");
    crate::private_state::directory_at(&directory)?;
    Ok(directory.join(format!(
        "{}.json",
        crate::identity::stable_id(&[&context.agent, &context.session_id])
    )))
}

fn read(path: &Path) -> Result<Vec<Visit>> {
    let visits = HostStateFile::<Vec<Visit>>::read(path, MAX_BYTES)?.unwrap_or_default();
    if visits.len() > 32
        || visits
            .iter()
            .any(|v| v.root.len() > 4096 || !Path::new(&v.root).is_absolute())
    {
        bail!("invalid checkout journal metadata");
    }
    Ok(visits)
}
