// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary caller-observation-file
use super::{Scope, WorkCliCommand};
use anyhow::{Context, Result};
use clap::Args;
use nexus::work_history::*;
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Args)]
pub(crate) struct StartArgs {
    name: String,
    #[arg(long)]
    id: Option<String>,
    #[arg(long, default_value = "command")]
    kind: String,
    #[arg(long)]
    work_session: Option<String>,
    #[arg(long)]
    parent: Option<String>,
    #[arg(long)]
    retry_of: Option<String>,
    #[arg(long)]
    environment: Option<String>,
    #[arg(long)]
    executor: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct FinishArgs {
    id: String,
    #[arg(long, value_enum, default_value_t = Scope::Activity)]
    scope: Scope,
    #[arg(long)]
    outcome: String,
    #[arg(long)]
    elapsed_ns: Option<String>,
    #[arg(long, default_value = "caller_reported")]
    timing_method: String,
    #[arg(long)]
    clock_id: Option<String>,
    #[arg(long)]
    exit_code: Option<i32>,
    #[arg(long)]
    signal: Option<i32>,
    #[arg(long)]
    assessment: Option<String>,
}

impl StartArgs {
    fn observations(self) -> Vec<WorkObservationInput> {
        let subject = WorkSubject::Activity {
            id: self.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        };
        let declaration = WorkEvent::ActivityDeclared {
            name: self.name,
            kind: self.kind,
            work_session_id: self
                .work_session
                .or_else(|| std::env::var("NEXUS_WORK_SESSION_ID").ok()),
            parent_activity_id: self
                .parent
                .or_else(|| std::env::var("NEXUS_PARENT_ACTIVITY_ID").ok()),
            initiator: std::env::var("NEXUS_AGENT").ok(),
            executor: self.executor,
            context: WorkContext {
                environment: self.environment,
                host_session_id: std::env::var("NEXUS_HOST_SESSION_ID").ok(),
                ..Default::default()
            },
        };
        let mut observations = vec![
            observation(subject.clone(), declaration),
            observation(subject.clone(), WorkEvent::Started),
        ];
        if let Some(id) = self.retry_of {
            observations.push(observation(
                subject,
                WorkEvent::Linked {
                    relation: WorkRelation::RetryOf,
                    target: WorkReference::Activity { id },
                },
            ));
        }
        observations
    }
}

impl FinishArgs {
    fn observation(self) -> Result<WorkObservationInput> {
        Ok(observation(
            self.scope.subject(self.id),
            WorkEvent::Finished {
                outcome: serde_json::from_value(Value::String(self.outcome))
                    .context("unknown work outcome")?,
                elapsed: self.elapsed_ns.map(|nanoseconds| Elapsed {
                    nanoseconds,
                    method: self.timing_method,
                    clock_id: self.clock_id,
                }),
                exit_code: self.exit_code,
                signal: self.signal,
                assessment: self.assessment,
            },
        ))
    }
}

fn observation(subject: WorkSubject, body: WorkEvent) -> WorkObservationInput {
    WorkObservationInput {
        id: Uuid::new_v4().to_string(),
        subject,
        occurred_at: chrono::Utc::now(),
        producer: WorkProducer {
            name: "nexus-cli".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            instance_id: nexus::runtime::work_capture::process_instance_id().into(),
        },
        method: "caller_reported".into(),
        body,
        correction: None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordFile {
    format_version: u32,
    observations: Vec<WorkObservationInput>,
}

pub(super) fn read_file(file: &Path) -> Result<Vec<WorkObservationInput>> {
    let input: Box<dyn Read> = if file == Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(file)?)
    };
    let mut bytes = Vec::new();
    input.take(768 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 768 * 1024,
        "work record input exceeds 768 KiB"
    );
    let file: RecordFile = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        file.format_version == 1,
        "unsupported work recording version"
    );
    Ok(file.observations)
}

pub(super) fn action(command: WorkCliCommand) -> Result<WorkAction> {
    let observations = match command {
        WorkCliCommand::Open { title, id } => vec![observation(
            WorkSubject::Session {
                id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            },
            WorkEvent::SessionOpened {
                title,
                context: WorkContext::default(),
            },
        )],
        WorkCliCommand::Start(args) => args.observations(),
        WorkCliCommand::Finish(args) => vec![args.observation()?],
        WorkCliCommand::Milestone {
            id,
            name,
            scope,
            environment,
        } => vec![observation(
            scope.subject(id),
            WorkEvent::Milestone {
                name,
                context: WorkContext {
                    environment,
                    ..Default::default()
                },
            },
        )],
        WorkCliCommand::Link {
            id,
            scope,
            relation,
            target,
        } => vec![observation(
            scope.subject(id),
            WorkEvent::Linked {
                relation: serde_json::from_value(Value::String(relation))
                    .context("unknown work relationship")?,
                target: serde_json::from_str(&target)
                    .context("target must be a typed WorkReference JSON object")?,
            },
        )],
        WorkCliCommand::Record { file } => read_file(&file)?,
        _ => unreachable!("read and local commands are dispatched before recording"),
    };
    Ok(WorkAction::Record { observations })
}
