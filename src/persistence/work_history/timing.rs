// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::work_history::*;

pub(super) fn project_events(summary: &mut WorkSummary, events: &[&WorkObservation]) {
    declaration(summary, events);
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
    wall_clock_fallback(summary, &starts, &finishes);
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
                context,
                ..
            } => {
                summary.title = name.clone();
                summary.kind = kind.clone();
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
    if starts
        .windows(2)
        .any(|pair| pair[0].input.occurred_at != pair[1].input.occurred_at)
    {
        summary.conflicts.push("conflicting_starts".into());
        summary.started_at = None;
    } else if let Some(start) = starts.first() {
        summary.started_at = Some(start.input.occurred_at);
    }
}
fn finish(summary: &mut WorkSummary, finishes: &[&WorkObservation]) {
    if finishes.windows(2).any(|pair| {
        pair[0].input.body != pair[1].input.body
            || pair[0].input.occurred_at != pair[1].input.occurred_at
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
) {
    if summary.elapsed.is_some() {
        return;
    }
    let (Some(start), Some(finish)) = (summary.started_at, summary.finished_at) else {
        return;
    };
    let (Some(start_source), Some(finish_source)) = (starts.first(), finishes.first()) else {
        return;
    };
    if start_source.input.producer != finish_source.input.producer {
        summary.gaps.push("clock_domain_mismatch".into());
        return;
    }
    if let Some(nanoseconds) = finish
        .signed_duration_since(start)
        .num_nanoseconds()
        .filter(|duration| *duration >= 0)
    {
        summary.elapsed = Some(Elapsed {
            nanoseconds: nanoseconds.to_string(),
            method: "wall_clock".into(),
            clock_id: Some(start_source.input.producer.instance_id.clone()),
        });
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
