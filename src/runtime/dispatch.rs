// @feature runtime
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @entrypoint handle
use crate::coordination::api::{MemoryConsolidationReport, ServiceRequest, ServiceResponse};
use crate::coordination::client_evidence::ClientFilesystemEvidence;
use crate::coordination::{
    NexusService, PapercutDispatch, ProjectBinding, SosDispatch, SosTriageJob,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Semaphore;

const MAX_QUEUED_MODEL_WORK: usize = 32;
const SOS_RECOVERY_INTERVAL: Duration = Duration::from_secs(60);

pub(super) struct MemoryConsolidationScheduler(tokio::task::JoinHandle<()>);

impl MemoryConsolidationScheduler {
    pub(super) fn start(service: Arc<NexusService>) -> Option<Self> {
        let seconds = service.memory_schedule_seconds()?;
        Some(Self(tokio::spawn(async move {
            let period = Duration::from_secs(seconds);
            let start = tokio::time::Instant::now() + period;
            let mut interval = tokio::time::interval_at(start, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let service = service.clone();
                drop(tokio::task::spawn_blocking(move || {
                    let _ = service.consolidate_memories();
                }));
            }
        })))
    }
}

impl Drop for MemoryConsolidationScheduler {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
pub(super) struct BlockingExecutionGate {
    slots: Arc<Semaphore>,
    queue_timeout: Duration,
}

#[derive(Debug, Error)]
pub(super) enum BlockingExecutionError {
    #[error("Nexus blocking execution capacity is temporarily exhausted")]
    AtCapacity,
    #[error("Nexus blocking worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
}

impl BlockingExecutionGate {
    pub(super) fn new(capacity: usize, queue_timeout: Duration) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            queue_timeout,
        }
    }

    pub(super) async fn run<F, T>(&self, work: F) -> Result<T, BlockingExecutionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = tokio::time::timeout(self.queue_timeout, self.slots.clone().acquire_owned())
            .await
            .map_err(|_| BlockingExecutionError::AtCapacity)?
            .map_err(|_| BlockingExecutionError::AtCapacity)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(BlockingExecutionError::from)
    }
}

#[derive(Clone)]
pub(super) struct ModelWorkScheduler {
    queue: ModelWorkQueue,
}

#[derive(Clone)]
struct ModelWorkQueue {
    sender: tokio::sync::mpsc::Sender<QueuedModelWork>,
    pending: Arc<Mutex<HashSet<ModelWorkKey>>>,
}

struct QueuedModelWork {
    key: ModelWorkKey,
    job: ModelWorkJob,
}

enum ModelWorkJob {
    Papercut(crate::coordination::PapercutReviewJob),
    Sos(SosTriageJob),
}

trait CompletedModelDispatch {
    fn into_parts(self) -> (ServiceResponse, Option<ModelWorkJob>);
}

impl CompletedModelDispatch for PapercutDispatch {
    fn into_parts(self) -> (ServiceResponse, Option<ModelWorkJob>) {
        (self.response, self.review_job.map(ModelWorkJob::Papercut))
    }
}

