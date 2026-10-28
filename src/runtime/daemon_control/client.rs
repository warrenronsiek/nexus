// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary local-daemon-update-handshake: only typed owner-verified replies identify a managed daemon
use super::{ControlRequest, ControlResponse, DaemonSnapshot, METHOD};
use crate::{
    runtime::{bounded_io, protocol::WireRequest},
    LoadedConfig,
};
use anyhow::{bail, ensure, Context, Result};
use fs2::FileExt;
use std::path::Path;
use std::time::Duration;
use tokio::{io::BufReader, net::UnixStream};

async fn connect(path: &Path) -> Result<Option<UnixStream>> {
    match UnixStream::connect(path).await {
        Ok(stream) => {
            ensure!(
                stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
                "daemon control peer is not owned by the current user"
            );
            Ok(Some(stream))
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error).context("connect to managed daemon"),
    }
}
async fn exchange(
    loaded: &LoadedConfig,
    request: ControlRequest,
) -> Result<Option<ControlResponse>> {
    let Some(mut stream) = connect(&loaded.config.runtime.socket_path).await? else {
        return Ok(None);
    };
    let wire = WireRequest {
        method: METHOD.into(),
        params: serde_json::to_value(request)?,
        project_id: None,
        client_evidence: None,
    };
    super::super::daemon::write_json_line(&mut stream, &wire).await?;
    decode_response(stream).await.map(Some)
}
async fn decode_response(stream: UnixStream) -> Result<ControlResponse> {
    let bytes = bounded_io::read_json_line(&mut BufReader::new(stream))
        .await?
        .context("daemon control closed without a reply")?;
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("running daemon has no verified update control"))
}
async fn request(
    loaded: &LoadedConfig,
    request: ControlRequest,
) -> Result<Option<ControlResponse>> {
    tokio::time::timeout(Duration::from_secs(5), exchange(loaded, request))
        .await
        .context("daemon update control timed out")?
}

pub async fn inspect(loaded: &LoadedConfig) -> Result<Option<DaemonSnapshot>> {
    match request(loaded, ControlRequest::Inspect).await? {
        None => Ok(None),
        Some(ControlResponse::Snapshot { snapshot }) => {
            snapshot.loaded()?;
            ensure!(
                snapshot.config.runtime.socket_path == loaded.config.runtime.socket_path,
                "daemon snapshot socket mismatch"
            );
            Ok(Some(*snapshot))
        }
        _ => bail!("running daemon did not provide an update snapshot"),
    }
}
pub async fn stop(loaded: &LoadedConfig, snapshot: &DaemonSnapshot) -> Result<()> {
    match request(
        loaded,
        ControlRequest::Stop {
            instance_id: snapshot.instance_id.clone(),
        },
    )
    .await?
    {
        Some(ControlResponse::Stopping { instance_id }) if instance_id == snapshot.instance_id => {}
        None => return Ok(()),
        _ => bail!("daemon instance did not accept the stop request"),
    }
    let mut waiting = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(30),
        Duration::from_secs(30),
    );
    let drained = wait_stopped(snapshot);
    tokio::pin!(drained);
    loop {
        tokio::select! {
            result=&mut drained=>return result,
            _=waiting.tick()=>eprintln!("Nexus is finishing admitted work before updating; waiting for the daemon to release its database lock."),
        }
    }
}
async fn wait_stopped(snapshot: &DaemonSnapshot) -> Result<()> {
    loop {
        if !snapshot.config.runtime.socket_path.exists()
            && lock_available(&snapshot.config.runtime.lock_path)?
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn lock_available(path: &Path) -> Result<bool> {
    let file = match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    match file.try_lock_exclusive() {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error.into()),
    }
}
