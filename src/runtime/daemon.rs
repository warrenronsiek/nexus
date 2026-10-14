// @feature runtime
// @feature agent-memory
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @entrypoint serve
// @boundary dynamic-json
use crate::config::LoadedConfig;
use crate::coordination::api::{ServiceRequest, ServiceResponse};
use crate::coordination::domain::HookResponse;
use crate::coordination::NexusService;
use crate::runtime::protocol::WireRequest;
use crate::runtime::{bounded_io, dispatch};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::Value;
use std::fs::OpenOptions;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;

const MAX_IN_FLIGHT_LOCAL_CONNECTIONS: usize = 64;
const LOCAL_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn serve(loaded: LoadedConfig) -> Result<()> {
    let runtime = start_runtime(loaded).await?;
    let _memory_scheduler = dispatch::MemoryConsolidationScheduler::start(runtime.service.clone());
    let model_work = dispatch::ModelWorkScheduler::start(runtime.service.clone());
    run_event_loop(
        &runtime.listener,
        runtime.service.clone(),
        model_work,
        runtime.reconcile_seconds,
    )
    .await
}

struct DaemonRuntime {
    socket_path: std::path::PathBuf,
    _lock: std::fs::File,
    listener: UnixListener,
    service: Arc<NexusService>,
    reconcile_seconds: u64,
}