impl CompletedModelDispatch for SosDispatch {
    fn into_parts(self) -> (ServiceResponse, Option<ModelWorkJob>) {
        (self.response, self.triage_job.map(ModelWorkJob::Sos))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ModelWorkKey {
    Papercut {
        project_id: String,
        papercut_id: String,
    },
    Sos {
        project_id: String,
        report_id: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReviewScheduleOutcome {
    Scheduled,
    Duplicate,
    AtCapacity,
    Closed,
}

#[derive(Clone, Copy)]
enum WorkRecovery {
    None,
    Durable,
}

impl ModelWorkScheduler {
    pub(super) fn start(service: Arc<NexusService>) -> Self {
        let (sender, mut receiver) =
            tokio::sync::mpsc::channel::<QueuedModelWork>(MAX_QUEUED_MODEL_WORK);
        let queue = ModelWorkQueue::new(sender);
        let pending = queue.pending.clone();
        tokio::spawn(async move {
            let mut recovery = tokio::time::interval(SOS_RECOVERY_INTERVAL);
            recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let queued = tokio::select! {
                    queued = receiver.recv() => {
                        let Some(queued) = queued else { break };
                        vec![queued]
                    }
                    _ = recovery.tick() => {
                        load_recoverable_sos(service.clone())
                            .await
                            .into_iter()
                            .filter_map(|job| claim_work(&pending, ModelWorkJob::Sos(job)))
                            .collect()
                    }
                };
                for queued in queued {
                    let key = queued.key.clone();
                    run_model_work(service.clone(), queued.job).await;
                    if let Ok(mut active) = pending.lock() {
                        active.remove(&key);
                    }
                }
            }
        });
        Self { queue }
    }

    fn schedule_papercut(&self, job: crate::coordination::PapercutReviewJob) {
        self.schedule(
            ModelWorkJob::Papercut(job),
            "papercut review",
            WorkRecovery::None,
        );
    }

    fn schedule_sos(&self, job: SosTriageJob) {
        self.schedule(ModelWorkJob::Sos(job), "SOS triage", WorkRecovery::Durable);
    }

    fn schedule(&self, job: ModelWorkJob, label: &str, recovery: WorkRecovery) {
        match self.queue.schedule(job) {
            ReviewScheduleOutcome::Scheduled | ReviewScheduleOutcome::Duplicate => {}
            ReviewScheduleOutcome::AtCapacity => {
                log_unscheduled_work(label, "queue is full", recovery);
            }
            ReviewScheduleOutcome::Closed => {
                log_unscheduled_work(label, "worker is unavailable", recovery);
            }
        }
    }
}

fn log_unscheduled_work(label: &str, reason: &str, recovery: WorkRecovery) {
    match recovery {
        WorkRecovery::None => {
            eprintln!("Nexus {label} {reason}; the durable report was retained without review");
        }
        WorkRecovery::Durable => {
            eprintln!("Nexus {label} {reason}; durable work will be retried");
        }
    }
}

impl ModelWorkQueue {
    fn new(sender: tokio::sync::mpsc::Sender<QueuedModelWork>) -> Self {
        Self {
            sender,
            pending: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn schedule(&self, job: ModelWorkJob) -> ReviewScheduleOutcome {
        let key = work_key(&job);
        let Ok(mut pending) = self.pending.lock() else {
            return ReviewScheduleOutcome::Closed;
        };
        if pending.contains(&key) {
            return ReviewScheduleOutcome::Duplicate;
        }
        if pending.len() >= MAX_QUEUED_MODEL_WORK {
            return ReviewScheduleOutcome::AtCapacity;
        }
        pending.insert(key.clone());
        drop(pending);
        let queued = QueuedModelWork {
            key: key.clone(),
            job,
        };
        match self.sender.try_send(queued) {
            Ok(()) => ReviewScheduleOutcome::Scheduled,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.complete_key(&key);
                ReviewScheduleOutcome::AtCapacity
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.complete_key(&key);
                ReviewScheduleOutcome::Closed
            }
        }
    }

    #[cfg(test)]
    fn complete(&self, queued: &QueuedModelWork) {
        self.complete_key(&queued.key);
    }

    fn complete_key(&self, key: &ModelWorkKey) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(key);
        }
    }
}

fn claim_work(
    pending: &Mutex<HashSet<ModelWorkKey>>,
    job: ModelWorkJob,
) -> Option<QueuedModelWork> {
    let key = work_key(&job);
    let mut pending = pending.lock().ok()?;
    if pending.contains(&key) || pending.len() >= MAX_QUEUED_MODEL_WORK {
        return None;
    }
    pending.insert(key.clone());
    Some(QueuedModelWork { key, job })
}

fn work_key(job: &ModelWorkJob) -> ModelWorkKey {
    match job {
        ModelWorkJob::Papercut(job) => ModelWorkKey::Papercut {
            project_id: job.project_id.clone(),
            papercut_id: job.papercut_id.clone(),
        },
        ModelWorkJob::Sos(job) => ModelWorkKey::Sos {
            project_id: job.project_id.clone(),
            report_id: job.report_id.clone(),
        },
    }
}

async fn run_model_work(service: Arc<NexusService>, job: ModelWorkJob) {
    let _ = tokio::task::spawn_blocking(move || match job {
        ModelWorkJob::Papercut(job) => {
            if let Err(error) = service.review_papercut(&job) {
                let _ = service.record_papercut_review_failure(&job, &error);
            }
        }
        ModelWorkJob::Sos(job) => {
            if let Err(error) = service.triage_sos(&job) {
                let _ = service.record_sos_triage_failure(&job, &error);
            }
        }
    })
    .await;
}

async fn load_recoverable_sos(service: Arc<NexusService>) -> Vec<SosTriageJob> {
    tokio::task::spawn_blocking(move || service.ready_sos_triage_jobs())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

pub(super) async fn handle(
    service: Arc<NexusService>,
    model_work: &ModelWorkScheduler,
    request: ServiceRequest,
    project_id: Option<String>,
) -> ServiceResponse {
    handle_with_evidence(service, model_work, request, project_id, None, None).await
}

pub(super) async fn handle_bounded(
    service: Arc<NexusService>,
    model_work: &ModelWorkScheduler,
    execution_gate: &BlockingExecutionGate,
    request: ServiceRequest,
    project_id: Option<String>,
) -> ServiceResponse {
    handle_with_evidence(
        service,
        model_work,
        request,
        project_id,
        None,
        Some(execution_gate),
    )
    .await
}

pub(super) async fn handle_observed_bounded(
    service: Arc<NexusService>,
    model_work: &ModelWorkScheduler,
    execution_gate: &BlockingExecutionGate,
    request: ServiceRequest,
    project_id: String,
    evidence: ClientFilesystemEvidence,
) -> ServiceResponse {
    handle_with_evidence(
        service,
        model_work,
        request,
        Some(project_id),
        Some(evidence),
        Some(execution_gate),
    )
    .await
}

async fn handle_with_evidence(
    service: Arc<NexusService>,
    model_work: &ModelWorkScheduler,
    request: ServiceRequest,
    project_id: Option<String>,
    client_evidence: Option<ClientFilesystemEvidence>,
    execution_gate: Option<&BlockingExecutionGate>,
) -> ServiceResponse {
    if matches!(&request, ServiceRequest::MemoryConsolidate) {
        return start_memory_consolidation(service);
    }
    let (response, work) = dispatch_request(
        service.clone(),
        request,
        project_id,
        client_evidence,
        execution_gate,
    )
    .await;
    if let Some(work) = work {
        match work {
            ModelWorkJob::Papercut(job) => model_work.schedule_papercut(job),
            ModelWorkJob::Sos(job) => model_work.schedule_sos(job),
        }
    }
    response
}

fn start_memory_consolidation(service: Arc<NexusService>) -> ServiceResponse {
    match service.try_claim_memory_consolidation() {
        Ok(false) => {}
        Ok(true) => {
            drop(tokio::task::spawn_blocking(move || {
                let _ = service.run_claimed_memory_consolidation();
            }));
        }
        Err(error) => return ServiceResponse::error(error),
    }
    ServiceResponse::MemoryConsolidation(MemoryConsolidationReport {
        ok: true,
        in_progress: true,
        ..MemoryConsolidationReport::default()
    })
}

async fn dispatch_request(
    service: Arc<NexusService>,
    request: ServiceRequest,
    project_id: Option<String>,
    client_evidence: Option<ClientFilesystemEvidence>,
    execution_gate: Option<&BlockingExecutionGate>,
) -> (ServiceResponse, Option<ModelWorkJob>) {
    match request {
        ServiceRequest::Papercuts(command) => {
            dispatch_model_work(project_id, execution_gate, move |binding| {
                service.dispatch_papercuts(command, &binding)
            })
            .await
        }
        ServiceRequest::Sos(command) => {
            dispatch_model_work(project_id, execution_gate, move |binding| {
                service.dispatch_sos(command, &binding)
            })
            .await
        }
        request => (
            dispatch_regular(
                service,
                request,
                project_id,
                client_evidence,
                execution_gate,
            )
            .await,
            None,
        ),
    }
}

async fn dispatch_model_work<T>(
    project_id: Option<String>,
    execution_gate: Option<&BlockingExecutionGate>,
    work: impl FnOnce(ProjectBinding) -> T + Send + 'static,
) -> (ServiceResponse, Option<ModelWorkJob>)
where
    T: CompletedModelDispatch + Send + 'static,
{
    let binding = project_id
        .map(|project_id| ProjectBinding::Remote { project_id })
        .unwrap_or(ProjectBinding::Local);
    match run_blocking(execution_gate, move || work(binding)).await {
        Ok(completed) => completed.into_parts(),
        Err(error) => (ServiceResponse::error(error), None),
    }
}

async fn dispatch_regular(
    service: Arc<NexusService>,
    request: ServiceRequest,
    project_id: Option<String>,
    client_evidence: Option<ClientFilesystemEvidence>,
    execution_gate: Option<&BlockingExecutionGate>,
) -> ServiceResponse {
    run_blocking(execution_gate, move || {
        match (project_id, client_evidence) {
            (Some(project_id), Some(evidence)) => {
                service.handle_remote_observed(project_id, request, evidence)
            }
            (Some(project_id), None) => service.handle_remote(project_id, request),
            (None, None) => service.handle(request),
            (None, Some(_)) => unreachable!("local requests cannot carry client evidence"),
        }
    })
    .await
    .unwrap_or_else(ServiceResponse::error)
}

async fn run_blocking<F, T>(
    execution_gate: Option<&BlockingExecutionGate>,
    work: F,
) -> Result<T, BlockingExecutionError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match execution_gate {
        Some(execution_gate) => execution_gate.run(work).await,
        None => tokio::task::spawn_blocking(work)
            .await
            .map_err(BlockingExecutionError::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    fn review_job(id: &str) -> crate::coordination::PapercutReviewJob {
        crate::coordination::PapercutReviewJob {
            project_id: "project-a".into(),
            papercut_id: id.into(),
            reviewer_worktree: None,
        }
    }

    #[test]
    fn papercut_review_queue_deduplicates_and_rejects_work_beyond_capacity() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let queue = ModelWorkQueue::new(sender);

        assert_eq!(
            queue.schedule(ModelWorkJob::Papercut(review_job("one"))),
            ReviewScheduleOutcome::Scheduled
        );
        assert_eq!(
            queue.schedule(ModelWorkJob::Papercut(review_job("one"))),
            ReviewScheduleOutcome::Duplicate
        );
        assert_eq!(
            queue.schedule(ModelWorkJob::Papercut(review_job("two"))),
            ReviewScheduleOutcome::AtCapacity
        );

        let completed = receiver.try_recv().expect("first review was queued");
        queue.complete(&completed);
        assert_eq!(
            queue.schedule(ModelWorkJob::Papercut(review_job("one"))),
            ReviewScheduleOutcome::Scheduled
        );
    }

    #[test]
    fn queued_and_recovered_work_share_one_pending_capacity() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(MAX_QUEUED_MODEL_WORK + 1);
        let queue = ModelWorkQueue::new(sender);
        for ordinal in 0..(MAX_QUEUED_MODEL_WORK - 1) {
            assert_eq!(
                queue.schedule(ModelWorkJob::Papercut(review_job(&format!(
                    "queued-{ordinal}"
                )))),
                ReviewScheduleOutcome::Scheduled
            );
        }
        assert!(claim_work(
            &queue.pending,
            ModelWorkJob::Papercut(review_job("recovered"))
        )
        .is_some());

        assert_eq!(
            queue.schedule(ModelWorkJob::Papercut(review_job("overflow"))),
            ReviewScheduleOutcome::AtCapacity
        );
        assert!(claim_work(
            &queue.pending,
            ModelWorkJob::Papercut(review_job("recovered-overflow"))
        )
        .is_none());
    }

    #[tokio::test]
    async fn dropping_all_schedulers_releases_the_review_worker_service() {
        let config = crate::config::Config::default();
        let service = Arc::new(crate::coordination::NexusService::from_store(
            crate::config::LoadedConfig {
                config,
                sources: Vec::new(),
                hash: "review-worker-lifetime".into(),
            },
            crate::persistence::Store::open_memory().unwrap(),
        ));
        let weak_service = Arc::downgrade(&service);
        let scheduler = ModelWorkScheduler::start(service.clone());

        drop(service);
        drop(scheduler);

        tokio::time::timeout(Duration::from_secs(1), async {
            while weak_service.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("review worker retained the service after every scheduler was dropped");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_caller_does_not_release_blocking_execution_capacity() {
        let gate = BlockingExecutionGate::new(1, Duration::from_millis(25));
        let (started_sender, started_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let first_gate = gate.clone();
        let first = tokio::spawn(async move {
            first_gate
                .run(move || {
                    started_sender.send(()).unwrap();
                    release_receiver.recv().unwrap();
                })
                .await
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the first blocking worker must start");

        first.abort();
        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_in_worker = second_started.clone();
        let error = gate
            .run(move || second_started_in_worker.store(true, Ordering::SeqCst))
            .await
            .expect_err("the detached first worker must retain its permit");
        assert!(matches!(error, BlockingExecutionError::AtCapacity));
        assert!(!second_started.load(Ordering::SeqCst));

        release_sender.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match gate.run(|| ()).await {
                    Ok(()) => break,
                    Err(BlockingExecutionError::AtCapacity) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected execution error: {error}"),
                }
            }
        })
        .await
        .expect("capacity must return when the blocking worker exits");
    }
}
