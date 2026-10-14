// @feature runtime
// @spec docs/features/runtime.md
// @boundary bearer-secret-file
use crate::secret_file::{validate_bearer_secret, OpenedSecretFile};
use anyhow::{bail, Context, Result};
use axum::http::{header, HeaderMap};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Clone)]
pub struct ServerSecret {
    hash: [u8; 32],
}

impl ServerSecret {
    pub fn new(secret: impl AsRef<str>) -> Result<Self> {
        let secret = secret.as_ref().trim();
        validate_bearer_secret(secret)?;
        Ok(Self {
            hash: *blake3::hash(secret.as_bytes()).as_bytes(),
        })
    }

    pub(super) fn permits(&self, headers: &HeaderMap) -> bool {
        let Some(secret) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        else {
            return false;
        };
        let supplied = blake3::hash(secret.as_bytes());
        bool::from(self.hash.ct_eq(supplied.as_bytes()))
    }
}

pub struct LoadedServerSecret {
    verifier: ServerSecret,
    generated: bool,
}

impl LoadedServerSecret {
    pub fn verifier(&self) -> ServerSecret {
        self.verifier.clone()
    }

    pub fn was_generated(&self) -> bool {
        self.generated
    }
}

pub fn load_or_generate_secret(
    explicit: Option<&str>,
    secret_file: Option<&Path>,
) -> Result<LoadedServerSecret> {
    let explicit = explicit.map(str::trim).filter(|secret| !secret.is_empty());
    if let Some(explicit) = explicit {
        validate_bearer_secret(explicit)?;
    }
    let stored = secret_file
        .map(read_secret_file_if_exists)
        .transpose()?
        .flatten();
    let (secret, generated) = match explicit {
        Some(explicit) => (
            load_explicit_secret(explicit, stored.as_deref(), secret_file)?,
            false,
        ),
        None => load_managed_secret(stored, secret_file)?,
    };
    let verifier = ServerSecret::new(&secret)?;
    Ok(LoadedServerSecret {
        verifier,
        generated,
    })
}

fn load_explicit_secret(
    explicit: &str,
    stored: Option<&str>,
    secret_file: Option<&Path>,
) -> Result<String> {
    if let Some(stored) = stored {
        if stored != explicit {
            bail!("explicit Nexus server secret does not match the existing secret file");
        }
        return Ok(explicit.to_owned());
    }
    let Some(path) = secret_file else {
        return Ok(explicit.to_owned());
    };
    match publish_secret_file(path, explicit)? {
        PublishedSecret::Created => Ok(explicit.to_owned()),
        PublishedSecret::Existing(stored) if stored == explicit => Ok(stored),
        PublishedSecret::Existing(_) => {
            bail!("explicit Nexus server secret does not match the existing secret file")
        }
    }
}

fn load_managed_secret(
    stored: Option<String>,
    secret_file: Option<&Path>,
) -> Result<(String, bool)> {
    if let Some(stored) = stored {
        return Ok((stored, false));
    }
    let path = secret_file.context(
        "Nexus server requires an explicit secret or a secret file for generated credentials",
    )?;
    let candidate = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    Ok(match publish_secret_file(path, &candidate)? {
        PublishedSecret::Created => (candidate, true),
        PublishedSecret::Existing(stored) => (stored, false),
    })
}

fn read_secret_file_if_exists(path: &Path) -> Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "Nexus server secret file {} must not be a symbolic link",
                path.display()
            )
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect Nexus server secret file {}", path.display()))
        }
    }
    let file = match OpenedSecretFile::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open Nexus server secret file {}", path.display()))
        }
    };
    let secret = file
        .read_utf8()
        .with_context(|| format!("read Nexus server secret file {}", path.display()))?;
    Ok(Some(secret.trim().to_owned()))
}

enum PublishedSecret {
    Created,
    Existing(String),
}

fn publish_secret_file(path: &Path, secret: &str) -> Result<PublishedSecret> {
    let parent = secret_parent_directory(path)?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create secret directory {}", parent.display()))?;
    let temporary = parent.join(format!(".nexus-secret-{}.tmp", Uuid::new_v4().simple()));
    let result = stage_secret_file(&temporary, secret)
        .and_then(|()| publish_staged_secret(&temporary, path))
        .and_then(|published| {
            if matches!(&published, PublishedSecret::Created) {
                sync_secret_directory(parent)?;
            }
            Ok(published)
        });
    let _ = std::fs::remove_file(&temporary);
    result
}

fn secret_parent_directory(path: &Path) -> Result<&Path> {
    let parent = path
        .parent()
        .context("Nexus server secret file must have a parent directory")?;
    if parent.as_os_str().is_empty() {
        Ok(Path::new("."))
    } else {
        Ok(parent)
    }
}

fn sync_secret_directory(parent: &Path) -> Result<()> {
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(parent)
        .with_context(|| format!("open secret directory {} for sync", parent.display()))?;
    directory
        .sync_all()
        .with_context(|| format!("sync secret directory {}", parent.display()))
}

fn stage_secret_file(temporary: &Path, secret: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(temporary)
        .with_context(|| format!("create temporary secret file {}", temporary.display()))?;
    file.write_all(secret.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn publish_staged_secret(temporary: &Path, path: &Path) -> Result<PublishedSecret> {
    match std::fs::hard_link(temporary, path) {
        Ok(()) => Ok(PublishedSecret::Created),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let stored = read_secret_file_if_exists(path)?.with_context(|| {
                format!("Nexus server secret disappeared at {}", path.display())
            })?;
            Ok(PublishedSecret::Existing(stored))
        }
        Err(error) => {
            Err(error).with_context(|| format!("publish Nexus server secret {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_relative_secret_file_uses_the_current_directory() {
        assert_eq!(
            secret_parent_directory(Path::new("service.secret")).unwrap(),
            Path::new(".")
        );
    }

    #[test]
    fn bearer_secrets_must_be_visible_ascii_before_persistence() {
        let valid = "AbCdEf0123456789-._~+/=AbCdEf0123456789";
        assert!(ServerSecret::new(valid).is_ok());

        let temporary = tempfile::tempdir().unwrap();
        let canonical_path = temporary.path().join("canonical.secret");
        load_or_generate_secret(Some(&format!("  {valid}\r\n")), Some(&canonical_path))
            .expect("source-boundary whitespace must be canonicalized");
        assert_eq!(std::fs::read_to_string(&canonical_path).unwrap(), valid);

        for (name, invalid) in [
            ("space", format!("{} {}", "a".repeat(16), "b".repeat(16))),
            ("newline", format!("{}\n{}", "a".repeat(16), "b".repeat(16))),
            ("non-ascii", "é".repeat(32)),
        ] {
            let path = temporary.path().join(format!("{name}.secret"));
            let error = load_or_generate_secret(Some(&invalid), Some(&path))
                .err()
                .expect("an HTTP-unsafe bearer secret must be rejected");
            assert!(
                error.to_string().contains("visible ASCII"),
                "unexpected error for {name}: {error:#}"
            );
            assert!(!path.exists(), "invalid secret {name} was persisted");
        }
    }
}
