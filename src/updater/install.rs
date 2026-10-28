// @feature installation
// @spec docs/features/installation.md
// @boundary release-executable-install: validate staged executable bytes and identity before atomic replacement of an owned installation.
use super::{release::Artifact, BuildIdentity};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::{NamedTempFile, TempPath};

pub(super) struct PreparedBinary {
    target: PathBuf,
    candidate: TempPath,
    lock: File,
}
pub(super) struct Replacement {
    target: PathBuf,
    backup: Option<PathBuf>,
    _lock: File,
}
impl PreparedBinary {
    pub(super) fn verify_path(
        target: &Path,
        source: &Path,
        identity: &BuildIdentity,
        artifact: &Artifact,
    ) -> Result<Self> {
        let lock = installation_lock(target)?;
        artifact.verify(source)?;
        let candidate = stage_candidate(target, source, artifact)?;
        let mut command = Command::new(&candidate);
        command.arg("--version").stdin(Stdio::null());
        let output = crate::git_process::bounded_output(command, Duration::from_secs(5), 4096)
            .context("verify candidate executable identity")?;
        ensure!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim()
                    == format!("nexus {}", identity.display()),
            "candidate executable does not report the expected release identity"
        );
        Ok(Self {
            target: target.to_owned(),
            candidate,
            lock,
        })
    }
    pub(super) fn activate(self) -> Result<Replacement> {
        let Self {
            target,
            candidate,
            lock,
        } = self;
        let parent = target
            .parent()
            .context("installation directory unavailable")?;
        let backup = backup_executable(parent, &target)?;
        if let Err(error) = candidate.persist(&target) {
            let _ = fs::remove_file(&backup);
            return Err(error.error).context("atomically replace the Nexus executable");
        }
        let replacement = Replacement {
            target,
            backup: Some(backup),
            _lock: lock,
        };
        if let Err(error) = sync_parent(&replacement.target) {
            replacement
                .rollback()
                .context("restore executable after installation sync failed")?;
            return Err(error);
        }
        Ok(replacement)
    }
}
impl Replacement {
    pub(super) fn rollback(mut self) -> Result<()> {
        if let Some(backup) = self.backup.take() {
            fs::rename(&backup, &self.target).with_context(|| {
                format!(
                    "restore previous executable; recovery copy retained at {}",
                    backup.display()
                )
            })?;
            sync_parent(&self.target)?;
        }
        Ok(())
    }
    pub(super) fn commit(mut self) -> Result<()> {
        if let Some(backup) = self.backup.take() {
            fs::remove_file(&backup).with_context(|| {
                format!(
                    "release installed; remove recovery copy at {}",
                    backup.display()
                )
            })?;
            sync_parent(&self.target)?;
        }
        Ok(())
    }
}
impl Drop for Replacement {
    fn drop(&mut self) {
        if let Some(backup) = self.backup.take() {
            let _ = fs::rename(backup, &self.target);
            let _ = sync_parent(&self.target);
        }
    }
}
fn installation_lock(target: &Path) -> Result<File> {
    let parent = target
        .parent()
        .context("installed executable has no parent directory")?;
    validate_owned(target, 0o6022)?;
    validate_directory(parent)?;
    open_lock(&parent.join(".nexus-update.lock"))
}
fn validate_owned(path: &Path, forbidden: u32) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & forbidden == 0,
        "update requires an owned regular file without unsafe permissions"
    );
    Ok(())
}
fn validate_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0,
        "update requires an owned installation directory without shared write permissions"
    );
    Ok(())
}
fn open_lock(path: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = lock.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "update lock must be a private owned regular file"
    );
    lock.try_lock_exclusive()
        .context("another Nexus update owns this installation")?;
    Ok(lock)
}
fn stage_candidate(target: &Path, source: &Path, artifact: &Artifact) -> Result<TempPath> {
    let candidate = tempfile::Builder::new()
        .prefix(".nexus-candidate-")
        .tempfile_in(target.parent().unwrap())?;
    fs::copy(source, candidate.path())?;
    artifact.verify(candidate.path())?;
    finish_candidate(&candidate)?;
    // Linux refuses to execute a file while any writable handle remains open.
    Ok(candidate.into_temp_path())
}
fn finish_candidate(candidate: &NamedTempFile) -> Result<()> {
    fs::set_permissions(candidate.path(), fs::Permissions::from_mode(0o755))?;
    candidate.as_file().sync_all()?;
    Ok(())
}
fn backup_executable(parent: &Path, target: &Path) -> Result<PathBuf> {
    let backup = tempfile::Builder::new()
        .prefix(".nexus-rollback-")
        .tempfile_in(parent)?;
    fs::copy(target, backup.path())?;
    backup.as_file().sync_all()?;
    let (file, path) = backup.keep()?;
    drop(file);
    Ok(path)
}
fn sync_parent(path: &Path) -> Result<()> {
    File::open(
        path.parent()
            .context("installation directory unavailable")?,
    )?
    .sync_all()
    .context("sync installation directory")
}

#[cfg(test)]
mod tests {
    use super::super::{release::Artifact, BuildIdentity};
    use super::*;
    use sha2::{Digest, Sha256};

    fn candidate(directory: &std::path::Path) -> (std::path::PathBuf, BuildIdentity, Artifact) {
        let identity = BuildIdentity {
            version: "0.2.0".into(),
            revision: "a".repeat(40),
            target: "aarch64-apple-darwin".into(),
            dirty: false,
        };
        let bytes =
            format!("#!/bin/sh\nprintf '%s\\n' 'nexus {}'\n", identity.display()).into_bytes();
        let path = directory.join("candidate");
        std::fs::write(&path, &bytes).unwrap();
        let artifact = Artifact {
            target: identity.target.clone(),
            name: format!("nexus-{}", identity.target),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            size: bytes.len() as u64,
        };
        (path, identity, artifact)
    }
    #[test]
    fn verified_replacement_is_atomic_and_rollback_restores_the_previous_executable() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("nexus");
        std::fs::write(&executable, b"old executable").unwrap();
        let database = directory.path().join("nexus.db");
        std::fs::write(&database, b"durable data").unwrap();
        let (source, identity, artifact) = candidate(directory.path());
        let prepared =
            PreparedBinary::verify_path(&executable, &source, &identity, &artifact).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"old executable");
        let replacement = prepared.activate().unwrap();
        assert_eq!(
            std::fs::read(&executable).unwrap(),
            std::fs::read(&source).unwrap()
        );
        replacement.rollback().unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"old executable");
        assert_eq!(std::fs::read(database).unwrap(), b"durable data");
        let replacement = PreparedBinary::verify_path(&executable, &source, &identity, &artifact)
            .unwrap()
            .activate()
            .unwrap();
        replacement.commit().unwrap();
        assert_eq!(
            std::fs::read(&executable).unwrap(),
            std::fs::read(source).unwrap()
        );
    }
    #[test]
    fn damaged_or_wrong_version_candidates_leave_the_live_installation_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("nexus");
        std::fs::write(&executable, b"old executable").unwrap();
        let (source, mut identity, artifact) = candidate(directory.path());
        identity.revision = "b".repeat(40);
        assert!(PreparedBinary::verify_path(&executable, &source, &identity, &artifact).is_err());
        std::fs::write(&source, b"damaged").unwrap();
        assert!(PreparedBinary::verify_path(&executable, &source, &identity, &artifact).is_err());
        assert_eq!(std::fs::read(executable).unwrap(), b"old executable");
    }
}