impl Drop for DaemonRuntime {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn start_runtime(loaded: LoadedConfig) -> Result<DaemonRuntime> {
    let socket_path = loaded.config.runtime.socket_path.clone();
    let lock_path = loaded.config.runtime.lock_path.clone();
    let reconcile_seconds = loaded.config.coordination.reconcile_seconds;
    prepare_runtime_paths(&socket_path, &lock_path).await?;
    let lock = acquire_daemon_lock(&lock_path)?;
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;
    let service = Arc::new(NexusService::new(loaded)?);
    Ok(DaemonRuntime {
        socket_path,
        _lock: lock,
        listener,
        service,
        reconcile_seconds,
    })
}

async fn prepare_runtime_paths(socket_path: &Path, lock_path: &Path) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Some(parent) = lock_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

fn acquire_daemon_lock(lock_path: &Path) -> Result<std::fs::File> {
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .with_context(|| format!("another Nexus daemon owns {}", lock_path.display()))?;
    Ok(lock)
}

async fn run_event_loop(
    listener: &UnixListener,
    service: Arc<NexusService>,
    model_work: dispatch::ModelWorkScheduler,
    reconcile_seconds: u64,
) -> Result<()> {
    let mut reconcile = reconciliation_interval(reconcile_seconds);
    let connection_slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT_LOCAL_CONNECTIONS));
    let reconciliation_scheduler = ReconciliationScheduler::new();

    loop {
        tokio::select! {
            incoming = listener.accept() => {
                let (stream, _) = incoming?;
                let service = service.clone();
                let model_work = model_work.clone();
                spawn_admitted_connection(connection_slots.clone(), async move {
                    let _ = handle_connection(stream, service, model_work).await;
                });
            }
            _ = reconcile.tick() => {
                let service = service.clone();
                reconciliation_scheduler.schedule(move || {
                    let _ = service.reconcile_all();
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ReconciliationScheduler {
    slot: Arc<Semaphore>,
}

impl ReconciliationScheduler {
    fn new() -> Self {
        Self {
            slot: Arc::new(Semaphore::new(1)),
        }
    }

    fn schedule(&self, work: impl FnOnce() + Send + 'static) -> bool {
        let Ok(permit) = self.slot.clone().try_acquire_owned() else {
            return false;
        };
        drop(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work();
        }));
        true
    }
}

fn spawn_admitted_connection<F>(slots: Arc<Semaphore>, task: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    let Ok(permit) = slots.try_acquire_owned() else {
        return false;
    };
    tokio::spawn(async move {
        task.await;
        drop(permit);
    });
    true
}

fn reconciliation_interval(seconds: u64) -> tokio::time::Interval {
    let period = Duration::from_secs(seconds);
    let start = tokio::time::Instant::now() + period;
    let mut interval = tokio::time::interval_at(start, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn handle_connection(
    stream: UnixStream,
    service: Arc<NexusService>,
    model_work: dispatch::ModelWorkScheduler,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    while let Some(frame) = read_request_frame(&mut reader).await? {
        let response = match serde_json::from_slice::<WireRequest>(&frame) {
            Ok(request) => match ServiceRequest::decode(&request.method, request.params) {
                Ok(request) => dispatch::handle(service.clone(), &model_work, request, None).await,
                Err(error) if ServiceRequest::is_lifecycle_method(&request.method) => {
                    ServiceResponse::Hook(HookResponse::fail_open(error))
                }
                Err(error) => ServiceResponse::error(error),
            },
            Err(error) => ServiceResponse::error(format!("invalid request: {error}")),
        };
        writer
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
    }
    Ok(())
}

async fn read_request_frame<R>(reader: &mut R) -> Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    read_request_frame_with_timeout(reader, LOCAL_REQUEST_READ_TIMEOUT).await
}

async fn read_request_frame_with_timeout<R>(
    reader: &mut R,
    timeout: Duration,
) -> Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    tokio::time::timeout(timeout, bounded_io::read_json_line(reader))
        .await
        .context("local daemon request read timed out")?
        .context("read local daemon request")
}

pub async fn request(socket_path: &Path, request: &ServiceRequest) -> Result<Value> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect {}", socket_path.display()))?;
    let request = WireRequest::from_service(request, None)?;
    stream
        .write_all(serde_json::to_string(&request)?.as_bytes())
        .await?;
    stream.write_all(b"\n").await?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await?;
    serde_json::from_str(&line).context("decode daemon response")
}

pub async fn ensure_and_request(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    request_value: &ServiceRequest,
) -> Result<Value> {
    if let Ok(response) = request(&loaded.config.runtime.socket_path, request_value).await {
        return Ok(response);
    }

    let executable = std::env::current_exe()?;
    let mut command = tokio::process::Command::new(executable);
    if let Some(path) = explicit_config {
        command.arg("--config").arg(path);
    }
    command
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().context("start Nexus daemon")?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(response) = request(&loaded.config.runtime.socket_path, request_value).await {
            return Ok(response);
        }
    }
    anyhow::bail!(
        "Nexus daemon did not become ready at {}",
        loaded.config.runtime.socket_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::{oneshot, Semaphore};

    #[tokio::test]
    async fn oversized_nonterminated_local_request_is_rejected_at_the_sentinel() {
        let input = vec![b'x'; crate::runtime::bounded_io::MAX_JSON_FRAME_BYTES + 2];
        let mut reader = BufReader::new(std::io::Cursor::new(input));

        let error = read_request_frame_with_timeout(&mut reader, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn silent_local_peer_is_disconnected_after_the_read_deadline() {
        let (_writer, reader) = tokio::io::duplex(64);
        let mut reader = BufReader::new(reader);

        let error = read_request_frame_with_timeout(&mut reader, Duration::from_millis(20))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("timed out"), "{error:#}");
    }

    #[tokio::test]
    async fn local_connection_permit_is_owned_until_its_task_exits() {
        let slots = Arc::new(Semaphore::new(1));
        let (release, released) = oneshot::channel();

        assert!(spawn_admitted_connection(slots.clone(), async move {
            let _ = released.await;
        }));
        assert!(!spawn_admitted_connection(slots.clone(), async {}));

        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while slots.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(slots.available_permits(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconciliation_scheduler_skips_overlapping_work_without_blocking_the_runtime() {
        let scheduler = ReconciliationScheduler::new();
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        assert!(scheduler.schedule(move || {
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        }));
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the first reconciliation worker must start");

        let overlapping_started = Arc::new(AtomicBool::new(false));
        let overlapping_started_in_worker = overlapping_started.clone();
        let scheduled_at = std::time::Instant::now();
        assert!(!scheduler.schedule(move || {
            overlapping_started_in_worker.store(true, Ordering::SeqCst);
        }));
        assert!(scheduled_at.elapsed() < Duration::from_millis(100));
        assert!(!overlapping_started.load(Ordering::SeqCst));

        release_sender.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !scheduler.schedule(|| {}) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reconciliation capacity must return when the worker exits");
    }
}
