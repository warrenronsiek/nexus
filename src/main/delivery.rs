// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
// @boundary delivery-cli-json: decode caller-owned typed observations from bounded files or stdin.
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use nexus::coordination::{api::ServiceRequest, domain::HookContext};
use nexus::delivery::*;
use nexus::work_history::GitRevision;
use nexus::LoadedConfig;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(super) enum DeliveryCliCommand {
    /// Print the canonical typed delivery recording and query schema.
    Schema,
    /// Record {observations:[...]}; - reads stdin. Events are immutable and replay-safe.
    Record { file: PathBuf },
    /// Register a committed change and its original caller context from bounded local Git metadata.
    Contribute(ContributeArgs),
    /// Discover PRs, rollouts, contributions, deployments, or remediation requests.
    List(ListArgs),
    /// Read an object, related contributions, and bounded source history.
    Get {
        kind: String,
        id: String,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Resolve changed code to originating agents with evidence and contact availability.
    #[command(alias = "resolve-origin")]
    Origin(OriginArgs),
    /// Read pending remediation requests for the caller or an explicit agent/session pair.
    Inbox {
        #[arg(long, requires = "session")]
        agent: Option<String>,
        #[arg(long, requires = "agent")]
        session: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Register yourself as shepherd, or explicitly hand off using the current assignment event ID.
    Shepherd(ShepherdArgs),
    /// Read registered watches and their latest refresh health.
    Watches {
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Register a durable GitHub watch from a DeliveryWatchRegistration JSON file.
    Watch { file: PathBuf },
    /// Stop a registered watch without deleting its observations.
    WatchStop { id: String },
}

#[derive(Debug, Args)]
pub(super) struct ContributeArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    title: String,
    #[arg(long)]
    rationale: String,
    #[arg(long)]
    repository: String,
    #[arg(long)]
    base: String,
    #[arg(long, default_value = "HEAD")]
    head: String,
    #[arg(long)]
    feature: Vec<String>,
    #[arg(long)]
    constraint: Vec<String>,
    #[arg(long)]
    work_session: Option<String>,
    #[arg(long)]
    activity: Option<String>,
}

#[derive(Debug, Default, Args)]
pub(super) struct ListArgs {
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long)]
    state: Option<String>,
    #[arg(long)]
    feature: Option<String>,
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    branch: Option<String>,
    #[arg(long)]
    after: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Debug, Args)]
pub(super) struct OriginArgs {
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long, requires = "repository")]
    revision: Option<String>,
    #[arg(long)]
    path: Option<String>,
    #[arg(long, requires = "path")]
    line_start: Option<u32>,
    #[arg(long, requires = "line_start")]
    line_end: Option<u32>,
    #[arg(long)]
    feature: Option<String>,
    #[arg(long)]
    fingerprint: Option<String>,
    #[arg(long)]
    pull_request: Option<String>,
    #[arg(long)]
    rollout: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Debug, Args)]
pub(super) struct ShepherdArgs {
    kind: String,
    id: String,
    /// Required for handoff/takeover; use assignment_event_id from get.
    #[arg(long)]
    expected_assignment: Option<String>,
    #[arg(long, requires = "session")]
    agent: Option<String>,
    #[arg(long, requires = "agent")]
    session: Option<String>,
    #[arg(long)]
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationBundle {
    observations: Vec<DeliveryObservationInput>,
}

pub(super) async fn run(
    command: DeliveryCliCommand,
    root: &Path,
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
) -> Result<()> {
    let context = caller_context(root, loaded);
    let result = match command {
        DeliveryCliCommand::Schema => serde_json::to_value(schemars::schema_for!(DeliveryAction))?,
        command => {
            let action = match command {
                DeliveryCliCommand::Contribute(args) => contribution(args, root, loaded)?,
                command => action(command, &context)?,
            };
            crate::history_client::HistoryClient::new(
                loaded,
                explicit,
                context,
                ServiceRequest::Delivery,
                "delivery coordination",
            )
            .send(action)
            .await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn action(command: DeliveryCliCommand, context: &HookContext) -> Result<DeliveryAction> {
    Ok(match command {
        DeliveryCliCommand::Record { file } => DeliveryAction::Record {
            observations: read_json::<ObservationBundle>(&file)?.observations,
        },
        DeliveryCliCommand::List(args) => args.into_action()?,
        DeliveryCliCommand::Get {
            kind,
            id,
            project_id,
        } => DeliveryAction::Get {
            project_id,
            subject: DeliverySubject {
                kind: enum_value(&kind)?,
                id,
            },
        },
        DeliveryCliCommand::Origin(args) => args.into_action(),
        DeliveryCliCommand::Inbox {
            agent,
            session,
            project_id,
            after,
            limit,
        } => DeliveryAction::Inbox {
            project_id,
            recipient: agent
                .zip(session)
                .map(|(agent, session_id)| AgentIdentity { agent, session_id }),
            after,
            limit,
        },
        DeliveryCliCommand::Shepherd(args) => args.into_action(context)?,
        DeliveryCliCommand::Watches { project_id } => DeliveryAction::Watches { project_id },
        DeliveryCliCommand::Watch { file } => event(DeliveryEvent::WatchRegistered {
            watch: read_json(&file)?,
        }),
        DeliveryCliCommand::WatchStop { id } => event(DeliveryEvent::WatchStopped { watch_id: id }),
        DeliveryCliCommand::Schema => unreachable!("schema is handled without a service request"),
        DeliveryCliCommand::Contribute(_) => {
            unreachable!("contribution capture is handled before transport")
        }
    })
}

fn event(body: DeliveryEvent) -> DeliveryAction {
    DeliveryAction::Record {
        observations: vec![DeliveryObservationInput {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: chrono::Utc::now(),
            body,
        }],
    }
}

fn enum_value<T: DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .context("unsupported delivery kind")
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    const MAX_BYTES: u64 = 768 * 1024;
    let mut reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path).context("open delivery JSON file")?)
    };
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("delivery JSON exceeds {MAX_BYTES} bytes");
    }
    serde_json::from_slice(&bytes).context("decode typed delivery JSON")
}

