// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @boundary work-accounting-metadata: only recognized milestone names and bounded reserved values qualify projected titles, roles, source gaps, and executor identities; originals remain immutable.
use crate::work_history::*;
use std::collections::BTreeSet;

pub(super) fn project(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    session_title(summary, events);
    capture_gaps(summary, events);
    accounting_role(summary, events);
    executor(summary, events);
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
    match roles.len() {
        0 => {}
        1 => {
            summary.context.labels.insert(
                "nexus.accounting.role".into(),
                roles.into_iter().next().expect("one role"),
            );
        }
        _ => {
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
    if identities.len() > 1 {
        summary.executor = None;
        summary.conflicts.push("conflicting_executors".into());
    } else {
        summary.executor = identities.into_iter().next();
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
    }
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
