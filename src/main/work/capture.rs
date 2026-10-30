// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary caller-host-event-file: accept bounded source identity metadata, never execute or retain arbitrary hook payloads.
use anyhow::{bail, Result};
use clap::Args;
use nexus::{
    runtime::work_capture,
    work_history::{WorkAction, WorkEvent},
    LoadedConfig,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Args)]
pub(crate) struct ContextArgs {
    #[arg(long)]
    agent: String,
    #[arg(long)]
    host_session: String,
    #[arg(long)]
    turn: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    work_session: Option<String>,
    #[arg(long)]
    parent: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct HostEventArgs {
    #[arg(long)]
    host: String,
    file: PathBuf,
    #[arg(long, hide = true)]
    pub(super) advisory: bool,
}

pub(super) fn context(args: ContextArgs, loaded: &LoadedConfig) -> Result<Value> {
    let active = work_capture::active_host_context(loaded, &args.agent, &args.host_session);
    let mut handoff = match active {
        Ok(active) => active,
        Err(error) => match args.turn.as_deref() {
            Some(turn) => work_capture::host_handoff(
                &args.agent,
                &args.host_session,
                Some(turn),
                args.model.as_deref(),
            ),
            None => return Err(error),
        },
    };
    if let Some(turn) = args.turn {
        let supplied = work_capture::host_handoff(
            &args.agent,
            &args.host_session,
            Some(&turn),
            args.model.as_deref(),
        );
        handoff.turn_id = Some(turn);
        handoff.parent_activity_id = supplied.parent_activity_id;
    }
    if let Some(model) = args.model {
        handoff.model = Some(model);
    }
    handoff.work_session_id = args
        .work_session
        .or_else(|| std::env::var("NEXUS_WORK_SESSION_ID").ok())
        .unwrap_or(handoff.work_session_id);
    handoff.parent_activity_id = args
        .parent
        .or_else(|| std::env::var("NEXUS_PARENT_ACTIVITY_ID").ok())
        .or(handoff.parent_activity_id);
    let encoded = serde_json::to_vec(&handoff)?;
    work_capture::WorkHandoff::decode(&encoded)?;
    Ok(serde_json::to_value(handoff)?)
}

pub(super) async fn host_event(
    args: HostEventArgs,
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
) -> Result<Value> {
    let result = capture_host_event(&args, loaded, explicit).await;
    if result.is_err() {
        let _ = work_capture::capture_adapter_rejection(
            loaded,
            &args.host,
            std::time::Duration::from_millis(1500),
        );
    }
    result
}

async fn capture_host_event(
    args: &HostEventArgs,
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
) -> Result<Value> {
    let bytes = read_input(&args.file)?;
    // Stamp source boundaries before optional metadata or delivery adds latency.
    let command = work_capture::host_event(&args.host, &bytes)?;
    let receipt = work_capture::capture_lifecycle_with_budget(
        loaded,
        command,
        std::time::Duration::from_millis(1500),
    );
    capture_title(&args.host, &bytes, loaded).await?;
    let _ = work_capture::flush(loaded, explicit).await;
    let destination = std::env::var_os("CLAUDE_ENV_FILE").map(PathBuf::from);
    let environment_handoff =
        work_capture::export_host_environment(&args.host, &bytes, destination.as_deref());
    let health = work_capture::health(loaded).ok();
    Ok(
        json!({"capture":receipt,"health":health,"health_available":health.is_some(),"environment_handoff":environment_handoff}),
    )
}

async fn capture_title(host: &str, bytes: &[u8], loaded: &LoadedConfig) -> Result<()> {
    let Some(title) = work_capture::resolve_session_title(host, bytes).await else {
        return Ok(());
    };
    let mut command = work_capture::host_event_with_title(host, bytes, Some(&title))?;
    if let WorkAction::Record { observations } = &mut command.command {
        observations.retain(|item|matches!(&item.body,WorkEvent::Milestone {name,..} if name=="Host session title updated"));
    }
    let _ = work_capture::capture_lifecycle_with_budget(
        loaded,
        command,
        std::time::Duration::from_millis(250),
    );
    Ok(())
}

fn read_input(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let mut reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path)?)
    };
    reader
        .by_ref()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("host event exceeds 1 MiB");
    }
    Ok(bytes)
}

pub(super) async fn flush(
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
    limit: Option<u16>,
) -> Result<work_capture::CaptureHealth> {
    match limit {
        Some(limit) => work_capture::flush_pending(loaded, explicit, limit).await,
        None => work_capture::flush(loaded, explicit).await,
    }
}
