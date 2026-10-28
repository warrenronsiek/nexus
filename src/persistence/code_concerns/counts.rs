// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use crate::code_concerns::*;
use crate::persistence::backend::{DbBackend, DbConnection};
use crate::persistence::schema::{
    code_concerns as c, concern_areas as areas, concern_attributions as a, concern_encounters as e,
};
use anyhow::Result;
use diesel::dsl::{count, count_star};
use diesel::prelude::*;

pub(super) struct Population<'a> {
    pub project: &'a str,
    pub query: &'a ConcernQuery,
    pub concern: Option<&'a str>,
    pub area: Option<&'a str>,
}
enum Qualification {
    UnknownSource,
    UnknownWork,
    UnknownHost,
    UnknownSourceTime,
    Returned,
}
struct CoverageCounts {
    source: usize,
    work: usize,
    host: usize,
    source_time: usize,
    returned: usize,
}
impl Population<'_> {
    pub fn query(&self) -> e::BoxedQuery<'_, DbBackend> {
        let mut query = e::table.filter(e::project_id.eq(self.project)).into_boxed();
        if let Some(id) = self.concern {
            query = query.filter(e::concern_id.eq(id))
        }
        if let Some(repo) = &self.query.repository {
            query = query.filter(
                e::repository
                    .eq(repo)
                    .or(e::event_id.eq_any(self.repository_areas(repo).select(a::event_id))),
            )
        }
        if let Some(time) = self.query.occurred_after {
            query = query.filter(e::effective_time.ge(stamp(time)))
        }
        if let Some(time) = self.query.occurred_before {
            query = query.filter(e::effective_time.lt(stamp(time)))
        }
        if let Some(area) = self.area.or(self.query.area_id.as_deref()) {
            query = query.filter(
                e::event_id.eq_any(
                    a::table
                        .filter(a::project_id.eq(self.project))
                        .filter(a::area_id.eq(area))
                        .select(a::event_id),
                ),
            )
        }
        query.filter(e::concern_id.eq_any(self.concern_selection().select(c::id)))
    }
    fn repository_areas<'a>(&'a self, repository: &'a str) -> a::BoxedQuery<'a, DbBackend> {
        a::table
            .filter(a::project_id.eq(self.project))
            .filter(
                a::area_id.eq_any(
                    areas::table
                        .filter(areas::project_id.eq(self.project))
                        .filter(areas::repository.eq(repository))
                        .select(areas::id),
                ),
            )
            .into_boxed()
    }
    fn concern_selection(&self) -> c::BoxedQuery<'_, DbBackend> {
        let mut concerns = c::table.filter(c::project_id.eq(self.project)).into_boxed();
        if let Some(status) = self.query.status {
            concerns = concerns.filter(c::status.eq(query_status_name(status)))
        }
        if let Some(search) = &self.query.search {
            concerns = concerns.filter(c::title.like(format!("%{search}%")))
        }
        concerns
    }
    pub fn counts(&self, connection: &mut DbConnection) -> Result<ConcernCounts> {
        let (raw, source, work, host, concerns) = self
            .query()
            .select((
                count_star(),
                count(e::source_key).aggregate_distinct(),
                count(e::work_session_id).aggregate_distinct(),
                count(e::host_session_key).aggregate_distinct(),
                count(e::concern_id).aggregate_distinct(),
            ))
            .first::<(i64, i64, i64, i64, i64)>(connection)?;
        let coverage = self.coverage(connection)?;
        Ok(ConcernCounts {
            raw_observations: raw as usize,
            known_source_occurrences: source as usize,
            work_sessions: work as usize,
            host_sessions: host as usize,
            concerns: concerns as usize,
            unknown_source_observations: coverage.source,
            unknown_work_observations: coverage.work,
            unknown_host_observations: coverage.host,
            unknown_source_time_observations: coverage.source_time,
            returns_after_resolution: coverage.returned,
        })
    }
    fn coverage(&self, connection: &mut DbConnection) -> Result<CoverageCounts> {
        Ok(CoverageCounts {
            source: self.qualified_count(connection, Qualification::UnknownSource)?,
            work: self.qualified_count(connection, Qualification::UnknownWork)?,
            host: self.qualified_count(connection, Qualification::UnknownHost)?,
            source_time: self.qualified_count(connection, Qualification::UnknownSourceTime)?,
            returned: self.qualified_count(connection, Qualification::Returned)?,
        })
    }
    fn qualified_count(
        &self,
        connection: &mut DbConnection,
        qualification: Qualification,
    ) -> Result<usize> {
        let query = self.query();
        let qualified = match qualification {
            Qualification::UnknownSource => query.filter(e::source_key.is_null()),
            Qualification::UnknownWork => query.filter(e::work_session_id.is_null()),
            Qualification::UnknownHost => query.filter(e::host_session_key.is_null()),
            Qualification::UnknownSourceTime => query.filter(e::source_time.is_null()),
            Qualification::Returned => query.filter(e::returned_after_resolution.eq(1)),
        };
        Ok(qualified.count().get_result::<i64>(connection)? as usize)
    }
    pub fn daily(&self, connection: &mut DbConnection) -> Result<Vec<ConcernDaily>> {
        let rows = e::table
            .filter(e::project_id.eq(self.project))
            .filter(e::event_id.eq_any(self.query().select(e::event_id)))
            .group_by(e::effective_day)
            .select((
                e::effective_day,
                count_star(),
                count(e::source_key).aggregate_distinct(),
                count(e::work_session_id).aggregate_distinct(),
            ))
            .order(e::effective_day.desc())
            .limit(367)
            .load::<(String, i64, i64, i64)>(connection)?;
        Ok(rows
            .into_iter()
            .map(|(date, raw, source, work)| ConcernDaily {
                date,
                raw_observations: raw as usize,
                known_source_occurrences: source as usize,
                work_sessions: work as usize,
            })
            .collect())
    }
}
pub(super) fn stamp(time: chrono::DateTime<chrono::Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}
pub(super) fn status_name(status: ConcernStatus) -> &'static str {
    match status {
        ConcernStatus::Open => "open",
        ConcernStatus::Resolved => "resolved",
        ConcernStatus::Consolidated => "consolidated",
    }
}

fn query_status_name(status: ConcernQueryStatus) -> &'static str {
    match status {
        ConcernQueryStatus::Open => "open",
        ConcernQueryStatus::Resolved => "resolved",
    }
}
