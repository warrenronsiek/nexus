// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @entrypoint host_event
// @boundary native-host-work-events: retain typed identity, timing, and outcomes only; unknown prompt, result, transcript, and error fields are discarded.
use super::{
    lifecycle::{event, stable_id, HostCapture},
    CaptureContext,
};
use crate::coordination::{api::WorkCommand, domain::HookContext};
use crate::work_history::{
    Elapsed, WorkAction, WorkEvent, WorkObservationInput, WorkOutcome, WorkSubject,
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct HostEventInput {
    session_id: String,
    hook_event_name: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default, alias = "prompt_id")]
    turn_id: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default, deserialize_with = "crate::optional_hint::deserialize")]
    model: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    parent_agent_id: Option<String>,
    #[serde(default)]
    session_title: Option<String>,
    #[serde(default)]
    source_event_id: Option<String>,
    #[serde(default)]
    occurred_at: Option<DateTime<Utc>>,
    #[serde(default)]
    work_session_id: Option<String>,
    #[serde(default)]
    parent_activity_id: Option<String>,
    #[serde(default)]
    execution_id: Option<String>,
    #[serde(default)]
    executor: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    outcome: Option<WorkOutcome>,
    #[serde(default)]
    elapsed: Option<Elapsed>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    signal: Option<i32>,
}

pub fn host_event(host: &str, bytes: &[u8]) -> Result<WorkCommand> {
    host_event_with_title(host, bytes, None)
}

pub fn host_event_with_title(host: &str, bytes: &[u8], title: Option<&str>) -> Result<WorkCommand> {
    let input = decode_input(host, bytes)?;
    let mut capture = prepare_capture(host, &input);
    apply_event(&input, &mut capture, host)?;
    append_title(&input, title, &mut capture);
    qualify_source(&input, host, &mut capture.observations);
    Ok(WorkCommand {
        context: capture.context.hook,
        command: WorkAction::Record {
            observations: capture.observations,
        },
    })
}

fn decode_input(host: &str, bytes: &[u8]) -> Result<HostEventInput> {
    if !matches!(host, "codex" | "claude") {
        bail!("unsupported host adapter");
    }
    if bytes.len() > 1024 * 1024 {
        bail!("host event exceeds 1 MiB");
    }
    let input: HostEventInput = serde_json::from_slice(bytes).context("decode host event")?;
    input.validate()?;
    Ok(input)
}

fn prepare_capture(host: &str, input: &HostEventInput) -> HostCapture {
    let hook = HookContext {
        session_id: input.session_id.clone(),
        agent: host.into(),
        project_root: input.cwd.clone(),
        turn_id: input.turn_id.clone(),
        model: input.model.clone(),
    };
    let mut context = CaptureContext::from_hook(hook);
    context.work_session_id = input.work_session_id.clone().or(context.work_session_id);
    context.parent_activity_id = input
        .parent_activity_id
        .clone()
        .or(context.parent_activity_id);
    let mut capture = HostCapture::with_context(context);
    if !input.hook_event_name.starts_with("Execution") {
        capture.root();
    }
    capture
}

fn append_title(input: &HostEventInput, title: Option<&str>, capture: &mut HostCapture) {
    let Some(title) = input.session_title.as_deref().or(title) else {
        return;
    };
    let mut context = capture.role_context("container");
    context
        .labels
        .insert("nexus.session_title".into(), title.into());
    if let Some(revision) = &input.source_event_id {
        context
            .labels
            .insert("nexus.title.source_revision".into(), revision.clone());
    }
    let identity = input
        .source_event_id
        .as_deref()
        .or(input.turn_id.as_deref())
        .unwrap_or("initial");
    let mut subjects = vec![WorkSubject::Activity {
        id: capture.agent_id.clone(),
    }];
    if capture.context.work_session_id.is_none() {
        subjects.push(WorkSubject::Session {
            id: capture.work_id.clone(),
        });
    }
    for subject in subjects {
        capture.observations.push(event(
            subject,
            &format!("title-{identity}-{}", stable_id(&[title])),
            WorkEvent::Milestone {
                name: "Host session title updated".into(),
                context: context.clone(),
            },
        ));
    }
}

