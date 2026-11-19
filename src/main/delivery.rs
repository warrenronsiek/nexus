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
    /// Resolve the origin and queue a remediation for the recipient host.
    Remediate(RemediationArgs),
    /// Confirm receipt as the recorded recipient.
    Acknowledge { id: String },
    /// Show request state, verified receipt, and the latest observed reachability.
    RemediationStatus { id: String },
    /// Assess an environment and optionally check the pinned candidate or registered PR head.
    Environment(EnvironmentArgs),
    /// Find distinct retained PRs where an exact check name failed.
    CheckHistory {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        check: String,
        #[arg(long)]
        since: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Record release requirements, a target inventory, or a time-limited advisory.
    EnvironmentRecord {
        #[command(subcommand)]
        input: EnvironmentRecord,
    },
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
    #[command(flatten)]
    selectors: OriginSelectors,
}

#[derive(Debug, Args)]
pub(super) struct OriginSelectors {
    #[arg(long)]
    repository: Option<String>,
    /// Full Git object ID or a unique prefix of recorded revisions.
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
    /// Provider PR number with --repository, or an existing Nexus PR record ID.
    #[arg(long, conflicts_with = "pull_request_id")]
    pull_request: Option<String>,
    #[arg(long)]
    pull_request_id: Option<String>,
    #[arg(long, requires = "pull_request")]
    provider: Option<String>,
    #[arg(long)]
    branch: Option<String>,
    #[arg(long)]
    rollout: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
    /// Try the supplied selectors in evidence order.
    #[arg(long)]
    fallback: bool,
}

#[derive(Debug, Args)]
pub(super) struct RemediationArgs {
    #[command(flatten)]
    selectors: OriginSelectors,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    concern: Option<String>,
    #[arg(long, requires = "session")]
    agent: Option<String>,
    #[arg(long, requires = "agent")]
    session: Option<String>,
    #[arg(long)]
    title: String,
    #[arg(long)]
    message: String,
    #[arg(long = "evidence")]
    evidence: Vec<String>,
    /// File containing one evidence reference per line.
    #[arg(long)]
    evidence_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(super) struct EnvironmentArgs {
    #[arg(long)]
    environment: String,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long, requires = "repository", conflicts_with = "pull_request_id")]
    revision: Option<String>,
    #[arg(long)]
    pull_request_id: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(super) enum EnvironmentRecord {
    /// Record the cross-repository keys required by an exact candidate revision, as JSON.
    Requirements { file: PathBuf },
    /// Record key names present in a repository/environment, as JSON.
    Inventory { file: PathBuf },
    /// Record a time-limited operational advisory.
    Advisory {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        expires_at: chrono::DateTime<chrono::Utc>,
    },
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
    match command {
        DeliveryCliCommand::Record { file } => {
            read_json::<ObservationBundle>(&file).map(|bundle| DeliveryAction::Record {
                observations: bundle.observations,
            })
        }
        DeliveryCliCommand::List(args) => args.into_action(),
        DeliveryCliCommand::Get {
            kind,
            id,
            project_id,
        } => enum_value(&kind).map(|kind| DeliveryAction::Get {
            project_id,
            subject: DeliverySubject { kind, id },
        }),
        DeliveryCliCommand::Remediate(args) => args.into_action(),
        DeliveryCliCommand::Shepherd(args) => args.into_action(context),
        DeliveryCliCommand::Watch { file } => {
            read_json(&file).map(|watch| event(DeliveryEvent::WatchRegistered { watch }))
        }
        other => read_or_environment_action(other),
    }
}

fn read_or_environment_action(command: DeliveryCliCommand) -> Result<DeliveryAction> {
    Ok(match command {
        DeliveryCliCommand::Origin(args) => DeliveryAction::ResolveOrigin {
            project_id: args.project_id,
            query: args.selectors.into_query(),
        },
        DeliveryCliCommand::Acknowledge { id } => DeliveryAction::Acknowledge { id },
        DeliveryCliCommand::RemediationStatus { id } => DeliveryAction::RemediationStatus {
            project_id: None,
            id,
        },
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
        DeliveryCliCommand::Watches { project_id } => DeliveryAction::Watches { project_id },
        DeliveryCliCommand::WatchStop { id } => event(DeliveryEvent::WatchStopped { watch_id: id }),
        other => return environment_action(other),
    })
}

