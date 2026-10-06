// @feature observability-ui
// @feature runtime
// @feature installation
// @spec docs/features/observability-ui.md
// @spec docs/features/runtime.md
// @spec docs/features/installation.md
// @entrypoint open
use crate::config::LoadedConfig;
use anyhow::{bail, Context, Result};
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const DASHBOARD: &str = include_str!("../../pi-extension/dist/nexus-tui.mjs");

pub async fn open(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    working_directory: &Path,
) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("Run `nexus tui` from an interactive terminal");
    }
    let cache = loaded
        .config
        .storage
        .database_path
        .parent()
        .context("Nexus database must have a parent directory")?
        .join("tui");
    let mut command = terminal_command(&cache, working_directory, explicit_config)?;
    Err(command.exec()).context("open Nexus terminal dashboard")
}

fn bundled_dashboard(cache: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(cache)?;
    let hash = blake3::hash(DASHBOARD.as_bytes()).to_hex();
    let bundle = cache.join(format!("dashboard-{hash}.mjs"));
    if std::fs::read_to_string(&bundle).ok().as_deref() != Some(DASHBOARD) {
        let pending = cache.join(format!("dashboard-{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&pending, DASHBOARD)?;
        std::fs::rename(pending, &bundle)?;
    }
    Ok(bundle)
}

fn terminal_command(
    cache: &Path,
    working_directory: &Path,
    explicit_config: Option<&Path>,
) -> Result<Command> {
    let bundle = bundled_dashboard(cache)?;
    let mut command = Command::new(super::terminal_runtime::ensure_node(cache)?);
    command
        .arg(bundle)
        .arg("--nexus")
        .arg(std::env::current_exe()?)
        .current_dir(working_directory);
    if let Some(config) = explicit_config {
        command.arg("--config").arg(config.canonicalize()?);
    }
    Ok(command)
}
