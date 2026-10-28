// @feature installation
// @spec docs/features/installation.md
// @feature runtime
// @spec docs/features/runtime.md
// @boundary private-state: restrictive owned files contain resolved deployment configuration and are never emitted in reports.
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub(crate) fn directory_at(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "private state directory must be private and owned by the current user"
    );
    Ok(())
}
pub(crate) fn temporary_in(directory: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    directory_at(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("private state has no parent")?;
    let file = temporary_in(parent, bytes)?;
    file.persist(path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub(crate) fn read(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= limit,
        "private state must be a bounded private owned file"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "private state exceeds its limit"
    );
    Ok(Some(bytes))
}

fn prepare_lock_directory(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let metadata = std::fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() },
        "private state must be a private owned directory"
    );
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(Into::into)
}

fn open_directory_lock(directory: &Path, name: &str) -> Result<File> {
    prepare_lock_directory(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(name))?;
    let metadata = lock.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "private state lock must be an owned private file"
    );
    Ok(lock)
}

pub(crate) fn lock_file(directory: &Path, name: &str) -> Result<File> {
    let lock = open_directory_lock(directory, name)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => return Err(error).context("private state is busy"),
        }
    }
    Ok(lock)
}