fn environment_action(command: DeliveryCliCommand) -> Result<DeliveryAction> {
    Ok(match command {
        DeliveryCliCommand::Environment(args) => DeliveryAction::EnvironmentContext {
            project_id: None,
            environment: args.environment,
            candidate: args
                .repository
                .zip(args.revision)
                .map(|(repo, commit)| git_revision(repo, commit)),
            pull_request_id: args.pull_request_id,
        },
        DeliveryCliCommand::CheckHistory {
            repository,
            check,
            since,
        } => DeliveryAction::CheckHistory {
            project_id: None,
            repository,
            check,
            since: since.unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(7)),
        },
        DeliveryCliCommand::EnvironmentRecord { input } => event(match input {
            EnvironmentRecord::Requirements { file } => {
                DeliveryEvent::ReleaseRequirementsRecorded {
                    requirements: read_json(&file)?,
                }
            }
            EnvironmentRecord::Inventory { file } => DeliveryEvent::EnvironmentInventoryObserved {
                inventory: read_json(&file)?,
            },
            EnvironmentRecord::Advisory {
                id,
                environment,
                title,
                description,
                expires_at,
            } => DeliveryEvent::EnvironmentAdvisoryReported {
                advisory: EnvironmentAdvisory {
                    id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    environment,
                    title,
                    description,
                    expires_at,
                },
            },
        }),
        DeliveryCliCommand::Schema => unreachable!("schema is handled without a service request"),
        DeliveryCliCommand::Contribute(_) => {
            unreachable!("contribution capture is handled before transport")
        }
        _ => unreachable!("core delivery command"),
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
                updated_since: None,
                after: self.after,
                limit: self.limit,
            },
        })
    }
}
impl OriginSelectors {
    fn into_query(self) -> OriginQuery {
        let (revision, revision_prefix) = match self.revision {
            Some(commit) if matches!(commit.len(), 40 | 64) => (
                self.repository
                    .clone()
                    .map(|repository| git_revision(repository, commit)),
                None,
            ),
            prefix => (None, prefix),
        };
        let pull_request_number = self
            .pull_request
            .as_deref()
            .filter(|_| self.repository.is_some())
            .and_then(|number| number.trim_start_matches('#').parse().ok());
        let pull_request_id = self
            .pull_request_id
            .or(self.pull_request.filter(|_| pull_request_number.is_none()));
        OriginQuery {
            revision,
            revision_prefix,
            repository: self.repository,
            path: self.path,
            line_start: self.line_start,
            line_end: self.line_end,
            feature: self.feature,
            fingerprint: self.fingerprint,
            pull_request_id,
            pull_request_number,
            pull_request_provider: self.provider,
            branch: self.branch,
            fallback: self.fallback,
            rollout_id: self.rollout,
            limit: self.limit,
        }
    }
}

fn git_revision(repository: String, commit: String) -> GitRevision {
    GitRevision {
        repository,
        object_format: if commit.len() == 64 { "sha256" } else { "sha1" }.into(),
        commit: commit.to_ascii_lowercase(),
    }
}

impl RemediationArgs {
    fn into_action(self) -> Result<DeliveryAction> {
        let mut evidence = self.evidence;
        if let Some(path) = self.evidence_file {
            let mut file = String::new();
            std::fs::File::open(path)
                .context("open remediation evidence file")?
                .take(128 * 1024 + 1)
                .read_to_string(&mut file)?;
            if file.len() > 128 * 1024 {
                bail!("remediation evidence file exceeds 128 KiB");
            }
            evidence.extend(
                file.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned),
            );
        }
        Ok(DeliveryAction::Remediate {
            request: Box::new(RemediationDraft {
                id: self.id,
                origin: self.selectors.into_query(),
                concern_id: self.concern,
                recipient: self
                    .agent
                    .zip(self.session)
                    .map(|(agent, session_id)| AgentIdentity { agent, session_id }),
                title: self.title,
                message: self.message,
                evidence,
            }),
        })
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
