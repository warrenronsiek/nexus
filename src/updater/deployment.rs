// @feature installation
// @spec docs/features/installation.md
// @boundary deployment-command: an explicitly selected executable owns service identity, supervision, health, and recovery; requests and receipts use a versioned closed JSON protocol.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Operation<'a> {
    Inspect,
    Apply {
        archive: &'a Path,
        image_id: &'a str,
        revision: &'a str,
    },
    Recover {
        image_id: &'a str,
    },
}
#[derive(Serialize)]
struct Request<'a> {
    format_version: u32,
    endpoint: &'a str,
    #[serde(flatten)]
    operation: Operation<'a>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspection {
    format_version: u32,
    endpoint: String,
    target: String,
    image_id: String,
}

pub(super) struct Deployment {
    command: PathBuf,
    endpoint: String,
    previous: Inspection,
    archive: tempfile::NamedTempFile,
    image_id: String,
    revision: String,
}
impl Deployment {
    pub(super) async fn prepare(
        command: &Path,
        endpoint: &str,
        registry: &super::registry::Registry,
        release: &super::release::ReleaseManifest,
    ) -> Result<Self> {
        let command = command
            .canonicalize()
            .context("resolve service updater executable")?;
        let output = invoke(&command, endpoint, Operation::Inspect)?;
        let previous: Inspection =
            serde_json::from_slice(&output).context("decode service updater inspection")?;
        ensure!(
            previous.format_version == 1 && previous.endpoint == endpoint,
            "service updater must prove the exact configured endpoint"
        );
        ensure!(
            previous.image_id.starts_with("sha256:")
                && super::release::hex(&previous.image_id[7..], 64),
            "service updater must identify the previous immutable image"
        );
        let image = release.image(&previous.target)?;
        let archive = super::service::verified_archive(registry, release, image).await?;
        Ok(Self {
            command,
            endpoint: endpoint.into(),
            previous,
            archive,
            image_id: image.image_id.clone(),
            revision: release.revision.clone(),
        })
    }
    pub(super) fn apply(&self) -> Result<()> {
        invoke(
            &self.command,
            &self.endpoint,
            Operation::Apply {
                archive: self.archive.path(),
                image_id: &self.image_id,
                revision: &self.revision,
            },
        )?;
        Ok(())
    }
    pub(super) fn recover(&self) -> Result<()> {
        invoke(
            &self.command,
            &self.endpoint,
            Operation::Recover {
                image_id: &self.previous.image_id,
            },
        )?;
        Ok(())
    }
}
fn invoke(command: &Path, endpoint: &str, operation: Operation<'_>) -> Result<Vec<u8>> {
    // File-backed stdin avoids a pipe writer deadlock and keeps the bounded process runner authoritative.
    let mut input = tempfile::tempfile()?;
    serde_json::to_writer(
        &mut input,
        &Request {
            format_version: 1,
            endpoint,
            operation,
        },
    )?;
    input.write_all(b"\n")?;
    input.rewind()?;
    let mut process = Command::new(command);
    process.stdin(Stdio::from(input));
    let output = crate::git_process::bounded_output(process, Duration::from_secs(600), 64 * 1024)
        .context("run service updater")?;
    ensure!(
        output.status.success(),
        "service updater failed ({}); inspect deployment logs",
        output.status
    );
    Ok(output.stdout)
}

#[cfg(test)]
mod tests;
