// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @boundary work-accounting-metadata: only recognized milestone names and bounded reserved values qualify projected titles, roles, source gaps, and executor identities; originals remain immutable.
use crate::work_history::*;
use std::collections::BTreeSet;

pub(super) fn project(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    host_origin(summary, events);
    session_title(summary, events);
    capture_gaps(summary, events);
    accounting_role(summary, events);
    executor(summary, events);
    tool_operation_title(summary, events);
    invocation_mode(summary, events);
    consumed_result(summary, events);
}
fn host_origin(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    if !matches!(summary.subject, WorkSubject::Session { .. }) {
        return;
    }
    let openings: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.input.body, WorkEvent::SessionOpened { .. }))
        .collect();
    let owner = openings.first().and_then(|event| event.host_root_owner());
    match owner.filter(|owner| {
        openings
            .iter()
            .all(|event| event.host_root_owner() == Some(owner))
    }) {
        Some(owner) => {
            summary
                .context
                .labels
                .insert("nexus.capture.host".into(), owner.into());
        }
        None => {
            summary.context.labels.remove("nexus.capture.host");
        }
    }
}

fn contexts<'a>(
    events: &'a [&WorkObservation],
    name: &'a str,
) -> impl Iterator<Item = &'a WorkContext> {
    events
        .iter()
        .filter_map(move |item| match &item.input.body {
            WorkEvent::Milestone {
                name: event_name,
                context,
            } if event_name == name => Some(context),
            _ => None,
        })
}
enum Agreement<T> {
    Absent,
    One(T),
    Conflict,
}
fn agreement<T: Ord>(values: impl IntoIterator<Item = T>) -> Agreement<T> {
    let mut values: BTreeSet<T> = values.into_iter().collect();
    if values.len() > 1 {
        Agreement::Conflict
    } else {
        values
            .pop_first()
            .map(Agreement::One)
            .unwrap_or(Agreement::Absent)
    }
}
fn role(value: &str) -> bool {
    matches!(value, "container" | "execution" | "wait")
}
fn accounting_role(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    let mut roles = contexts(events, "Accounting role observed")
        .filter_map(|context| context.labels.get("nexus.accounting.role"))
        .filter(|value| role(value))
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(declared) = summary
        .context
        .labels
        .get("nexus.accounting.role")
        .filter(|value| role(value))
    {
        roles.insert(declared.clone());
    }
    match agreement(roles) {
        Agreement::Absent => {}
        Agreement::One(value) => {
            summary
                .context
                .labels
                .insert("nexus.accounting.role".into(), value);
        }
        Agreement::Conflict => {
            summary
                .context
                .labels
                .insert("nexus.accounting.role".into(), "unknown".into());
            summary
                .conflicts
                .push("conflicting_accounting_roles".into());
        }
    }
}
fn executor(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    let contexts = contexts(events, "Executor identity observed").collect::<Vec<_>>();
    if contexts.is_empty() {
        return;
    }
    let unqualified = contexts.iter().any(|context| {
        context
            .labels
            .get("nexus.executor.declaration")
            .is_some_and(|value| value == "unqualified")
    });
    let mut identities = BTreeSet::new();
    if !unqualified {
        if let Some(declared) = &summary.executor {
            identities.insert(declared.clone());
        }
    }
    for context in contexts {
        if let Some(identity) = context
            .labels
            .get("nexus.executor.id")
            .filter(|value| !value.is_empty() && value.len() <= 512)
        {
            identities.insert(identity.clone());
        }
    }
    match agreement(identities) {
        Agreement::Conflict => {
            summary.executor = None;
            summary.conflicts.push("conflicting_executors".into());
        }
        Agreement::One(value) => summary.executor = Some(value),
        Agreement::Absent => summary.executor = None,
    }
}
fn session_title(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    if !matches!(summary.subject, WorkSubject::Session { .. }) && summary.kind != "host.session" {
        return;
    }
    let latest = events
        .iter()
        .filter_map(|item| match &item.input.body {
            WorkEvent::Milestone { name, context } if name == "Host session title updated" => {
                context
                    .labels
                    .get("nexus.session_title")
                    .filter(|title| !title.trim().is_empty())
                    .map(|title| (item.input.occurred_at, item.input.id.as_str(), title))
            }
            _ => None,
        })
        .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    if let Some((_, _, title)) = latest {
        summary.title = title.clone();
        summary
            .context
            .labels
            .insert("nexus.session_title".into(), title.clone());
    } else if let Some(title) = worktree_title(summary) {
        summary.title = title;
    }
}

