// @feature installation
// @feature observability-ui
// @spec docs/features/installation.md
// @spec docs/features/observability-ui.md
// @entrypoint ensure_node
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const NODE_VERSION: &str = "24.21.0";

#[derive(Clone, Copy)]
struct NodeRelease<'a> {
    platform: &'a str,
    sha256: &'a str,
}

impl NodeRelease<'_> {
    fn directory(self) -> String {
        format!("node-v{NODE_VERSION}-{}", self.platform)
    }
}

pub(super) fn ensure_node(cache: &Path) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join("node"))
            .find(|path| supported_node(path))
    }) {
        return Ok(path);
    }
    install_runtime(cache, platform_release()?, download_runtime)
}

fn platform_release() -> Result<NodeRelease<'static>> {
    let (platform, sha256) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => (
            "linux-x64",
            "6e1db87ef58b8819e5d5402eff1536491b18edd8eb7bee5ef7897876e88dc5ff",
        ),
        ("linux", "aarch64") => (
            "linux-arm64",
            "724282c3b43aec998aa9527380465b45d229e021b58035f5f4f63095eabfe5d5",
        ),
        ("macos", "x86_64") => (
            "darwin-x64",
            "1462cb3b3046b815cf8ea436d3da450ec1a9f11dac7e5a46b0ada5305d7e8097",
        ),
        ("macos", "aarch64") => (
            "darwin-arm64",
            "bed7eea5325e1108f32ce5228ddd6a5f0f08a499ee42aa7442aea583702f6057",
        ),
        _ => bail!(
            "Automatic terminal runtime installation supports Linux and macOS on x64 or arm64"
        ),
    };
    Ok(NodeRelease { platform, sha256 })
}

fn supported_node(executable: &Path) -> bool {
    let Ok(mut child) = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if !matches!(child.wait_timeout(Duration::from_secs(2)), Ok(Some(_))) {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    child.wait_with_output().ok().is_some_and(|output| {
        output.status.success() && compatible_version(&String::from_utf8_lossy(&output.stdout))
    })
}

fn compatible_version(version: &str) -> bool {
    let Some(version) = version.trim().strip_prefix('v') else {
        return false;
    };
    let mut parts = version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    matches!((parts.next(), parts.next()), (Some(major), Some(minor)) if major > 22 || (major == 22 && minor >= 19))
}

fn install_runtime(
    cache: &Path,
    release: NodeRelease<'_>,
    download: impl FnOnce(&str, &Path) -> Result<()>,
) -> Result<PathBuf> {
    let _lock = lock_runtime_cache(cache)?;
    let directory = cache.join(release.directory());
    let node = directory.join("bin/node");
    if supported_node(&node) {
        return Ok(node);
    }
    eprintln!("Preparing the Nexus terminal runtime…");
    let pending = fetch_runtime(cache, release, download)?;
    publish_runtime(pending, &directory)?;
    Ok(node)
}

fn lock_runtime_cache(cache: &Path) -> Result<fs::File> {
    fs::create_dir_all(cache)?;
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(cache.join("runtime.lock"))?;
    lock.lock_exclusive()?;
    Ok(lock)
}

fn fetch_runtime(
    cache: &Path,
    release: NodeRelease<'_>,
    download: impl FnOnce(&str, &Path) -> Result<()>,
) -> Result<tempfile::TempDir> {
    let pending = tempfile::Builder::new()
        .prefix(".runtime-")
        .tempdir_in(cache)?;
    let archive = pending.path().join("runtime.tar.gz");
    let url = format!(
        "https://nodejs.org/dist/v{NODE_VERSION}/{}.tar.gz",
        release.directory()
    );
    download(&url, &archive).context("download Nexus terminal runtime")?;
    verify_archive(&archive, release.sha256)?;
    extract_runtime(&archive, pending.path(), release)?;
    Ok(pending)
}

fn verify_archive(archive: &Path, expected_sha256: &str) -> Result<()> {
    if format!("{:x}", Sha256::digest(fs::read(archive)?)) != expected_sha256 {
        bail!("Nexus terminal runtime checksum mismatch; the download was not installed");
    }
    Ok(())
}

fn extract_runtime(archive: &Path, pending: &Path, release: NodeRelease<'_>) -> Result<()> {
    let extracted = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(pending)
        .arg("--strip-components=1")
        .arg(format!("{}/bin/node", release.directory()))
        .arg(format!("{}/LICENSE", release.directory()))
        .status()
        .context("extract Nexus terminal runtime")?;
    if !extracted.success() || !supported_node(&pending.join("bin/node")) {
        bail!("The downloaded Nexus terminal runtime could not run on this machine");
    }
    fs::remove_file(archive)?;
    Ok(())
}

fn publish_runtime(pending: tempfile::TempDir, directory: &Path) -> Result<()> {
    if directory.exists() {
        fs::remove_dir_all(directory)?;
    }
    fs::rename(pending.path(), directory)?;
    Ok(())
}

fn download_runtime(url: &str, destination: &Path) -> Result<()> {
    let result = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "120",
            "--max-filesize",
            "134217728",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .status()
        .context("download terminal runtime with curl")?;
    if !result.success() {
        bail!("Terminal runtime download failed ({result}); rerun `nexus tui` to retry");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::cell::Cell;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn runtime_archive(directory: &std::path::Path) -> Vec<u8> {
        let binary = directory.join("node-v24.21.0-test/bin/node");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "#!/bin/sh\nprintf 'v24.21.0\\n'\n").unwrap();
        std::fs::write(
            directory.join("node-v24.21.0-test/LICENSE"),
            "runtime license fixture",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let archive = directory.join("runtime.tar.gz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(directory)
            .arg("node-v24.21.0-test")
            .status()
            .unwrap()
            .success());
        std::fs::read(archive).unwrap()
    }

    #[test]
    fn managed_runtime_is_verified_published_and_reused() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = runtime_archive(temporary.path());
        let digest = format!("{:x}", sha2::Sha256::digest(&archive));
        let release = NodeRelease {
            platform: "test",
            sha256: &digest,
        };
        let cache = temporary.path().join("cache");
        let downloads = Cell::new(0);
        let install = || {
            install_runtime(&cache, release, |url, target| {
                assert_eq!(
                    url,
                    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-test.tar.gz"
                );
                downloads.set(downloads.get() + 1);
                std::fs::write(target, &archive)?;
                Ok(())
            })
        };
        let node = install().unwrap();
        assert_eq!(
            Command::new(&node)
                .arg("--version")
                .output()
                .unwrap()
                .stdout,
            b"v24.21.0\n"
        );
        assert_eq!(install().unwrap(), node);
        assert_eq!(downloads.get(), 1);
        assert!(!cache.read_dir().unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".runtime-")));
    }

    #[test]
    fn damaged_runtime_download_is_rejected_without_publishing() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = runtime_archive(temporary.path());
        let release = NodeRelease {
            platform: "test",
            sha256: "incorrect checksum",
        };
        let cache = temporary.path().join("cache");
        let error = install_runtime(&cache, release, |_, target| {
            std::fs::write(target, &archive)?;
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"), "{error}");
        assert!(!cache.join("node-v24.21.0-test").exists());
        assert!(!cache.read_dir().unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".runtime-")));
    }
}
