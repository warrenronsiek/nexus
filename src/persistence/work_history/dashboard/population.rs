// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// Root-scoped dashboard populations read from the rebuildable membership index.
// Projection still verifies each candidate's current membership before it counts.
use super::super::rows::SubjectRow;
use super::SOURCE_LIMIT;
use crate::persistence::{
    backend::DbConnection,
    schema::{work_links, work_subjects},
};
use anyhow::Result;
use diesel::dsl::{count, exists};
use diesel::prelude::*;
use std::collections::{HashMap, HashSet};

/// Host-capture envelopes. A root whose members are only these recorded no work.
pub(super) const CONTAINER_KINDS: [&str; 3] = ["host.session", "request", "host.turn"];
const ROOT_CHUNK: usize = 256;

/// Roots whose membership index contains at least one non-container activity.
pub(super) fn work_bearing_roots(
    connection: &mut DbConnection,
    project: &str,
    roots: &[String],
) -> Result<HashSet<String>> {
    let member = diesel::alias!(work_subjects as member);
    let mut found = HashSet::new();
    for chunk in roots.chunks(ROOT_CHUNK) {
        let work = work_links::table
            .filter(work_links::project_id.eq(work_subjects::project_id))
            .filter(work_links::target_scope.eq("session"))
            .filter(work_links::target_id.eq(work_subjects::id))
            .filter(work_links::relation.eq("member_of"))
            .filter(work_links::source_scope.eq("activity"))
            .filter(exists(
                member
                    .filter(
                        member
                            .field(work_subjects::project_id)
                            .eq(work_links::project_id),
                    )
                    .filter(member.field(work_subjects::scope).eq("activity"))
                    .filter(member.field(work_subjects::id).eq(work_links::source_id))
                    .filter(member.field(work_subjects::kind).ne_all(CONTAINER_KINDS)),
            ));
        found.extend(
            work_subjects::table
                .filter(work_subjects::project_id.eq(project))
                .filter(work_subjects::scope.eq("session"))
                .filter(work_subjects::id.eq_any(chunk))
                .filter(exists(work))
                .select(work_subjects::id)
                .load::<String>(connection)?,
        );
    }
    Ok(found)
}

/// Activities sampled whole roots at a time, newest root first, so every included
/// session contributes all of its activity kinds rather than only its newest rows.
pub(super) struct MemberSample {
    pub rows: Vec<SubjectRow>,
    pub truncated: bool,
}

pub(super) fn member_sample(
    connection: &mut DbConnection,
    project: &str,
    roots: &[String],
) -> Result<MemberSample> {
    let (mut chosen, truncated) = whole_roots(connection, project, roots)?;
    if chosen.is_empty() {
        // The newest root alone exceeds the budget: keep its most recent activities.
        chosen.extend(roots.first().cloned());
    }
    let mut rows = UniqueRows::default();
    for chunk in chosen.chunks(ROOT_CHUNK) {
        rows.extend(member_rows(connection, project, chunk)?);
    }
    let mut rows = rows.rows;
    rows.truncate(SOURCE_LIMIT);
    Ok(MemberSample { rows, truncated })
}

/// Shared activities belong to several roots but are counted once per identity.
#[derive(Default)]
struct UniqueRows {
    rows: Vec<SubjectRow>,
    seen: HashSet<String>,
}

impl UniqueRows {
    fn extend(&mut self, rows: Vec<SubjectRow>) {
        for row in rows {
            if self.seen.insert(row.id.clone()) {
                self.rows.push(row);
            }
        }
    }
}

fn whole_roots(
    connection: &mut DbConnection,
    project: &str,
    roots: &[String],
) -> Result<(Vec<String>, bool)> {
    let mut chosen = Vec::new();
    let mut total = 0usize;
    for chunk in roots.chunks(ROOT_CHUNK) {
        let counts: HashMap<String, i64> = links(project)
            .filter(work_links::target_id.eq_any(chunk))
            .group_by(work_links::target_id)
            .select((
                work_links::target_id,
                count(work_links::source_id).aggregate_distinct(),
            ))
            .load::<(String, i64)>(connection)?
            .into_iter()
            .collect();
        for root in chunk {
            let count = counts.get(root).copied().unwrap_or(0) as usize;
            if total + count > SOURCE_LIMIT {
                return Ok((chosen, true));
            }
            total += count;
            chosen.push(root.clone());
        }
    }
    Ok((chosen, false))
}

/// Request activities (prompt submission to host response) for the given roots.
pub(super) fn request_rows(
    connection: &mut DbConnection,
    project: &str,
    roots: &[String],
) -> Result<(Vec<SubjectRow>, bool)> {
    let mut rows = UniqueRows::default();
    for chunk in roots.chunks(ROOT_CHUNK) {
        rows.extend(
            links(project)
                .filter(work_links::target_id.eq_any(chunk))
                .inner_join(member_join())
                .filter(work_subjects::kind.eq("request"))
                .select(SubjectRow::as_select())
                .distinct()
                .load::<SubjectRow>(connection)?,
        );
        if rows.rows.len() > SOURCE_LIMIT {
            rows.rows.truncate(SOURCE_LIMIT);
            return Ok((rows.rows, true));
        }
    }
    Ok((rows.rows, false))
}

/// At most one source window of member activities, most recently received first.
fn member_rows(
    connection: &mut DbConnection,
    project: &str,
    roots: &[String],
) -> Result<Vec<SubjectRow>> {
    Ok(links(project)
        .filter(work_links::target_id.eq_any(roots))
        .inner_join(member_join())
        .select(SubjectRow::as_select())
        .distinct()
        .order((work_subjects::recorded_at.desc(), work_subjects::id.asc()))
        .limit(SOURCE_LIMIT as i64)
        .load(connection)?)
}

/// Activity-to-session membership edges in one project.
#[diesel::dsl::auto_type]
fn links<'a>(project: &'a str) -> _ {
    work_links::table
        .filter(work_links::project_id.eq(project))
        .filter(work_links::relation.eq("member_of"))
        .filter(work_links::target_scope.eq("session"))
        .filter(work_links::source_scope.eq("activity"))
}

/// Joins a membership edge to its member activity's indexed attributes.
#[diesel::dsl::auto_type]
fn member_join() -> _ {
    let activity: &'static str = "activity";
    work_subjects::table.on(work_subjects::project_id
        .eq(work_links::project_id)
        .and(work_subjects::scope.eq(activity))
        .and(work_subjects::id.eq(work_links::source_id)))
}
