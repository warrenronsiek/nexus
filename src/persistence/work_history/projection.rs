// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::corrections::{effective, Diagnostics};
use super::lineage::Lineage;
use super::rows::ObservationRow;
use super::timing::project_events;
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{eval_records, work_links, work_observations, work_subjects};
use crate::work_history::*;
use anyhow::Result;
use diesel::prelude::*;

const PROJECTION_LIMIT: i64 = 1024;

pub(super) fn observed(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
) -> Result<(Vec<WorkObservation>, bool)> {
    let mut rows = work_observations::table
        .filter(work_observations::project_id.eq(project))
        .filter(work_observations::subject_scope.eq(subject.scope()))
        .filter(work_observations::subject_id.eq(subject.id()))
        .order(work_observations::id.asc())
        .limit(PROJECTION_LIMIT + 1)
        .select(ObservationRow::as_select())
        .load::<ObservationRow>(connection)?;
    let truncated = rows.len() > PROJECTION_LIMIT as usize;
    rows.truncate(PROJECTION_LIMIT as usize);
    Ok((
        rows.iter()
            .map(ObservationRow::observation)
            .collect::<Result<_>>()?,
        truncated,
    ))
}

pub(super) fn active_links(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
) -> Result<(Vec<WorkLink>, Diagnostics)> {
    let (observations, truncated) = observed(connection, project, subject)?;
    let mut diagnostics = Diagnostics::default();
    let active = effective(connection, project, &observations, &mut diagnostics)?;
    let mut links = vec![];
    for item in active {
        for (relation, target) in item.input.body.references() {
            let state = if target_exists(connection, project, &target)? {
                "resolved"
            } else {
                "unresolved"
            };
            if state == "unresolved" {
                diagnostics.gaps.push("unresolved_link".into());
            }
            links.push(WorkLink {
                event_id: item.input.id.clone(),
                source: subject.clone(),
                relation: relation.as_str().into(),
                target,
                state: state.into(),
            });
        }
    }
    if truncated {
        diagnostics.gaps.push("projection_limit".into());
    }
    Ok((links, diagnostics))
}

fn target_exists(
    connection: &mut DbConnection,
    project: &str,
    target: &WorkReference,
) -> Result<bool> {
    if let Some(subject) = target.subject() {
        return Ok(
            diesel::select(diesel::dsl::exists(work_subjects::table.find((
                project,
                subject.scope(),
                subject.id(),
            ))))
            .get_result(connection)?,
        );
    }
    if let WorkReference::Eval { id } = target {
        return Ok(
            diesel::select(diesel::dsl::exists(eval_records::table.find((project, id))))
                .get_result(connection)?,
        );
    }
    Ok(true)
}

pub(super) fn detail(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
) -> Result<Option<WorkDetail>> {
    let (observations, projection_truncated) = observed(connection, project, subject)?;
    if observations.is_empty() {
        return Ok(None);
    }
    let mut summary = summarize(connection, project, subject, &observations)?;
    if projection_truncated {
        summary.disclose_limit();
    }
    summary.normalize_diagnostics();
    let (links, _) = active_links(connection, project, subject)?;
    bound_evidence(summary, observations, links, projection_truncated).map(Some)
}
fn bound_evidence(
    summary: WorkSummary,
    mut observations: Vec<WorkObservation>,
    mut links: Vec<WorkLink>,
    projection_truncated: bool,
) -> Result<WorkDetail> {
    let links_truncated = super::paging::bound_items(&mut links, 128, 128 * 1024)?;
    let observations_truncated = super::paging::bound_items(&mut observations, 64, 512 * 1024)?;
    Ok(WorkDetail {
        summary,
        observations,
        links,
        truncated: projection_truncated || links_truncated || observations_truncated,
    })
}

fn summarize(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
    observations: &[WorkObservation],
) -> Result<WorkSummary> {
    let mut diagnostics = Diagnostics::default();
    let active = effective(connection, project, observations, &mut diagnostics)?;
    let lineage = Lineage::resolve(connection, project, subject)?;
    diagnostics.merge(lineage.diagnostics);
    let mut summary = WorkSummary::unobserved(subject);
    summary.work_session_ids = lineage.sessions;
    summary.parent_activity_id = lineage.parent;
    summary.conflicts = diagnostics.conflicts;
    summary.gaps = diagnostics.gaps;
    project_events(&mut summary, &active);
    include_root_membership(&mut summary);
    summary.has_children = has_children(connection, project, subject)?;
    Ok(summary)
}
fn include_root_membership(summary: &mut WorkSummary) {
    if matches!(summary.subject, WorkSubject::Session { .. })
        && !summary
            .work_session_ids
            .iter()
            .any(|id| id == summary.subject.id())
    {
        summary.work_session_ids.push(summary.subject.id().into());
    }
}
fn has_children(
    connection: &mut DbConnection,
    project: &str,
    subject: &WorkSubject,
) -> Result<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        work_links::table
            .filter(work_links::project_id.eq(project))
            .filter(work_links::target_scope.eq(subject.scope()))
            .filter(work_links::target_id.eq(subject.id()))
            .filter(work_links::relation.eq(if subject.scope() == "session" {
                "member_of"
            } else {
                "parent"
            })),
    ))
    .get_result(connection)?)
}
