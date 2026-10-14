// @feature configuration
// @feature runtime
// @spec docs/features/configuration.md
// @spec docs/features/runtime.md
// @boundary secret-file
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Error, ErrorKind, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

pub(crate) const MAX_SECRET_FILE_BYTES: usize = 4 * 1024;
const MIN_BEARER_SECRET_BYTES: usize = 32;
const MAX_SECRET_SOURCE_BYTES: usize = MAX_SECRET_FILE_BYTES + 2;

pub(crate) fn validate_bearer_secret(secret: &str) -> std::io::Result<()> {
    if secret.len() < MIN_BEARER_SECRET_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Nexus bearer secret must contain at least {MIN_BEARER_SECRET_BYTES} characters"
            ),
        ));
    }
    if secret.len() > MAX_SECRET_FILE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Nexus bearer secret must not exceed {MAX_SECRET_FILE_BYTES} characters"),
        ));
    }
    if !secret.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Nexus bearer secret must contain only visible ASCII characters without whitespace",
        ));
    }
    Ok(())
}

pub(crate) struct OpenedSecretFile {
    file: File,
    metadata: Metadata,
}

impl OpenedSecretFile {
    pub(crate) fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| symbolic_link_error(path, error))?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Nexus secret file {} must refer to a regular file",
                    path.display()
                ),
            ));
        }
        validate_secret_file_metadata(&metadata, path)?;
        Ok(Self { file, metadata })
    }

    pub(crate) fn read_utf8(mut self) -> std::io::Result<String> {
        if self.metadata.len() > MAX_SECRET_SOURCE_BYTES as u64 {
            return Err(secret_too_large());
        }
        let mut bytes = Vec::with_capacity(self.metadata.len() as usize);
        self.file
            .by_ref()
            .take((MAX_SECRET_SOURCE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_SECRET_SOURCE_BYTES {
            return Err(secret_too_large());
        }
        String::from_utf8(bytes)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "secret file must contain UTF-8"))
    }
}

fn symbolic_link_error(path: &Path, error: Error) -> Error {
    if error.raw_os_error() == Some(libc::ELOOP) {
        Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Nexus secret file {} must not be a symbolic link",
                path.display()
            ),
        )
    } else {
        error
    }
}

fn validate_secret_file_metadata(metadata: &Metadata, path: &Path) -> std::io::Result<()> {
    let permissions = metadata.mode() & 0o777;
    if permissions & !0o600 != 0 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "Nexus secret file {} has permissions {permissions:03o}; permissions must be no broader than 600",
                path.display()
            ),
        ));
    }
    validate_secret_file_owner(path, metadata.uid(), effective_user_id())
}

fn validate_secret_file_owner(
    path: &Path,
    owner_uid: u32,
    effective_uid: u32,
) -> std::io::Result<()> {
    if owner_uid != effective_uid {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "Nexus secret file {} must be owned by the current user",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn effective_user_id() -> u32 {
    // SAFETY: `geteuid` accepts no arguments and has no caller-side safety preconditions.
    unsafe { libc::geteuid() }
}

fn secret_too_large() -> Error {
    Error::new(
        ErrorKind::InvalidData,
        format!("secret file exceeds {MAX_SECRET_SOURCE_BYTES} bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const TEST_SECRET: &str = "test-secret-that-is-at-least-32-bytes";

    fn write_secret(path: &Path, mode: u32) {
        std::fs::write(path, TEST_SECRET).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn secret_files_reject_permissions_visible_to_other_users() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("client.secret");
        write_secret(&path, 0o644);

        let error = OpenedSecretFile::open(&path)
            .err()
            .expect("an overexposed companion secret must be rejected");

        assert!(error.to_string().contains("permissions"), "{error}");
    }

    #[test]
    fn owner_read_write_secret_files_remain_valid() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("client.secret");
        write_secret(&path, 0o600);

        let secret = OpenedSecretFile::open(&path)
            .and_then(OpenedSecretFile::read_utf8)
            .expect("an owner-only companion secret must remain readable");

        assert_eq!(secret, TEST_SECRET);
    }

    #[test]
    fn secret_file_owner_must_match_the_effective_user() {
        let error = validate_secret_file_owner(Path::new("client.secret"), 41, 42)
            .expect_err("a secret owned by another user must be rejected");

        assert!(error.to_string().contains("owned by the current user"));
    }

    #[test]
    fn secret_files_keep_rejecting_symbolic_links_and_non_regular_files() {
        let temporary = tempfile::tempdir().unwrap();
        let target = temporary.path().join("target.secret");
        let link = temporary.path().join("client.secret");
        let directory = temporary.path().join("directory.secret");
        write_secret(&target, 0o600);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        std::fs::create_dir(&directory).unwrap();

        let error = OpenedSecretFile::open(&link)
            .err()
            .expect("a symbolic-link companion secret must be rejected");
        assert!(error.to_string().contains("symbolic link"), "{error}");
        let error = OpenedSecretFile::open(&directory)
            .err()
            .expect("a non-regular companion secret must be rejected");
        assert!(error.to_string().contains("regular file"), "{error}");
    }
}
