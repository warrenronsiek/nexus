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
use std::collections::HashMap;

mod prefetch;

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

/// A project-scoped read retains bounded lineage links only for this operation.
/// It never spans writes, requests, or projects; uncached reads preserve the same evidence.
pub(super) struct Projection<'a> {
    connection: &'a mut DbConnection,
    project: &'a str,
    links: HashMap<WorkSubject, (Vec<WorkLink>, Diagnostics)>,
    bytes: usize,
    known: HashMap<WorkSubject, bool>,
    children: HashMap<WorkSubject, bool>,
    observations: HashMap<WorkSubject, Vec<WorkObservation>>,
}
impl<'a> Projection<'a> {
    fn links_from_observations(
        &mut self,
        subject: &WorkSubject,
        observations: &[WorkObservation],
        truncated: bool,
    ) -> Result<(Vec<WorkLink>, Diagnostics)> {
        let mut diagnostics = Diagnostics::default();
        let active = effective(
            self.connection,
            self.project,
            observations,
            &mut diagnostics,
        )?;
        let mut links = vec![];
        for item in active {
            for (relation, target) in item.input.body.references() {
                let state = if self.target_exists(&target)? {
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

    pub fn new(connection: &'a mut DbConnection, project: &'a str) -> Self {
        Self {
            connection,
            project,
            links: HashMap::new(),
            bytes: 0,
            known: HashMap::new(),
            children: HashMap::new(),
            observations: HashMap::new(),
        }
    }
    fn observed(&mut self, subject: &WorkSubject) -> Result<(Vec<WorkObservation>, bool)> {
        if let Some(observations) = self.observations.get(subject) {
            return Ok((observations.clone(), false));
        }
        observed(self.connection, self.project, subject)
    }
    fn target_exists(&mut self, target: &WorkReference) -> Result<bool> {
        let Some(subject) = target.subject() else {
            return target_exists(self.connection, self.project, target);
        };
        if let Some(exists) = self.known.get(&subject) {
            return Ok(*exists);
        }
        let exists = target_exists(self.connection, self.project, target)?;
        let bytes = serde_json::to_vec(&subject)?.len() + 1;
        if self.known.len() < 4096 && self.bytes + bytes <= 8 * 1024 * 1024 {
            self.bytes += bytes;
            self.known.insert(subject, exists);
        }
        Ok(exists)
    }
    pub fn active_links(&mut self, subject: &WorkSubject) -> Result<(Vec<WorkLink>, Diagnostics)> {
        if let Some(cached) = self.links.get(subject) {
            return Ok(cached.clone());
        }
        let (observations, truncated) = self.observed(subject)?;
        self.subject_links(subject, &observations, truncated)
    }
    fn subject_links(
        &mut self,
        subject: &WorkSubject,
        observations: &[WorkObservation],
        truncated: bool,
    ) -> Result<(Vec<WorkLink>, Diagnostics)> {
        if let Some(cached) = self.links.get(subject) {
            return Ok(cached.clone());
        }
        let result = self.links_from_observations(subject, observations, truncated)?;
        let bytes = serde_json::to_vec(&(&result.0, &result.1.conflicts, &result.1.gaps))?.len();
        if self.links.len() < 4096 && self.bytes + bytes <= 8 * 1024 * 1024 {
            self.links.insert(subject.clone(), result.clone());
            self.bytes += bytes;
        }
        Ok(result)
    }
    pub fn summary(&mut self, subject: &WorkSubject) -> Result<Option<WorkSummary>> {
        Ok(self.project(subject)?.map(|record| record.summary))
    }
    pub fn detail(&mut self, subject: &WorkSubject) -> Result<Option<WorkDetail>> {
        self.project(subject)?
            .map(|record| {
                bound_evidence(
                    record.summary,
                    record.observations,
                    record.links,
                    record.truncated,
                )
            })
            .transpose()
    }
    fn project(&mut self, subject: &WorkSubject) -> Result<Option<WorkDetail>> {
        let (observations, truncated) = self.observed(subject)?;
        if observations.is_empty() {
            return Ok(None);
        }
        let (links, _) = self.subject_links(subject, &observations, truncated)?;
        let mut summary = self.summarize(subject, &observations)?;
        if truncated {
            summary.disclose_limit();
        }
        summary.normalize_diagnostics();
        Ok(Some(WorkDetail {
            summary,
            observations,
            links,
            truncated,
        }))
    }
    fn summarize(
        &mut self,
        subject: &WorkSubject,
        observations: &[WorkObservation],
    ) -> Result<WorkSummary> {
        let mut diagnostics = Diagnostics::default();
        let active = effective(
            self.connection,
            self.project,
            observations,
            &mut diagnostics,
        )?;
        let lineage = Lineage::resolve(self, subject)?;
        diagnostics.merge(lineage.diagnostics);
        let mut summary = WorkSummary::unobserved(subject);
        summary.work_session_ids = lineage.sessions;
        summary.parent_activity_id = lineage.parent;
        summary.conflicts = diagnostics.conflicts;
        summary.gaps = diagnostics.gaps;
        project_events(&mut summary, &active);
        include_root_membership(&mut summary);
        summary.has_children = match self.children.get(subject) {
            Some(value) => *value,
            None => has_children(self.connection, self.project, subject)?,
        };
        Ok(summary)
    }
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