/// Harness roots without a host display name read as `host · worktree-directory`.
/// Only the generated placeholder is replaced; the host session ID stays in context.
fn worktree_title(summary: &WorkSummary) -> Option<String> {
    let host = summary
        .context
        .labels
        .get("nexus.capture.host")
        .filter(|_| matches!(summary.subject, WorkSubject::Session { .. }))?;
    let generated = summary.title == "Observed agent work"
        || summary
            .context
            .host_session_id
            .as_ref()
            .is_some_and(|id| summary.title == format!("{host} session {id}"));
    let directory = summary
        .context
        .worktree
        .as_deref()
        .filter(|_| generated)
        .and_then(|worktree| std::path::Path::new(worktree).file_name())
        .and_then(std::ffi::OsStr::to_str)?;
    Some(format!("{host} · {directory}"))
}

fn capture_gaps(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    for item in events {
        let WorkEvent::Milestone { name, context } = &item.input.body else {
            continue;
        };
        let Some(gap) = context.labels.get("nexus.capture.gap") else {
            continue;
        };
        if matches!(
            (name.as_str(), gap.as_str()),
            ("Request boundary unavailable", "missing_request_identity")
                | (
                    "Exact host context unavailable",
                    "context_bridge_unavailable"
                )
                | (
                    "Host lifecycle coverage incomplete",
                    "host_resume_epoch_unavailable" | "request_context_evicted"
                )
        ) {
            summary.gaps.push(gap.clone());
        }
    }
}

fn tool_operation_title(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    if summary.kind == "tool" {
        let operations: BTreeSet<_> = contexts(events, "Tool operation observed")
            .filter_map(|context| context.labels.get("nexus.tool.operation"))
            .filter(|value| {
                !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
            })
            .collect();
        match agreement(operations) {
            Agreement::One(value) => summary.title = format!("{} · {}", summary.title, value),
            Agreement::Conflict => summary.gaps.push("ambiguous_tool_operation".into()),
            Agreement::Absent => {}
        }
    }
}

fn invocation_mode(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    let modes: BTreeSet<_> = contexts(events, "Invocation observed")
        .filter_map(|context| context.labels.get("nexus.flow.mode"))
        .chain(summary.context.labels.get("nexus.flow.mode"))
        .filter(|value| matches!(value.as_str(), "synchronous" | "asynchronous"))
        .cloned()
        .collect();
    match agreement(modes) {
        Agreement::Absent => {}
        Agreement::One(value) => {
            summary
                .context
                .labels
                .insert("nexus.flow.mode".into(), value);
        }
        Agreement::Conflict => {
            summary.context.labels.remove("nexus.flow.mode");
            summary
                .conflicts
                .push("conflicting_invocation_modes".into());
        }
    }
}

fn consumed_result(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    let returns: BTreeSet<_> = events
        .iter()
        .filter_map(|event| match &event.input.body {
            WorkEvent::Milestone { name, .. } if name == "Result consumed" => {
                Some(event.input.occurred_at)
            }
            _ => None,
        })
        .collect();
    match agreement(returns) {
        Agreement::Absent => {}
        Agreement::One(value) => {
            summary
                .context
                .labels
                .insert("nexus.flow.returned_at".into(), value.to_rfc3339());
        }
        Agreement::Conflict => {
            summary.context.labels.remove("nexus.flow.returned_at");
            summary.conflicts.push("conflicting_returns".into());
        }
    }
}
