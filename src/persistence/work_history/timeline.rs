// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::{
    paging::{bound_items, Page},
    projection,
    rows::SubjectRow,
};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{work_links, work_subjects};
use crate::work_history::*;
use anyhow::{bail, Result};
use diesel::prelude::*;

pub(super) struct TimelineRequest<'a> {
    pub session: &'a str,
    pub parent: Option<&'a str>,
    pub after: Option<&'a str>,
    pub limit: Option<u32>,
}
struct Timeline<'a> {
    project: &'a str,
    request: TimelineRequest<'a>,
    page: Page<WorkSummary>,
    unresolved: Vec<WorkLink>,
    truncated: bool,
}
pub(super) fn timeline(
    connection: &mut DbConnection,
    project: &str,
    request: TimelineRequest<'_>,
) -> Result<WorkResponse> {
    let mut timeline = Timeline::open(project, request)?;
    let mut rows = timeline.rows(connection)?;
    let more = rows.len() > timeline.page.limit as usize;
    rows.truncate(timeline.page.limit as usize);
    let mut reader = projection::Projection::new(connection, project);
    reader.prefetch(
        &rows
            .iter()
            .map(|row| WorkSubject::Activity { id: row.id.clone() })
            .collect::<Vec<_>>(),
    )?;
    for row in rows {
        if !timeline.include(&mut reader, row)? {
            break;
        }
    }
    timeline.response(more)
}
impl<'a> Timeline<'a> {
    fn open(project: &'a str, request: TimelineRequest<'a>) -> Result<Self> {
        if request.session.len() > 512 || request.parent.is_some_and(|id| id.len() > 512) {
            bail!("work timeline identity exceeds limit");
        }
        let page = Page::open(
            request.after,
            format!(
                "timeline:{project}:{}:{:?}",
                request.session, request.parent
            ),
            request.limit,
        )?;
        Ok(Self {
            project,
            request,
            page,
            unresolved: vec![],
            truncated: false,
        })
    }
    fn rows(&self, connection: &mut DbConnection) -> Result<Vec<SubjectRow>> {
        let (scope, target, relation) = match self.request.parent {
            Some(parent) => ("activity", parent, "parent"),
            None => ("session", self.request.session, "member_of"),
        };
        let sources = work_links::table
            .filter(work_links::project_id.eq(self.project))
            .filter(work_links::target_scope.eq(scope))
            .filter(work_links::target_id.eq(target))
            .filter(work_links::relation.eq(relation))
            .select(work_links::source_id);
        Ok(work_subjects::table
            .filter(work_subjects::project_id.eq(self.project))
            .filter(work_subjects::scope.eq("activity"))
            .filter(work_subjects::id.eq_any(sources))
            .filter(work_subjects::id.gt(&self.page.cursor.after))
            .filter(work_subjects::recorded_at.le(self.page.cursor.cutoff.to_rfc3339()))
            .order(work_subjects::id.asc())
            .limit(self.page.limit + 1)
            .select(SubjectRow::as_select())
            .load(connection)?)
    }
    fn include(
        &mut self,
        reader: &mut projection::Projection<'_>,
        row: SubjectRow,
    ) -> Result<bool> {
        let detail = reader.detail(&WorkSubject::Activity { id: row.id.clone() })?;
        let item = match detail {
            Some(detail) if self.visible(reader, &detail.summary)? => {
                self.unresolved.extend(
                    detail
                        .links
                        .into_iter()
                        .filter(|link| link.state == "unresolved"),
                );
                self.truncated |= bound_items(&mut self.unresolved, 64, 96 * 1024)?;
                Some(detail.summary)
            }
            _ => None,
        };
        self.page.include(row.id, item)
    }
    fn visible(
        &self,
        reader: &mut projection::Projection<'_>,
        summary: &WorkSummary,
    ) -> Result<bool> {
        if !summary
            .work_session_ids
            .iter()
            .any(|id| id == self.request.session)
        {
            return Ok(false);
        }
        if let Some(parent) = self.request.parent {
            return Ok(summary.parent_activity_id.as_deref() == Some(parent));
        }
        match &summary.parent_activity_id {
            None => Ok(true),
            Some(parent) => {
                let parent = reader.summary(&WorkSubject::Activity { id: parent.clone() })?;
                Ok(!parent.is_some_and(|detail| {
                    detail
                        .work_session_ids
                        .iter()
                        .any(|id| id == self.request.session)
                }))
            }
        }
    }
    fn response(self, more: bool) -> Result<WorkResponse> {
        let page = self.page.finish(more)?;
        Ok(WorkResponse::Timeline {
            work_session_id: self.request.session.into(),
            parent_activity_id: self.request.parent.map(str::to_owned),
            items: page.items,
            next_cursor: page.next_cursor,
            unresolved_links: self.unresolved,
            truncated: self.truncated || page.truncated,
        })
    }
}
