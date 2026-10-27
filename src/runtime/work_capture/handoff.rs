// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use super::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Condvar, Mutex, OnceLock};

static ADMISSION_LOSSES: AtomicU64 = AtomicU64::new(0);
static WORKER_FAILURES: AtomicU64 = AtomicU64::new(0);
static PENDING_CALLBACKS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Durable,
    Pending,
    Rejected,
    Lost,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureReceipt {
    pub status: CaptureStatus,
    pub offered: usize,
    pub queued: usize,
    pub duplicates: usize,
    pub rejected: usize,
    pub lost: usize,
    pub loss_reasons: BTreeMap<&'static str, usize>,
    pub rejection_reasons: BTreeMap<&'static str, usize>,
    pub reason: Option<&'static str>,
}

impl CaptureReceipt {
    fn pending(offered: usize) -> Self {
        Self {
            status: CaptureStatus::Pending,
            offered,
            queued: 0,
            duplicates: 0,
            rejected: 0,
            lost: 0,
            loss_reasons: BTreeMap::new(),
            rejection_reasons: BTreeMap::new(),
            reason: None,
        }
    }
    pub(super) fn lost(offered: usize, reason: &'static str) -> Self {
        Self {
            status: CaptureStatus::Lost,
            offered,
            queued: 0,
            duplicates: 0,
            rejected: 0,
            lost: offered,
            loss_reasons: [(reason, offered)].into(),
            rejection_reasons: BTreeMap::new(),
            reason: Some(reason),
        }
    }
    pub(super) fn record_loss(&mut self, reason: &'static str) {
        self.lost += 1;
        self.status = CaptureStatus::Lost;
        self.reason = Some(reason);
        *self.loss_reasons.entry(reason).or_default() += 1;
    }
    pub(super) fn record_rejection(&mut self, reason: &'static str) {
        self.rejected += 1;
        if self.lost == 0 {
            self.status = CaptureStatus::Rejected;
            self.reason = Some(reason);
        }
        *self.rejection_reasons.entry(reason).or_default() += 1;
    }
}

#[derive(Clone)]
pub(crate) struct CaptureHandoff {
    queue: Arc<CaptureQueue>,
}

struct CaptureQueue {
    sender: mpsc::SyncSender<CaptureJob>,
    state: Arc<QueueState>,
    capacity: usize,
}
#[derive(Default)]
struct QueueState {
    occupied: AtomicUsize,
    losses: Mutex<BTreeMap<PathBuf, u64>>,
    completion: Mutex<()>,
    completed: Condvar,
}

impl QueueState {
    fn finish(&self) {
        let _guard = self
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.occupied.fetch_sub(1, Ordering::AcqRel);
        PENDING_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
        self.completed.notify_all();
    }
}
struct CaptureJob {
    loaded: Arc<LoadedConfig>,
    request: CaptureRequest,
    reply: CaptureReply,
}
enum CaptureRequest {
    Work(Box<WorkCommand>),
    AdapterRejected { host: String },
}

impl CaptureRequest {
    fn rejected_adapter(host: &str) -> Self {
        let host = if matches!(host, "claude" | "codex" | "pi") {
            host
        } else {
            "unknown"
        };
        Self::AdapterRejected { host: host.into() }
    }
    fn offered(&self) -> usize {
        match self {
            Self::Work(command) => offered(command),
            Self::AdapterRejected { .. } => 1,
        }
    }
    fn persist(self, loaded: &LoadedConfig) -> CaptureReceipt {
        match self {
            Self::Work(command) => persist_lifecycle(loaded, *command),
            Self::AdapterRejected { host } => rejected_transport(loaded, &host),
        }
    }
}

fn rejected_transport(loaded: &LoadedConfig, host: &str) -> CaptureReceipt {
    let Ok(path) = directory(loaded) else {
        return CaptureReceipt::lost(1, "directory_unavailable");
    };
    if !super::health::adapter_rejected(&path, host) {
        return CaptureReceipt::lost(1, "rejection_health_unavailable");
    }
    let mut receipt = CaptureReceipt::pending(1);
    receipt.record_rejection("invalid_host_event");
    receipt
}
enum CaptureReply {
    Async(tokio::sync::oneshot::Sender<CaptureReceipt>),
    Blocking(mpsc::Sender<CaptureReceipt>),
}

impl CaptureReply {
    fn send(self, receipt: CaptureReceipt) {
        match self {
            Self::Async(sender) => {
                let _ = sender.send(receipt);
            }
            Self::Blocking(sender) => {
                let _ = sender.send(receipt);
            }
        }
    }
}

impl Default for CaptureHandoff {
    fn default() -> Self {
        static SHARED: OnceLock<CaptureHandoff> = OnceLock::new();
        SHARED.get_or_init(|| Self::new(256)).clone()
    }
}

