// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @entrypoint capture_lifecycle
// @boundary work-outbox-wire: decode stored typed recording envelopes and bounded transport acknowledgments, never arbitrary workload inputs.
mod activity;
mod context_bridge;
mod git;
mod handoff;
mod health;
mod host_context;
mod host_events;
#[cfg(test)]
mod host_hint_tests;
mod lifecycle;
mod outbox;
mod session_title;
mod source_clock;
pub use session_title::resolve_session_title;
#[cfg(test)]
mod reliability_tests;
#[cfg(all(test, feature = "sqlite"))]
mod replay_tests;
use super::client::NexusClient;
use crate::config::{ClientConfig, LoadedConfig};
use crate::coordination::api::{ServiceRequest, WorkCommand};
use crate::coordination::domain::HookContext;
use crate::work_history::{WorkAction, WorkObservationInput};
pub use activity::{ActivityCapture, CaptureContext, WorkExecutor};
use anyhow::{Context, Result};
pub use context_bridge::active_host_context;
pub use git::git_hook;
pub(crate) use handoff::CaptureHandoff;
pub use handoff::{CaptureReceipt, CaptureStatus};
pub use host_context::{host_handoff, WorkHandoff};
pub use host_events::{host_event, host_event_with_title};
pub(crate) use lifecycle::lifecycle_command as prepare_lifecycle;
use lifecycle::stable_id;
use outbox::Outbox;
pub use outbox::{CaptureHealth, InvalidRetry, RetryInvalidReport};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn process_instance_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

pub(crate) fn directory(loaded: &LoadedConfig) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("NEXUS_WORK_OUTBOX_DIR") {
        return Ok(path.into());
    }
    let key = match &loaded.config.client {
        ClientConfig::Local => {
            return Ok(loaded
                .config
                .storage
                .database_path
                .parent()
                .context("database directory unavailable")?
                .join("work-outbox"));
        }
        ClientConfig::Remote(remote) => format!("{}:{}", remote.endpoint, remote.project_id),
    };
    Ok(dirs::data_local_dir()
        .context("local data directory unavailable")?
        .join("nexus/work-outbox")
        .join(stable_id(&[&key])))
}

fn project_id(loaded: &LoadedConfig, root: Option<&str>) -> Result<String> {
    match &loaded.config.client {
        ClientConfig::Remote(remote) => Ok(remote.project_id.trim().to_owned()),
        ClientConfig::Local => crate::coordination::local_project_id(root),
    }
}

pub fn health(loaded: &LoadedConfig) -> Result<CaptureHealth> {
    Outbox::open(&directory(loaded)?)?.health()
}

pub fn retry_invalid(loaded: &LoadedConfig, mode: InvalidRetry) -> Result<RetryInvalidReport> {
    Outbox::writer(&directory(loaded)?)?.retry_invalid(mode)
}

pub fn capture_lifecycle(loaded: &LoadedConfig, command: WorkCommand) -> CaptureReceipt {
    capture_lifecycle_with_budget(loaded, command, Duration::from_millis(75))
}

pub fn capture_lifecycle_with_budget(
    loaded: &LoadedConfig,
    command: WorkCommand,
    budget: Duration,
) -> CaptureReceipt {
    CaptureHandoff::default().capture_blocking(Arc::new(loaded.clone()), command, budget)
}

/// Give admitted jobs a bounded persistence window before this producer exits, then deliver.
/// Jobs still pending after the deadline retain no durability or delivery promise.
pub async fn flush_before_exit(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
) -> Result<CaptureHealth> {
    flush_before_exit_with_budget(loaded, explicit_config, Duration::from_secs(2)).await
}

pub(crate) async fn flush_before_exit_with_budget(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    budget: Duration,
) -> Result<CaptureHealth> {
    let deadline = Instant::now() + budget;
    let _ = CaptureHandoff::default().drain_pending(Duration::from_millis(1500).min(budget));
    let mut health = health(loaded)?;
    while health.queued > 0 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let previous = health.queued;
        match tokio::time::timeout(
            remaining,
            flush_with_budget(loaded, explicit_config, remaining),
        )
        .await
        {
            Ok(next) => health = next?,
            Err(_) => break,
        }
        if health.queued >= previous {
            tokio::time::sleep(
                Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        }
    }
    Ok(health)
}

