// @feature usage-analytics
// @feature runtime
// @spec docs/features/usage-analytics.md
// @spec docs/features/runtime.md
// @boundary local-classifier-jobs
use crate::analytics::{capture, classification, types::*};
use crate::config::LoadedConfig;
use crate::coordination::{
    api::{ServiceRequest, ToolHookPhase},
    domain::HookContext,
};
use crate::runtime::client::NexusClient;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Semaphore};

pub(crate) struct ClientAnalytics {
    actions: CaptureLane,
    inventory: CaptureLane,
}
struct CaptureLane {
    sender: mpsc::Sender<Prepared>,
    normalization: Arc<Semaphore>,
}
impl CaptureLane {
    fn new(capacity: usize) -> (Self, mpsc::Receiver<Prepared>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender,
                normalization: Arc::new(Semaphore::new(1)),
            },
            receiver,
        )
    }
    fn submit(&self, source: CaptureInput) {
        let Ok(permit) = self.normalization.clone().try_acquire_owned() else {
            return;
        };
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            for job in prepare(source) {
                let _ = sender.try_send(job);
            }
        });
    }
}
struct Prepared {
    context: HookContext,
    target: ClassificationTarget,
    input: String,
    created: Instant,
}

impl ClientAnalytics {
    pub fn start(loaded: Arc<LoadedConfig>, explicit: Option<PathBuf>) -> Option<Self> {
        if !loaded.config.analytics.classification_enabled
            || std::env::var_os("NEXUS_ANALYTICS_INTERNAL").is_some()
        {
            return None;
        }
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        let (actions, mut action_jobs) = CaptureLane::new(16);
        let (inventory, mut inventory_jobs) = CaptureLane::new(32);
        runtime.spawn(async move {
            let client = NexusClient::without_analytics(loaded.clone(), explicit);
            loop {
                let job = tokio::select! {biased;
                    Some(job)=action_jobs.recv()=>job,
                    Some(job)=inventory_jobs.recv()=>job,
                    else=>break,
                };
                if job.created.elapsed().as_secs() > 120 {
                    continue;
                }
                let _ = classify(&client, &loaded, job).await;
            }
        });
        Some(Self { actions, inventory })
    }
    pub fn observe(&self, request: &ServiceRequest) {
        match request {
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::After(completion),
                input,
            } => {
                let mut input = input.clone();
                input.tool_output = None;
                input.error = None;
                self.actions.submit(CaptureInput::Tool(input, *completion));
            }
            ServiceRequest::Analytics(AnalyticsCommand::Inventory {
                context, resources, ..
            }) => self
                .inventory
                .submit(CaptureInput::Inventory(context.clone(), resources.clone())),
            _ => {}
        }
    }
}
enum CaptureInput {
    Tool(
        crate::coordination::domain::ToolHookInput,
        crate::coordination::domain::ToolCompletion,
    ),
    Inventory(HookContext, Vec<Resource>),
}
fn prepare(source: CaptureInput) -> Vec<Prepared> {
    match source {
        CaptureInput::Tool(input, completion) => prepare_tool(input, completion),
        CaptureInput::Inventory(context, resources) => resources
            .into_iter()
            .filter(|resource| resource.activities.is_empty())
            .filter_map(|resource| {
                let text = classification::redact(&format!(
                    "Capability type: {}\nName: {}\nDescription: {}",
                    resource.kind.as_str(),
                    resource.name,
                    resource.description
                ))?;
                Some(Prepared {
                    context: context.clone(),
                    target: ClassificationTarget::Resource {
                        name: resource.name.clone(),
                        resource_kind: resource.kind,
                        pattern_key: resource_pattern(&resource),
                    },
                    input: text,
                    created: Instant::now(),
                })
            })
            .collect(),
    }
}
async fn classify(
    client: &NexusClient,
    loaded: &LoadedConfig,
    job: Prepared,
) -> anyhow::Result<()> {
    let permit = client
        .ensure_and_request(&ServiceRequest::Analytics(
            AnalyticsCommand::ReserveClassification {
                context: job.context.clone(),
                target: job.target.clone(),
            },
        ))
        .await?;
    if permit.get("granted").and_then(serde_json::Value::as_bool) != Some(true) {
        return Ok(());
    }
    let result = run_classifier(client, loaded, &job).await;
    let command = match result {
        Ok((draft, model)) => AnalyticsCommand::Classify {
            context: job.context,
            target: job.target,
            draft,
            model,
        },
        Err(_) => AnalyticsCommand::ClassificationFailed {
            context: job.context,
            target: job.target,
        },
    };
    client
        .ensure_and_request(&ServiceRequest::Analytics(command))
        .await?;
    Ok(())
}
async fn run_classifier(
    client: &NexusClient,
    loaded: &LoadedConfig,
    job: &Prepared,
) -> anyhow::Result<(CategoryDraft, String)> {
    let prompt = load_prompt(client, &job.input).await?;
    let provider = loaded.config.analyst.provider;
    let model = loaded.config.analyst.model(provider).clone();
    let model_id = format!(
        "{}:{}",
        crate::agents::model::provider_name(provider),
        model.model.as_deref().unwrap_or("configured-default")
    );
    let timeout =
        std::time::Duration::from_secs(loaded.config.analytics.classification_timeout_seconds);
    let text = tokio::task::spawn_blocking(move || {
        crate::agents::model::invoke_classification(provider, &model, &prompt, timeout)
    })
    .await??;
    Ok((classification::parse(&text)?, model_id))
}

async fn load_prompt(client: &NexusClient, input: &str) -> anyhow::Result<String> {
    let categories = client
        .ensure_and_request(&ServiceRequest::Analytics(AnalyticsCommand::Categories))
        .await?;
    let categories: Vec<Category> = serde_json::from_value(
        categories
            .get("categories")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("classifier category catalog unavailable"))?,
    )?;
    classification::prompt(input, &categories)
}

fn prepare_tool(
    input: crate::coordination::domain::ToolHookInput,
    completion: crate::coordination::domain::ToolCompletion,
) -> Vec<Prepared> {
    let evidence = input
        .context
        .project_root
        .as_deref()
        .map(|root| {
            crate::coordination::derive_filesystem_evidence(
                &input,
                completion,
                std::path::Path::new(root),
            )
        })
        .unwrap_or_default();
    let mut actions = capture::resolve(&input, &evidence.scripts);
    capture::version_scripts(&mut actions, &evidence.script_versions);
    actions
        .into_iter()
        .enumerate()
        .filter_map(|(index, action)| {
            let text = if action.kind == ResourceKind::Script {
                capture::script_input(
                    std::path::Path::new(input.context.project_root.as_deref()?),
                    &action.name,
                )?
            } else {
                action.model_input?
            };
            Some(Prepared {
                context: input.context.clone(),
                target: ClassificationTarget::Action {
                    invocation_id: input.tool_use_id.clone(),
                    index: index as u32,
                    pattern_key: action.pattern_key,
                },
                input: text,
                created: Instant::now(),
            })
        })
        .collect()
}