impl CaptureHandoff {
    pub(super) fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<CaptureJob>(capacity);
        let state = Arc::new(QueueState::default());
        let worker_state = state.clone();
        std::thread::Builder::new()
            .name("nexus-work-capture".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let offered = job.request.offered();
                    let receipt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        job.request.persist(&job.loaded)
                    }))
                    .unwrap_or_else(|_| {
                        WORKER_FAILURES.fetch_add(offered as u64, Ordering::Relaxed);
                        CaptureReceipt::lost(offered, "worker_failure")
                    });
                    let losses = std::mem::take(
                        &mut *worker_state
                            .losses
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    );
                    for (path, lost) in losses {
                        super::health::admission_loss(&path, lost);
                    }
                    job.reply.send(receipt);
                    worker_state.finish();
                }
            })
            .ok();
        Self {
            queue: Arc::new(CaptureQueue {
                sender,
                state,
                capacity,
            }),
        }
    }

    pub(super) fn drain_pending(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        let state = &self.queue.state;
        let mut guard = state
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.occupied.load(Ordering::Acquire) != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, _) = state
                .completed
                .wait_timeout(guard, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = next;
        }
        true
    }

    pub(crate) async fn capture(
        &self,
        loaded: Arc<LoadedConfig>,
        command: WorkCommand,
        budget: Duration,
    ) -> CaptureReceipt {
        self.capture_request(loaded, CaptureRequest::Work(Box::new(command)), budget)
            .await
    }

    pub(crate) async fn reject_adapter_async(
        &self,
        loaded: Arc<LoadedConfig>,
        host: &str,
        budget: Duration,
    ) -> CaptureReceipt {
        self.capture_request(loaded, CaptureRequest::rejected_adapter(host), budget)
            .await
    }

    async fn capture_request(
        &self,
        loaded: Arc<LoadedConfig>,
        request: CaptureRequest,
        budget: Duration,
    ) -> CaptureReceipt {
        let offered = request.offered();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if let Err(receipt) = self.offer(loaded, request, CaptureReply::Async(sender)) {
            return receipt;
        }
        match tokio::time::timeout(budget, receiver).await {
            Ok(Ok(receipt)) => receipt,
            Ok(Err(_)) => {
                WORKER_FAILURES.fetch_add(offered as u64, Ordering::Relaxed);
                CaptureReceipt::lost(offered, "worker_failure")
            }
            Err(_) => CaptureReceipt::pending(offered),
        }
    }

    pub(super) fn capture_blocking(
        &self,
        loaded: Arc<LoadedConfig>,
        command: WorkCommand,
        budget: Duration,
    ) -> CaptureReceipt {
        self.capture_blocking_request(loaded, CaptureRequest::Work(Box::new(command)), budget)
    }

    pub(super) fn reject_adapter(
        &self,
        loaded: Arc<LoadedConfig>,
        host: String,
        budget: Duration,
    ) -> CaptureReceipt {
        self.capture_blocking_request(loaded, CaptureRequest::rejected_adapter(&host), budget)
    }

    fn capture_blocking_request(
        &self,
        loaded: Arc<LoadedConfig>,
        request: CaptureRequest,
        budget: Duration,
    ) -> CaptureReceipt {
        let offered = request.offered();
        let (sender, receiver) = mpsc::channel();
        if let Err(receipt) = self.offer(loaded, request, CaptureReply::Blocking(sender)) {
            return receipt;
        }
        receiver
            .recv_timeout(budget)
            .unwrap_or_else(|error| match error {
                mpsc::RecvTimeoutError::Timeout => CaptureReceipt::pending(offered),
                mpsc::RecvTimeoutError::Disconnected => {
                    WORKER_FAILURES.fetch_add(offered as u64, Ordering::Relaxed);
                    CaptureReceipt::lost(offered, "worker_failure")
                }
            })
    }

    fn offer(
        &self,
        loaded: Arc<LoadedConfig>,
        request: CaptureRequest,
        reply: CaptureReply,
    ) -> std::result::Result<(), CaptureReceipt> {
        let offered = request.offered();
        let admitted =
            self.queue
                .state
                .occupied
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < self.queue.capacity).then_some(count + 1)
                });
        if admitted.is_err() {
            ADMISSION_LOSSES.fetch_add(offered as u64, Ordering::Relaxed);
            if let Ok(path) = directory(&loaded) {
                let mut losses = self
                    .queue
                    .state
                    .losses
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if losses.len() < 64 || losses.contains_key(&path) {
                    *losses.entry(path).or_default() += offered as u64;
                }
            }
            return Err(CaptureReceipt::lost(offered, "admission_capacity"));
        }
        PENDING_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        if self
            .queue
            .sender
            .try_send(CaptureJob {
                loaded,
                request,
                reply,
            })
            .is_err()
        {
            self.queue.state.occupied.fetch_sub(1, Ordering::AcqRel);
            PENDING_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
            WORKER_FAILURES.fetch_add(offered as u64, Ordering::Relaxed);
            return Err(CaptureReceipt::lost(offered, "worker_failure"));
        }
        Ok(())
    }
}

fn offered(command: &WorkCommand) -> usize {
    match &command.command {
        WorkAction::Record { observations } => observations.len(),
        _ => 0,
    }
}

pub(super) fn process_losses() -> (u64, u64, u64) {
    (
        ADMISSION_LOSSES.load(Ordering::Relaxed),
        WORKER_FAILURES.load(Ordering::Relaxed),
        PENDING_CALLBACKS.load(Ordering::Relaxed),
    )
}