pub fn capture_adapter_rejection(
    loaded: &LoadedConfig,
    host: &str,
    budget: Duration,
) -> CaptureReceipt {
    CaptureHandoff::default().reject_adapter(Arc::new(loaded.clone()), host.into(), budget)
}

fn persist_lifecycle(loaded: &LoadedConfig, mut command: WorkCommand) -> CaptureReceipt {
    context_bridge::enrich(loaded, &mut command);
    if let WorkAction::Record { observations } = command.command {
        return enqueue_receipt(loaded, &command.context, observations);
    }
    CaptureReceipt::lost(0, "unsupported_command")
}

pub fn enqueue(
    loaded: &LoadedConfig,
    context: &HookContext,
    observations: Vec<WorkObservationInput>,
) -> Result<()> {
    let receipt = capture_lifecycle(
        loaded,
        WorkCommand {
            context: context.clone(),
            command: WorkAction::Record { observations },
        },
    );
    if receipt.lost > 0 {
        anyhow::bail!(
            "work capture lost observations: {}",
            receipt.reason.unwrap_or("capture_failure")
        );
    }
    Ok(())
}

fn enqueue_receipt(
    loaded: &LoadedConfig,
    context: &HookContext,
    observations: Vec<WorkObservationInput>,
) -> CaptureReceipt {
    let mut receipt = CaptureReceipt {
        status: CaptureStatus::Durable,
        offered: observations.len(),
        queued: 0,
        duplicates: 0,
        rejected: 0,
        lost: 0,
        loss_reasons: Default::default(),
        rejection_reasons: Default::default(),
        reason: None,
    };
    let path = match directory(loaded) {
        Ok(path) => path,
        Err(_) => return CaptureReceipt::lost(observations.len(), "directory_unavailable"),
    };
    let project = match project_id(loaded, context.project_root.as_deref()) {
        Ok(project) => project,
        Err(_) => {
            let receipt = CaptureReceipt::lost(observations.len(), "project_unavailable");
            outbox::record_capture(&path, context, &receipt);
            return receipt;
        }
    };
    let outbox = match Outbox::writer(&path) {
        Ok(outbox) => outbox,
        Err(_) => {
            let receipt = CaptureReceipt::lost(observations.len(), "writer_unavailable");
            outbox::record_capture(&path, context, &receipt);
            return receipt;
        }
    };
    for observation in observations {
        account_enqueue(
            &mut receipt,
            persist_observation(&outbox, &project, context, observation),
        );
    }
    outbox::record_capture(&path, context, &receipt);
    receipt
}

fn persist_observation(
    outbox: &Outbox,
    project: &str,
    context: &HookContext,
    observation: WorkObservationInput,
) -> Result<outbox::Enqueued> {
    let key = stable_id(&[project, &observation.id]);
    let command = WorkCommand {
        context: context.clone(),
        command: WorkAction::Record {
            observations: vec![observation],
        },
    };
    outbox.enqueue_once(&key, &serde_json::to_vec(&command)?)
}

fn account_enqueue(receipt: &mut CaptureReceipt, result: Result<outbox::Enqueued>) {
    match result {
        Ok(outbox::Enqueued::New) => receipt.queued += 1,
        Ok(outbox::Enqueued::Duplicate) => receipt.duplicates += 1,
        Ok(outbox::Enqueued::Rejected) => receipt.record_rejection("known_rejection"),
        Ok(outbox::Enqueued::Conflict) => receipt.record_rejection("conflict"),
        Err(error) => receipt.record_loss(
            if error.downcast_ref::<outbox::CapacityExceeded>().is_some() {
                "capacity"
            } else {
                "enqueue_io"
            },
        ),
    }
}

pub async fn flush(loaded: &LoadedConfig, explicit_config: Option<&Path>) -> Result<CaptureHealth> {
    flush_with_budget(loaded, explicit_config, Duration::from_millis(250)).await
}

