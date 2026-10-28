// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary private-daemon-resume-file: private owned snapshots preserve resolved configuration without exposing it in argv
use super::{decode_snapshot, inspect, DaemonSnapshot};
use anyhow::{ensure, Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

pub async fn resume(path: &Path) -> Result<()> {
    let snapshot = decode_snapshot(path)?;
    std::fs::remove_file(path).context("consume managed daemon snapshot")?;
    std::env::set_current_dir(&snapshot.directory)
        .context("restore managed daemon working directory")?;
    super::super::daemon::serve(snapshot.loaded()?).await
}

pub async fn restart(snapshot: &DaemonSnapshot, executable: &Path) -> Result<()> {
    let directory = crate::private_state::directory_at;
    let parent = snapshot
        .config
        .runtime
        .lock_path
        .parent()
        .context("managed daemon lock has no parent")?
        .join("updates");
    directory(&parent)?;
    let path = parent.join(format!("daemon-{}.json", uuid::Uuid::new_v4()));
    let mut resume = snapshot.clone();
    resume.executable = executable.canonicalize()?;
    crate::private_state::write(&path, &serde_json::to_vec(&resume)?)?;
    let result = start(&resume, &path).await;
    let _ = std::fs::remove_file(&path);
    result
}
async fn start(snapshot: &DaemonSnapshot, path: &Path) -> Result<()> {
    let mut child = tokio::process::Command::new(&snapshot.executable)
        .arg("update-resume")
        .arg("--snapshot")
        .arg(path)
        .current_dir(&snapshot.directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("restart managed Nexus daemon")?;
    let loaded = snapshot.loaded()?;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            ensure!(
                child.try_wait()?.is_none(),
                "managed daemon exited before readiness"
            );
            if let Ok(Some(active)) = inspect(&loaded).await {
                ensure!(
                    active.instance_id != snapshot.instance_id,
                    "previous daemon is still running"
                );
                ensure!(
                    active.config_hash == snapshot.config_hash
                        && active.executable == snapshot.executable,
                    "another daemon owns the selected configuration"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("managed daemon restart did not become ready")?
}
