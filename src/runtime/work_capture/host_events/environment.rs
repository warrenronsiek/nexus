// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use super::decode_input;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentHandoff {
    Exported,
    NotRequested,
    Unavailable,
}

/// Append the exact native identity only at the host's environment-export boundary.
pub fn export_host_environment(
    host: &str,
    bytes: &[u8],
    destination: Option<&Path>,
) -> EnvironmentHandoff {
    try_export(host, bytes, destination).unwrap_or(EnvironmentHandoff::Unavailable)
}

fn try_export(host: &str, bytes: &[u8], destination: Option<&Path>) -> Result<EnvironmentHandoff> {
    let Some(destination) = destination.filter(|_| host == "claude") else {
        return Ok(EnvironmentHandoff::NotRequested);
    };
    let input = decode_input(host, bytes)?;
    if input.hook_event_name != "SessionStart" {
        return Ok(EnvironmentHandoff::NotRequested);
    }
    append_identity(destination, &input.session_id)?;
    Ok(EnvironmentHandoff::Exported)
}

fn append_identity(destination: &Path, session_id: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(destination)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0,
        "host environment must be an owned regular file without shared write access"
    );
    // Keep turn/work-parent context live in the exact host bridge; exporting a
    // startup snapshot would attach every later command to an obsolete turn.
    let quoted = session_id.replace('\'', "'\\''");
    file.write_all(
        format!("\nexport NEXUS_AGENT='claude'\nexport NEXUS_HOST_SESSION_ID='{quoted}'\n")
            .as_bytes(),
    )?;
    Ok(())
}
