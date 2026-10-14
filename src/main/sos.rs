// @feature runtime
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/sos.md
// @boundary cli-and-json
use anyhow::{Context, Result};
use clap::Subcommand;
use nexus::coordination::api::{ServiceRequest, SosCommand, SosDumpScope};
use nexus::coordination::domain::HookContext;
use nexus::sos::SosMetadata;
use nexus::LoadedConfig;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(super) enum SosCliCommand {
    /// List current aggregate SOS issues for this project.
    List,
    /// Inspect one aggregate issue, its source reports, and triage history.
    Get { issue_id: String },
    /// Dump the complete retained SOS registry.
    Dump {
        /// Include every observed project instead of only the current project.
        #[arg(long)]
        all_projects: bool,
    },
    /// Submit one complex problem for asynchronous triage.
    Report {
        message: String,
        #[arg(long = "type")]
        kind: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        trace: String,
        /// Arbitrary JSON object attached to the source report.
        #[arg(long, default_value = "{}")]
        metadata_json: String,
    },
    /// Mark an aggregate issue solved while retaining all source history.
    Resolve {
        issue_id: String,
        #[arg(long)]
        resolution: String,
    },
    /// Mark an aggregate issue obsolete while retaining all source history.
    Deprecate {
        issue_id: String,
        #[arg(long)]
        reason: String,
    },
}

pub(super) async fn run(command: SosCliCommand, root: &Path, loaded: &LoadedConfig) -> Result<()> {
    let context = || HookContext {
        session_id: "cli".into(),
        project_root: Some(root.to_string_lossy().into_owned()),
        agent: "user".into(),
        turn_id: None,
        model: None,
    };
    let command = match command {
        SosCliCommand::List => SosCommand::List { context: context() },
        SosCliCommand::Get { issue_id } => SosCommand::Get {
            context: context(),
            issue_id,
        },
        SosCliCommand::Dump { all_projects } => SosCommand::Dump {
            context: context(),
            scope: if all_projects {
                SosDumpScope::All
            } else {
                SosDumpScope::Project
            },
        },
        SosCliCommand::Report {
            message,
            kind,
            environment,
            version,
            model,
            trace,
            metadata_json,
        } => {
            let metadata = serde_json::from_str::<serde_json::Value>(&metadata_json)
                .context("--metadata-json must be valid JSON")?;
            let metadata = SosMetadata::try_from(metadata)
                .context("--metadata-json must be a JSON object within SOS bounds")?;
            let mut context = context();
            context.model = Some(model);
            SosCommand::Report {
                context,
                message,
                kind,
                environment,
                version,
                trace,
                metadata,
            }
        }
        SosCliCommand::Resolve {
            issue_id,
            resolution,
        } => SosCommand::Resolve {
            context: context(),
            issue_id,
            resolution,
        },
        SosCliCommand::Deprecate { issue_id, reason } => SosCommand::Deprecate {
            context: context(),
            issue_id,
            reason,
        },
    };
    super::request_and_print(loaded, ServiceRequest::Sos(command)).await
}
