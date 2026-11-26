// @feature runtime
// @feature agent-traces
// @spec docs/features/runtime.md
// @spec docs/features/agent-traces.md
// @boundary dynamic-json
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use nexus::agent_traces::{
    TraceBatch, TraceCapability, TraceFilter, TraceOutcome, TraceQuery, TraceReadResponse,
    TraceVersionSelector,
};
use nexus::coordination::api::{ServiceRequest, TraceReadRequest};
use nexus::runtime::client::NexusClient;
use nexus::LoadedConfig;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(super) enum TraceCliCommand {
    /// List traces for the configured project.
    List(TraceFilterArgs),
    /// Read ordered events from a trace, or inspect one exact event.
    Get(TraceGetArgs),
    /// List tool and skill invocations with their observed outcomes.
    Calls(TraceFilterArgs),
    /// Aggregate tool and skill usage across application agents.
    Usage(TraceFilterArgs),
    /// Show trace ingestion status.
    Status(TraceScopeArgs),
    /// Print JSON Schemas for producer batches and read queries.
    Schema,
}

#[derive(Debug, Args)]
pub(super) struct TraceScopeArgs {
    /// Stable application identifier.
    #[arg(long = "application", visible_alias = "application-id")]
    application_id: Option<String>,
    /// Application environment, such as production or staging.
    #[arg(long)]
    environment: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct TraceFilterArgs {
    #[command(flatten)]
    scope: TraceScopeArgs,
    /// Stable configured agent identifier.
    #[arg(long = "agent", visible_alias = "agent-id")]
    agent_id: Option<String>,
    /// Exact tool or skill name.
    #[arg(long)]
    name: Option<String>,
    /// Exact tool or skill version.
    #[arg(long)]
    version: Option<String>,
    /// Match calls whose producer did not supply a version.
    #[arg(long, conflicts_with = "version")]
    unversioned: bool,
    /// Capability kind: tool or skill.
    #[arg(long)]
    kind: Option<TraceCapability>,
    /// Exact recorded outcome, such as succeeded, failed, or timed_out.
    #[arg(long)]
    outcome: Option<TraceOutcome>,
    /// Inclusive RFC 3339 occurrence-time lower bound.
    #[arg(long)]
    since: Option<String>,
    /// RFC 3339 occurrence-time upper bound.
    #[arg(long)]
    until: Option<String>,
    #[command(flatten)]
    page: TracePageArgs,
}

#[derive(Debug, Args)]
struct TracePageArgs {
    /// Maximum number of records in a page.
    #[arg(long)]
    limit: Option<usize>,
    /// Opaque next_cursor returned by an earlier request with these filters.
    #[arg(long)]
    cursor: Option<String>,
}

impl From<TraceFilterArgs> for TraceFilter {
    fn from(args: TraceFilterArgs) -> Self {
        let version = match args.version {
            Some(value) => TraceVersionSelector::Exact { value },
            None if args.unversioned => TraceVersionSelector::Unversioned {},
            None => TraceVersionSelector::Any {},
        };
        Self {
            application_id: args.scope.application_id,
            environment: args.scope.environment,
            agent_id: args.agent_id,
            name: args.name,
            version,
            kind: args.kind,
            outcome: args.outcome,
            since: args.since,
            until: args.until,
            limit: args.page.limit,
            cursor: args.page.cursor,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct TraceGetArgs {
    trace_id: String,
    #[arg(long = "application", visible_alias = "application-id")]
    application_id: String,
    #[arg(long)]
    environment: String,
    /// Read precisely this event within the trace.
    #[arg(long = "event", visible_alias = "event-id")]
    event_id: Option<String>,
    /// Include available captured prompt, message, tool, and error content.
    #[arg(long)]
    include_payload: bool,
    #[command(flatten)]
    page: TracePageArgs,
}

pub(super) fn print_schema() -> Result<()> {
    let schemas = json!({
        "batch": schemars::schema_for!(TraceBatch),
        "query": schemars::schema_for!(TraceQuery),
    });
    println!("{}", serde_json::to_string_pretty(&schemas)?);
    Ok(())
}

pub(super) async fn run(
    command: TraceCliCommand,
    root: &Path,
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
) -> Result<()> {
    let query = match command {
        TraceCliCommand::List(args) => TraceQuery::List {
            filter: args.into(),
        },
        TraceCliCommand::Calls(args) => TraceQuery::Calls {
            filter: args.into(),
        },
        TraceCliCommand::Usage(args) => TraceQuery::Usage {
            filter: args.into(),
        },
        TraceCliCommand::Get(args) => TraceQuery::Get {
            application_id: args.application_id,
            environment: args.environment,
            trace_id: args.trace_id,
            event_id: args.event_id,
            include_payload: args.include_payload,
            limit: args.page.limit,
            cursor: args.page.cursor,
        },
        TraceCliCommand::Status(args) => TraceQuery::Status {
            application_id: args.application_id,
            environment: args.environment,
        },
        TraceCliCommand::Schema => return print_schema(),
    };
    query.validate()?;
    let request = ServiceRequest::Traces(TraceReadRequest {
        query,
        project_root: Some(root.to_string_lossy().into_owned()),
    });
    let response = NexusClient::new(loaded, explicit_config)
        .ensure_and_request(&request)
        .await?;
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        bail!(
            "trace read failed: {}",
            response["error"].as_str().unwrap_or("invalid response")
        );
    }
    let response: TraceReadResponse =
        serde_json::from_value(response).context("decode trace read response")?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