fn qualify_source(input: &HostEventInput, host: &str, observations: &mut [WorkObservationInput]) {
    let Some(at) = input.occurred_at else {
        return;
    };
    let source_id = stable_id(&["host-source", host, &input.session_id]);
    for observation in observations {
        observation.method = "host_source_boundary".into();
        observation.occurred_at = at;
        observation.source_clock = None;
        observation.producer.instance_id = source_id.clone();
    }
}

impl HostEventInput {
    fn validate(&self) -> Result<()> {
        for value in [
            Some(self.session_id.as_str()),
            Some(self.hook_event_name.as_str()),
            self.turn_id.as_deref(),
            self.request_id.as_deref(),
            self.agent_id.as_deref(),
            self.parent_agent_id.as_deref(),
            self.source_event_id.as_deref(),
            self.work_session_id.as_deref(),
            self.parent_activity_id.as_deref(),
            self.execution_id.as_deref(),
            self.executor.as_deref(),
            self.agent_type.as_deref(),
            self.kind.as_deref(),
            self.model.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
                bail!("invalid host event identity");
            }
        }
        for value in [self.session_title.as_deref(), self.name.as_deref()]
            .into_iter()
            .flatten()
        {
            if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
            {
                bail!("invalid host event label");
            }
        }
        Ok(())
    }
}

fn apply_event(input: &HostEventInput, capture: &mut HostCapture, host: &str) -> Result<()> {
    match input.hook_event_name.as_str() {
        "SessionStart" | "SessionTitle" | "SessionEnd" => session_event(input, capture),
        "UserPromptSubmit" | "RequestStart" | "RequestFinish" | "Stop" | "Interrupt"
        | "StopFailure" | "TurnStart" | "TurnFinish" => request_event(input, capture, host),
        "SubagentStart" | "SubagentStop" => child_event(input, capture),
        "ExecutionQueued" | "ExecutionStart" | "ExecutionPoll" | "ExecutionFinish" => {
            execution_event(input, capture)
        }
        _ => bail!("unsupported host event"),
    }
}

fn session_event(input: &HostEventInput, capture: &mut HostCapture) -> Result<()> {
    if input.hook_event_name == "SessionEnd" {
        capture.stop();
    }
    if input.hook_event_name == "SessionStart" && input.source.as_deref() == Some("resume") {
        let mut context = capture.role_context("container");
        context.labels.insert(
            "nexus.capture.gap".into(),
            "host_resume_epoch_unavailable".into(),
        );
        capture.observations.push(event(
            WorkSubject::Activity {
                id: capture.agent_id.clone(),
            },
            "resume-coverage-gap",
            WorkEvent::Milestone {
                name: "Host lifecycle coverage incomplete".into(),
                context,
            },
        ));
    }
    Ok(())
}

fn request_event(input: &HostEventInput, capture: &mut HostCapture, host: &str) -> Result<()> {
    match input.hook_event_name.as_str() {
        "UserPromptSubmit" | "RequestStart" => capture.request_start(input.request_id.as_deref()),
        "Stop" | "Interrupt" | "StopFailure" => capture.request_stop(
            input.request_id.as_deref(),
            response_outcome(host, &input.hook_event_name)?,
        ),
        "RequestFinish" => {
            let id = capture
                .request_id(input.request_id.as_deref())
                .context("request identity required")?;
            capture.finished(
                WorkSubject::Activity { id },
                input.outcome.unwrap_or(WorkOutcome::Unknown),
                "Caller observed request completion",
            );
        }
        "TurnStart" => {
            if input.turn_id.is_none() {
                bail!("turn_id is required");
            }
            let parent = capture
                .request_id(input.request_id.as_deref())
                .unwrap_or_else(|| capture.agent_id.clone());
            capture.declare_started(
                capture.turn_parent(),
                "Host turn",
                "host.turn",
                parent,
                None,
            );
        }
        "TurnFinish" => {
            if input.turn_id.is_none() {
                bail!("turn_id is required");
            }
            capture.finished(
                WorkSubject::Activity {
                    id: capture.turn_parent(),
                },
                input.outcome.unwrap_or(WorkOutcome::Unknown),
                "Caller observed turn completion; request and children may continue",
            );
        }
        _ => unreachable!("request event classified"),
    }
    Ok(())
}

