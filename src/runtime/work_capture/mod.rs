// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @entrypoint capture_lifecycle
// @boundary work-outbox-wire: decode stored typed recording envelopes and bounded transport acknowledgments, never arbitrary workload inputs.
mod activity;
mod git;
mod lifecycle;
mod outbox;
use super::client::NexusClient;
use crate::config::{ClientConfig, LoadedConfig};
use crate::coordination::api::{ServiceRequest, WorkCommand};
use crate::coordination::domain::HookContext;
use crate::work_history::{WorkAction, WorkObservationInput};
pub use activity::{ActivityCapture, CaptureContext, WorkExecutor};
use anyhow::{Context, Result};
pub use git::git_hook;
pub(crate) use lifecycle::lifecycle_command as prepare_lifecycle;
use lifecycle::stable_id;
pub use outbox::CaptureHealth;
use outbox::Outbox;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn process_instance_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

fn directory(loaded: &LoadedConfig) -> Result<PathBuf> {
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
                .join("work-outbox"))
        }
        ClientConfig::Remote(remote) => format!("{}:{}", remote.endpoint, remote.project_id),
    };
    Ok(dirs::data_local_dir()
        .context("local data directory unavailable")?
        .join("nexus/work-outbox")
        .join(stable_id(&[&key])))
}

pub fn health(loaded: &LoadedConfig) -> Result<CaptureHealth> {
    Outbox::open(&directory(loaded)?)?.health()
}

pub fn capture_lifecycle(loaded: &LoadedConfig, command: WorkCommand) {
    if let WorkAction::Record { observations } = command.command {
        let _ = enqueue(loaded, &command.context, observations);
    }
}

pub fn enqueue(
    loaded: &LoadedConfig,
    context: &HookContext,
    observations: Vec<WorkObservationInput>,
) -> Result<()> {
    let outbox = Outbox::open(&directory(loaded)?)?;
    for observation in observations {
        let key = stable_id(&[&observation.id]);
        let command = WorkCommand {
            context: context.clone(),
            command: WorkAction::Record {
                observations: vec![observation],
            },
        };
        outbox.enqueue_once(&key, &serde_json::to_vec(&command)?)?;
    }
    Ok(())
}

pub async fn flush(loaded: &LoadedConfig, explicit_config: Option<&Path>) -> Result<CaptureHealth> {
    let path = directory(loaded)?;
    let pending = Outbox::open(&path)?.pending()?;
    let client = NexusClient::without_analytics(
        Arc::new(loaded.clone()),
        explicit_config.map(Path::to_path_buf),
    );
    let deadline = Instant::now() + Duration::from_millis(250);
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
    let command: WorkCommand = match serde_json::from_slice::<WorkCommand>(&entry.bytes) {
        Ok(command) if matches!(command.command, WorkAction::Record { .. }) => command,
        _ => return settle(path, entry, Delivery::Rejected("corrupt")),
    };
    let result = tokio::time::timeout(
        budget,
        client.ensure_and_request(&ServiceRequest::Work(command)),
    )
    .await;
    let delivery = match result {
        Ok(Ok(response)) => disposition(&response),
        _ => Delivery::RetryLater,
    };
    settle(path, entry, delivery)
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