fn caller_context(root: &Path, loaded: &LoadedConfig) -> HookContext {
    nexus::runtime::work_capture::CaptureContext::from_environment(root)
        .with_active_host(loaded)
        .hook
}

fn contribution(
    args: ContributeArgs,
    root: &Path,
    loaded: &LoadedConfig,
) -> Result<DeliveryAction> {
    let capture =
        nexus::runtime::delivery_capture::capture(root, &args.repository, &args.base, &args.head)?;
    let caller = nexus::runtime::work_capture::CaptureContext::from_environment(root)
        .with_active_host(loaded);
    let (features, hunks) = changed_scope(capture.files, args.feature);
    let contribution = Contribution {
        id: args.id.clone(),
        repository: args.repository,
        title: args.title,
        origin: AgentIdentity {
            agent: caller.hook.agent,
            session_id: caller.hook.session_id,
        },
        work_session_id: args.work_session.or(caller.work_session_id),
        activity_id: args.activity.or(caller.parent_activity_id),
        worktree: Some(capture.worktree),
        branch: capture.branch,
        base_revision: Some(capture.base.clone()),
        head_revision: Some(capture.head.clone()),
        features,
        rationale: args.rationale,
        constraints: args.constraint,
        route: None,
    };
    let mapping = RevisionMapping {
        id: uuid::Uuid::new_v4().to_string(),
        contribution_id: args.id,
        revision: capture.head,
        source_revisions: vec![capture.base],
        operation: MappingOperation::Commit,
        // Git verifies the diff. The caller attests implementation responsibility.
        verification: MappingVerification::Supplied,
        hunks,
    };
    let occurred_at = chrono::Utc::now();
    Ok(DeliveryAction::Record {
        observations: [
            DeliveryEvent::ContributionRegistered { contribution },
            DeliveryEvent::RevisionMapped { mapping },
        ]
        .into_iter()
        .map(|body| DeliveryObservationInput {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at,
            body,
        })
        .collect(),
    })
}

impl ListArgs {
    fn into_action(self) -> Result<DeliveryAction> {
        Ok(DeliveryAction::List {
            project_id: self.project_id,
            query: DeliveryQuery {
                kind: self.kind.map(|v| enum_value(&v)).transpose()?,
                repository: self.repository,
                state: self.state,
                feature: self.feature,
                target: self.target,
                branch: self.branch,
                after: self.after,
                limit: self.limit,
            },
        })
    }
}
impl OriginArgs {
    fn into_action(self) -> DeliveryAction {
        let revision = self.revision.map(|commit| GitRevision {
            repository: self.repository.clone().expect("clap requires repository"),
            object_format: if commit.len() == 64 { "sha256" } else { "sha1" }.into(),
            commit,
        });
        DeliveryAction::ResolveOrigin {
            project_id: self.project_id,
            query: OriginQuery {
                revision,
                repository: self.repository,
                path: self.path,
                line_start: self.line_start,
                line_end: self.line_end,
                feature: self.feature,
                fingerprint: self.fingerprint,
                pull_request_id: self.pull_request,
                rollout_id: self.rollout,
                limit: self.limit,
            },
        }
    }
}
impl ShepherdArgs {
    fn into_action(self, context: &HookContext) -> Result<DeliveryAction> {
        let agent = self
            .agent
            .zip(self.session)
            .map(|(agent, session_id)| AgentIdentity { agent, session_id })
            .unwrap_or_else(|| AgentIdentity {
                agent: context.agent.clone(),
                session_id: context.session_id.clone(),
            });
        Ok(event(DeliveryEvent::ShepherdAssigned {
            subject: DeliverySubject {
                kind: enum_value(&self.kind)?,
                id: self.id,
            },
            expected_assignment: self.expected_assignment,
            agent: Some(agent),
            reason: self.reason,
        }))
    }
}
fn changed_scope(
    files: Vec<nexus::runtime::delivery_capture::CapturedFile>,
    labels: Vec<String>,
) -> (Vec<String>, Vec<ChangedHunk>) {
    let mut features = labels
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut hunks = Vec::new();
    for file in files {
        features.extend(file.features);
        if file.hunks.is_empty() {
            hunks.push(ChangedHunk {
                path: file.path,
                old_path: None,
                old_start: 0,
                old_lines: 0,
                new_start: 0,
                new_lines: 0,
                fingerprint: None,
            });
        } else {
            hunks.extend(file.hunks.into_iter().map(|hunk| ChangedHunk {
                path: file.path.clone(),
                old_path: None,
                old_start: hunk.old_start,
                old_lines: hunk.old_lines,
                new_start: hunk.new_start,
                new_lines: hunk.new_lines,
                fingerprint: Some(hunk.fingerprint),
            }));
        }
    }
    (features.into_iter().collect(), hunks)
}
