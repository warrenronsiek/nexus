// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::work_history::*;

pub(super) fn project_events(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    declaration(summary, events);
    super::metadata::project(summary, events);
    let starts = events
        .iter()
        .copied()
        .filter(|item| matches!(item.input.body, WorkEvent::Started))
        .collect::<Vec<_>>();
    let finishes = events
        .iter()
        .copied()
        .filter(|item| matches!(item.input.body, WorkEvent::Finished { .. }))
        .collect::<Vec<_>>();
    start(summary, &starts);
    finish(summary, &finishes);
    wall_clock_fallback(summary, &starts, &finishes, events);
    queue(summary, events, &starts);
    disclose_gaps(summary, finishes.is_empty());
    summary.state = state(summary, events).into();
}
fn declaration(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    let declarations = events
        .iter()
        .filter(|item| {
            matches!(
                item.input.body,
                WorkEvent::SessionOpened { .. } | WorkEvent::ActivityDeclared { .. }
            )
        })
        .collect::<Vec<_>>();
    if let Some(item) = declarations.first() {
        match &item.input.body {
            WorkEvent::SessionOpened { title, context } => {
                summary.title = title.clone();
                summary.context = context.clone();
                summary.started_at = Some(item.input.occurred_at);
            }
            WorkEvent::ActivityDeclared {
                name,
                kind,
                executor,
                context,
                ..
            } => {
                summary.title = name.clone();
                summary.kind = kind.clone();
                summary.executor = executor.clone();
                summary.context = context.clone();
            }
            _ => {}
        }
        if summary.context.host_session_id.is_none() {
            let actor = item
                .source
                .as_ref()
                .map_or(&item.provenance, |source| &source.provenance);
            summary.context.host_session_id = Some(actor.session_id.clone());
        }
    } else {
        summary.gaps.push("declaration_missing".into());
    }
    if declarations
        .windows(2)
        .any(|pair| pair[0].input.body != pair[1].input.body)
    {
        summary.conflicts.push("conflicting_declarations".into());
    }
}
fn start(summary: &mut WorkSummary, starts: &[&WorkObservation]) {
    if super::clock::competing_boundaries(starts) {
        summary.conflicts.push("conflicting_starts".into());
        summary.started_at = None;
    } else if let Some(start) = starts.first() {
        summary.started_at = Some(start.input.occurred_at);
    }
}
fn finish(summary: &mut WorkSummary, finishes: &[&WorkObservation]) {
    if finishes.windows(2).any(|pair| {
        pair[0].input.body != pair[1].input.body || !super::clock::same_boundary(pair[0], pair[1])
    }) {
        summary.conflicts.push("conflicting_finishes".into());
    } else if let Some(finish) = finishes.first() {
        if let WorkEvent::Finished {
            outcome, elapsed, ..
        } = &finish.input.body
        {
            summary.outcome = Some(*outcome);
            summary.elapsed = elapsed.clone();
            summary.finished_at = Some(finish.input.occurred_at);
        }
    }
}
fn disclose_gaps(summary: &mut WorkSummary, finish_missing: bool) {
    if summary.started_at.is_none() {
        summary.gaps.push("start_missing".into());
    }
    if finish_missing {
        summary.gaps.push("finish_missing".into());
    }
    if let (Some(start), Some(finish)) = (summary.started_at, summary.finished_at) {
        if finish < start {
            summary.gaps.push("wall_clock_anomaly".into());
            if summary.elapsed.is_none() {
                summary.conflicts.push("invalid_wall_clock_interval".into());
            }
        }
    }
}
fn wall_clock_fallback(
    summary: &mut WorkSummary,
    starts: &[&WorkObservation],
    finishes: &[&WorkObservation],
    events: &[&WorkObservation],
) {
    if summary.elapsed.is_some() {
        return;
    }
    let (Some(_), Some(_)) = (summary.started_at, summary.finished_at) else {
        return;
    };
    let start_source = starts.first().copied().or_else(|| {
        events
            .iter()
            .copied()
            .find(|item| matches!(item.input.body, WorkEvent::SessionOpened { .. }))
    });
    let (Some(start_source), Some(finish_source)) = (start_source, finishes.first()) else {
        return;
    };
    match super::clock::elapsed(start_source, finish_source) {
        Ok(elapsed) => summary.elapsed = Some(elapsed),
        Err(gap) => {
            summary.gaps.push(gap.code().into());
            if let Some(conflict) = gap.conflict() {
                summary.conflicts.push(conflict.into());
            }
        }
    }
}

fn state(summary: &WorkSummary, events: &[&WorkObservation]) -> &'static str {
    if !summary.conflicts.is_empty() {
        "conflicted"
    } else if summary.outcome.is_some() {
        "finished"
    } else if summary.started_at.is_some() {
        "running"
    } else if events
        .iter()
        .any(|item| matches!(item.input.body, WorkEvent::Queued))
    {
        "queued"
    } else {
        "unknown"
    }
}

fn queue(summary: &mut WorkSummary, events: &[&WorkObservation], starts: &[&WorkObservation]) {
    let queued = events
        .iter()
        .copied()
        .filter(|item| matches!(item.input.body, WorkEvent::Queued))
        .collect::<Vec<_>>();
    if super::clock::competing_boundaries(&queued) {
        summary.conflicts.push("conflicting_queues".into());
        return;
    }
    let Some(source) = queued.first() else {
        return;
    };
    summary.queued_at = Some(source.input.occurred_at);
    let Some(start) = starts.first().filter(|_| summary.started_at.is_some()) else {
        return;
    };
    match super::clock::elapsed(source, start) {
        Ok(elapsed) => summary.queue_elapsed = Some(elapsed),
        Err(gap) => summary.gaps.push(format!("queue_{}", gap.code())),
    }
    if start.input.occurred_at < source.input.occurred_at {
        summary.gaps.push("queue_wall_clock_anomaly".into());
    }
}