fn response_outcome(host: &str, event: &str) -> Result<WorkOutcome> {
    match (host, event) {
        (_, "Stop") => Ok(WorkOutcome::Unknown),
        ("codex", "Interrupt") => Ok(WorkOutcome::Canceled),
        ("claude", "StopFailure") => Ok(WorkOutcome::InfrastructureError),
        _ => bail!("unsupported host terminal event"),
    }
}

fn child_event(input: &HostEventInput, capture: &mut HostCapture) -> Result<()> {
    let agent = input.agent_id.as_deref().context("agent_id is required")?;
    let id = stable_id(&[
        "child",
        &capture.context.hook.agent,
        &input.session_id,
        agent,
    ]);
    if input.hook_event_name == "SubagentStart" {
        let parent = input
            .parent_activity_id
            .clone()
            .or_else(|| {
                input.parent_agent_id.as_ref().map(|parent| {
                    stable_id(&[
                        "child",
                        &capture.context.hook.agent,
                        &input.session_id,
                        parent,
                    ])
                })
            })
            .unwrap_or_else(|| capture.turn_parent());
        capture.declare_started(
            id.clone(),
            input.agent_type.as_deref().unwrap_or("Child agent"),
            "agent.task",
            parent,
            Some(id),
        );
        if let Some(WorkObservationInput {
            body: WorkEvent::ActivityDeclared { context, .. },
            ..
        }) = capture
            .observations
            .iter_mut()
            .rev()
            .find(|o| matches!(o.body, WorkEvent::ActivityDeclared { .. }))
        {
            context.agent_instance_id = Some(agent.into());
        }
    } else {
        capture.finished(
            WorkSubject::Activity { id },
            input.outcome.unwrap_or(WorkOutcome::Unknown),
            "Observed child response boundary; descendants may continue",
        );
    }
    Ok(())
}

fn execution_event(input: &HostEventInput, capture: &mut HostCapture) -> Result<()> {
    let id = input
        .execution_id
        .as_deref()
        .context("execution_id is required")?;
    let subject = WorkSubject::Activity { id: id.into() };
    match input.hook_event_name.as_str() {
        "ExecutionStart" | "ExecutionQueued" => {
            let context = capture.role_context("execution");
            capture.observations.push(event(
                subject.clone(),
                "declared",
                WorkEvent::ActivityDeclared {
                    name: input
                        .name
                        .clone()
                        .unwrap_or_else(|| "Caller execution".into()),
                    kind: input.kind.clone().unwrap_or_else(|| "execution".into()),
                    work_session_id: Some(capture.work_id.clone()),
                    parent_activity_id: input
                        .parent_activity_id
                        .clone()
                        .or_else(|| Some(capture.turn_parent())),
                    initiator: Some(capture.context.hook.agent.clone()),
                    executor: input.executor.clone(),
                    context,
                },
            ));
            let (key, body) = if input.hook_event_name == "ExecutionStart" {
                ("started", WorkEvent::Started)
            } else {
                ("queued", WorkEvent::Queued)
            };
            capture.observations.push(event(subject, key, body));
        }
        "ExecutionFinish" => capture.observations.push(event(
            subject,
            "finished",
            WorkEvent::Finished {
                outcome: input.outcome.unwrap_or(WorkOutcome::Unknown),
                elapsed: input.elapsed.clone(),
                exit_code: input.exit_code,
                signal: input.signal,
                assessment: None,
            },
        )),
        "ExecutionPoll" => {}
        _ => unreachable!("execution event classified"),
    }
    Ok(())
}