async fn flush_with_budget(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    budget: Duration,
) -> Result<CaptureHealth> {
    let path = directory(loaded)?;
    let pending = Outbox::open(&path)?.pending()?;
    let client = NexusClient::without_analytics(
        Arc::new(loaded.clone()),
        explicit_config.map(Path::to_path_buf),
    );
    let deadline = Instant::now() + budget;
    for entry in pending {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero()
            || matches!(
                deliver(&client, &path, &entry, remaining).await?,
                Delivery::RetryLater
            )
        {
            break;
        }
    }
    Outbox::open(&path)?.health()
}

#[derive(Clone, Copy)]
enum Delivery {
    Recorded,
    Rejected(&'static str),
    RetryLater,
}

async fn deliver(
    client: &NexusClient,
    path: &Path,
    entry: &outbox::Pending,
    budget: Duration,
) -> Result<Delivery> {
    let deadline = Instant::now() + budget;
    let command: WorkCommand = match serde_json::from_slice::<WorkCommand>(&entry.bytes) {
        Ok(command) if matches!(command.command, WorkAction::Record { .. }) => command,
        _ => return settle(path, entry, Delivery::Rejected("corrupt")),
    };
    if entry.phase() == outbox::PendingPhase::ReconcileCallback {
        let delivery = reconcile_callback(client, &command, deadline).await;
        return settle(path, entry, delivery);
    }
    let result = tokio::time::timeout(
        budget,
        client.ensure_and_request(&ServiceRequest::Work(command.clone())),
    )
    .await;
    let mut delivery = match result {
        Ok(Ok(response)) => disposition(&response),
        _ => Delivery::RetryLater,
    };
    if matches!(delivery, Delivery::Rejected("conflict")) {
        delivery = reconcile_callback(client, &command, deadline).await;
        if matches!(delivery, Delivery::RetryLater) && callback_candidate(&command).is_some() {
            Outbox::open(path)?.defer_callback(entry)?;
        }
    }
    settle(path, entry, delivery)
}

fn callback_replay_matches(
    accepted: &WorkObservationInput,
    incoming: &WorkObservationInput,
) -> bool {
    same_companion_body(accepted, incoming) && accepted.method == "companion_callback_wall_clock"
}

fn same_companion_body(accepted: &WorkObservationInput, incoming: &WorkObservationInput) -> bool {
    accepted.id == incoming.id
        && accepted.subject == incoming.subject
        && accepted.body == incoming.body
        && accepted.correction == incoming.correction
        && incoming.method == accepted.method
        && accepted.producer.name == "nexus-companion"
        && incoming.producer.name == accepted.producer.name
}

async fn reconcile_callback(
    client: &NexusClient,
    command: &WorkCommand,
    deadline: Instant,
) -> Delivery {
    let Some(incoming) = callback_candidate(command) else {
        return Delivery::Rejected("conflict");
    };
    match lookup_callback(client, command, incoming, deadline).await {
        Ok(Some(accepted)) if callback_replay_matches(&accepted.input, incoming) => {
            Delivery::Recorded
        }
        Ok(_) => Delivery::Rejected("conflict"),
        Err(_) => Delivery::RetryLater,
    }
}

fn callback_candidate(command: &WorkCommand) -> Option<&WorkObservationInput> {
    let WorkAction::Record { observations } = &command.command else {
        return None;
    };
    let [incoming] = observations.as_slice() else {
        return None;
    };
    (incoming.producer.name == "nexus-companion"
        && incoming.method == "companion_callback_wall_clock")
        .then_some(incoming)
}

async fn lookup_callback(
    client: &NexusClient,
    command: &WorkCommand,
    incoming: &WorkObservationInput,
    deadline: Instant,
) -> Result<Option<crate::work_history::WorkObservation>> {
    let lookup = ServiceRequest::Work(WorkCommand {
        context: command.context.clone(),
        command: WorkAction::Observation {
            project_id: None,
            id: incoming.id.clone(),
        },
    });
    let remaining = deadline.saturating_duration_since(Instant::now());
    let response = tokio::time::timeout(remaining, client.ensure_and_request(&lookup)).await??;
    if response.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        anyhow::bail!("callback lookup unavailable");
    }
    response
        .get("observation")
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

fn disposition(response: &serde_json::Value) -> Delivery {
    match (
        response.get("response").and_then(|v| v.as_str()),
        response.get("error_code").and_then(|v| v.as_str()),
    ) {
        (Some("recorded"), _) if response.get("ok").and_then(|v| v.as_bool()) == Some(true) => {
            Delivery::Recorded
        }
        (_, Some("invalid_input")) => Delivery::Rejected("invalid"),
        (_, Some("conflict")) => Delivery::Rejected("conflict"),
        _ => Delivery::RetryLater,
    }
}

fn settle(path: &Path, entry: &outbox::Pending, delivery: Delivery) -> Result<Delivery> {
    let outbox = Outbox::open(path)?;
    match delivery {
        Delivery::Recorded => outbox.acknowledge(entry)?,
        Delivery::Rejected(reason) => outbox.reject(entry, reason)?,
        Delivery::RetryLater => {}
    }
    Ok(delivery)
}

pub(crate) struct WorkDrain(tokio::task::JoinHandle<()>);
impl Drop for WorkDrain {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn start_drain(
    loaded: Arc<LoadedConfig>,
    explicit: Option<PathBuf>,
) -> Option<WorkDrain> {
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    Some(WorkDrain(runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let _ = flush(&loaded, explicit.as_deref()).await;
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::api::{ServiceRequest, ToolHookPhase};
    use crate::coordination::domain::{HookContext, ToolHookInput};
    use crate::work_history::{WorkAction, WorkEvent};

    #[test]
    fn clock_identity_is_one_uuid_per_process() {
        let identity = process_instance_id();
        assert!(uuid::Uuid::parse_str(identity).is_ok());
        assert_eq!(identity, process_instance_id());
    }

    #[tokio::test]
    async fn dropping_the_client_drain_releases_its_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = crate::Config::default();
        config.storage.database_path = temp.path().join("nexus.db");
        config.runtime.socket_path = temp.path().join("nexus.sock");
        let loaded = Arc::new(LoadedConfig {
            config,
            hash: "test".into(),
            sources: Vec::new(),
        });
        let drain = start_drain(loaded.clone(), None);
        drop(drain);
        tokio::task::yield_now().await;
        assert_eq!(Arc::strong_count(&loaded), 1);
    }

    #[test]
    fn host_tools_share_a_work_root_and_keep_explicit_parentage_without_arguments() {
        let context = HookContext {
            session_id: "host-session".into(),
            agent: "pi".into(),
            project_root: Some("/checkout".into()),
            turn_id: Some("turn".into()),
            model: None,
        };
        let tool = ToolHookInput {
            client_path_intents: Vec::new(),
            is_interrupt: None,
            agent_id: None,
            context,
            tool_use_id: "child".into(),
            tool_name: "bash".into(),
            parent_tool_use_id: Some("parent".into()),
            tool_input: serde_json::json!({"command":"secret text"}).into(),
            tool_output: None,
            error: None,
        };
        let request = ServiceRequest::ToolHook {
            phase: ToolHookPhase::Before,
            input: tool,
        };
        let command = prepare_lifecycle(&request).unwrap();
        let WorkAction::Record { observations } = command.command else {
            panic!("record action")
        };
        let declaration = observations
            .iter()
            .find_map(|item| match &item.body {
                WorkEvent::ActivityDeclared {
                    kind,
                    parent_activity_id,
                    work_session_id,
                    ..
                } if kind == "tool" => Some((parent_activity_id, work_session_id)),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            declaration.0.as_deref(),
            Some(stable_id(&["pi", "host-session", "parent"]).as_str())
        );
        assert!(declaration.1.is_some());
        assert!(observations
            .iter()
            .any(|item| matches!(item.body, WorkEvent::Started)));
        assert!(!serde_json::to_string(&observations)
            .unwrap()
            .contains("secret text"));
    }
}
