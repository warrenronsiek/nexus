// @feature audit-registry
// @feature persistence
// @spec docs/features/audit-registry.md
// @spec docs/features/persistence.md
use super::*;

const DETAIL_PAGE: usize = 8;
const RUN_PAGE: usize = 32;

pub(super) fn read(
    connection: &mut DbConnection,
    project: &str,
    action: &AuditAction,
) -> Result<AuditResponse> {
    let mut query = Read {
        connection,
        project,
    };
    match action {
        AuditAction::Control { control, .. } => query.control(control),
        AuditAction::List { filter, .. } => query.list(filter),
        AuditAction::Get {
            run_id,
            after,
            cutoff,
            ..
        } => query.get(run_id, *after, *cutoff),
        AuditAction::Journal {
            after,
            cutoff,
            control_key,
            ..
        } => query.journal(control_key.as_deref(), *after, *cutoff),
        AuditAction::Append { .. } => {
            unreachable!("append is dispatched through the writer transaction")
        }
    }
}

pub(super) fn control_head(
    connection: &mut DbConnection,
    project: &str,
    key: &str,
    cutoff: i64,
) -> Result<Option<JournalRow>> {
    Ok(records::table
        .filter(records::project_id.eq(project))
        .filter(records::control_key.eq(key))
        .filter(records::kind.eq_any(["define", "retire"]))
        .filter(records::ordinal.le(cutoff))
        .order(records::ordinal.desc())
        .select(JournalRow::as_select())
        .first(connection)
        .optional()?)
}

pub(super) fn definition(
    connection: &mut DbConnection,
    project: &str,
    control: &ControlVersion,
    cutoff: i64,
) -> Result<Option<AuditRecord>> {
    records::table
        .filter(records::project_id.eq(project))
        .filter(records::control_key.eq(&control.key))
        .filter(records::control_version.eq(control.version))
        .filter(records::kind.eq("define"))
        .filter(records::ordinal.le(cutoff))
        .select(JournalRow::as_select())
        .first(connection)
        .optional()?
        .map(JournalRow::decode)
        .transpose()
}

pub(super) fn latest_correction(
    connection: &mut DbConnection,
    project: &str,
    run: &str,
    cutoff: i64,
) -> Result<Option<AuditRecord>> {
    records::table
        .filter(records::project_id.eq(project))
        .filter(records::run_id.eq(run))
        .filter(records::kind.eq("correct_run"))
        .filter(records::ordinal.le(cutoff))
        .order(records::ordinal.desc())
        .select(JournalRow::as_select())
        .first(connection)
        .optional()?
        .map(JournalRow::decode)
        .transpose()
}

pub(super) fn latest_disposition(
    connection: &mut DbConnection,
    project: &str,
    run: &str,
    finding: Option<&str>,
) -> Result<Option<AuditRecord>> {
    let mut query = records::table
        .filter(records::project_id.eq(project))
        .filter(records::run_id.eq(run))
        .filter(records::kind.eq("disposition"))
        .into_boxed();
    query = match finding {
        Some(id) => query.filter(records::finding_id.eq(id)),
        None => query.filter(records::finding_id.is_null()),
    };
    query
        .order(records::ordinal.desc())
        .select(JournalRow::as_select())
        .first(connection)
        .optional()?
        .map(JournalRow::decode)
        .transpose()
}

struct Read<'a> {
    connection: &'a mut DbConnection,
    project: &'a str,
}

impl Read<'_> {
    fn cutoff(&mut self, requested: Option<i64>, after: Option<i64>) -> Result<i64> {
        let max = high_water(self.connection, self.project)?;
        let cutoff = requested.unwrap_or(max);
        ensure!(
            (0..=max).contains(&cutoff) && after.is_none_or(|a| (1..=cutoff).contains(&a)),
            invalid("invalid audit cutoff or cursor")
        );
        Ok(cutoff)
    }

    fn control(&mut self, control: &ControlVersion) -> Result<AuditResponse> {
        control.validate().map_err(invalid)?;
        let cutoff = self.cutoff(None, None)?;
        let definition = definition(self.connection, self.project, control, cutoff)?.map(Box::new);
        let retired = diesel::select(diesel::dsl::exists(
            records::table
                .filter(records::project_id.eq(self.project))
                .filter(records::control_key.eq(&control.key))
                .filter(records::control_version.eq(control.version))
                .filter(records::kind.eq("retire"))
                .filter(records::ordinal.le(cutoff)),
        ))
        .get_result(self.connection)?;
        Ok(AuditResponse::Control {
            project_id: self.project.into(),
            definition,
            retired,
        })
    }

    fn list(&mut self, filter: &RunFilter) -> Result<AuditResponse> {
        filter.validate().map_err(invalid)?;
        let cutoff = self.cutoff(filter.cutoff, filter.after)?;
        let total = runs(self.project, filter, cutoff)
            .count()
            .get_result(self.connection)?;
        let mut query = runs(self.project, filter, cutoff);
        if let Some(after) = filter.after {
            query = query.filter(records::ordinal.lt(after));
        }
        let fetched = query
            .order(records::ordinal.desc())
            .select(JournalRow::as_select())
            .limit((RUN_PAGE + 1) as i64)
            .load(self.connection)?;
        let next_cursor = next(&fetched, RUN_PAGE);
        let items = fetched
            .into_iter()
            .take(RUN_PAGE)
            .map(|row| {
                row.decode()
                    .and_then(|record| self.summarize(record, cutoff))
            })
            .collect::<Result<_>>()?;
        Ok(AuditResponse::Page {
            project_id: self.project.into(),
            cutoff,
            total,
            items,
            next_cursor,
        })
    }

    fn summarize(&mut self, record: AuditRecord, cutoff: i64) -> Result<RunSummary> {
        let AuditInput {
            id,
            occurred_at,
            event: AuditEvent::Run { control, run },
        } = record.input
        else {
            unreachable!("run summary uses original run rows")
        };
        let definition = definition(self.connection, self.project, &control, cutoff)?
            .context("stored audit run has no definition")?;
        let correction = latest_correction(self.connection, self.project, &id, cutoff)?;
        let result = correction.as_ref().map_or(&run.result, result_data);
        let category = result_category(definition_data(&definition), result);
        let (disposition, finding_dispositions) =
            disposition_counts(self.dispositions(&id, cutoff)?);
        let open_findings = count_open(result.findings.len(), &finding_dispositions)?;
        Ok(RunSummary {
            id,
            ordinal: record.ordinal,
            control,
            occurred_at,
            subject: run.subject,
            status: result.status,
            duration_ms: run.duration_ms,
            result_code: category.map(|c| c.code.clone()),
            result_label: category.map(|c| c.label.clone()),
            result_order: category.and_then(|c| c.order),
            findings: result.findings.len(),
            open_findings,
            finding_dispositions,
            disposition,
        })
    }

    fn dispositions(&mut self, run: &str, cutoff: i64) -> Result<Vec<AuditRecord>> {
        let latest = diesel::alias!(records as candidates);
        let ordinals = latest
            .filter(latest.field(records::project_id).eq(self.project))
            .filter(latest.field(records::run_id).eq(run))
            .filter(latest.field(records::kind).eq("disposition"))
            .filter(latest.field(records::ordinal).le(cutoff))
            .group_by(latest.field(records::finding_id))
            .select(max(latest.field(records::ordinal)).assume_not_null());
        records::table
            .filter(records::project_id.eq(self.project))
            .filter(records::ordinal.eq_any(ordinals))
            .select(JournalRow::as_select())
            .load(self.connection)?
            .into_iter()
            .map(JournalRow::decode)
            .collect()
    }

    fn get(
        &mut self,
        run_id: &str,
        after: Option<i64>,
        cutoff: Option<i64>,
    ) -> Result<AuditResponse> {
        let cutoff = self.cutoff(cutoff, after)?;
        let definition = self.run_definition(run_id, cutoff)?;
        let (records, next_cursor) = self.journal_page(JournalScope::Run(run_id), after, cutoff)?;
        Ok(AuditResponse::Detail {
            project_id: self.project.into(),
            cutoff,
            definition: Box::new(definition),
            records,
            next_cursor,
        })
    }

    fn run_definition(&mut self, run_id: &str, cutoff: i64) -> Result<AuditRecord> {
        identity(run_id).map_err(invalid)?;
        let base = find(self.connection, self.project, run_id, cutoff)?
            .filter(|row| row.kind == "run")
            .ok_or_else(|| invalid("audit run was not found"))?;
        definition(self.connection, self.project, &base.control(), cutoff)?
            .context("stored audit run has no definition")
    }

    fn journal(
        &mut self,
        control_key: Option<&str>,
        after: Option<i64>,
        cutoff: Option<i64>,
    ) -> Result<AuditResponse> {
        let cutoff = self.cutoff(cutoff, after)?;
        let scope = match control_key {
            Some(key) => {
                identity(key).map_err(invalid)?;
                JournalScope::Control(key)
            }
            None => JournalScope::Project,
        };
        let (records, next_cursor) = self.journal_page(scope, after, cutoff)?;
        Ok(AuditResponse::Journal {
            project_id: self.project.into(),
            cutoff,
            records,
            next_cursor,
        })
    }

    fn journal_page(
        &mut self,
        scope: JournalScope<'_>,
        after: Option<i64>,
        cutoff: i64,
    ) -> Result<(Vec<AuditRecord>, Option<i64>)> {
        let mut query = records::table
            .filter(records::project_id.eq(self.project))
            .filter(records::ordinal.gt(after.unwrap_or(0)))
            .filter(records::ordinal.le(cutoff))
            .into_boxed();
        query = match scope {
            JournalScope::Project => query,
            JournalScope::Run(run) => query.filter(records::run_id.eq(run)),
            JournalScope::Control(key) => query
                .filter(records::control_key.eq(key))
                .filter(records::kind.eq_any(["define", "retire"])),
        };
        let fetched = query
            .order(records::ordinal.asc())
            .select(JournalRow::as_select())
            .limit((DETAIL_PAGE + 1) as i64)
            .load(self.connection)?;
        let cursor = next(&fetched, DETAIL_PAGE);
        let records = fetched
            .into_iter()
            .take(DETAIL_PAGE)
            .map(JournalRow::decode)
            .collect::<Result<_>>()?;
        Ok((records, cursor))
    }
}

enum JournalScope<'a> {
    Project,
    Run(&'a str),
    Control(&'a str),
}

fn runs<'a>(
    project: &'a str,
    filter: &'a RunFilter,
    cutoff: i64,
) -> records::BoxedQuery<'a, crate::persistence::backend::DbBackend> {
    let mut query = records::table
        .filter(records::project_id.eq(project))
        .filter(records::kind.eq("run"))
        .filter(records::ordinal.le(cutoff))
        .into_boxed();
    if !filter.run_ids.is_empty() {
        query = query.filter(records::id.eq_any(&filter.run_ids));
    }
    if let Some(key) = &filter.control_key {
        query = query.filter(records::control_key.eq(key));
    }
    if let Some(version) = filter.version {
        query = query.filter(records::control_version.eq(version));
    }
    if let Some(from) = filter.from {
        query = query.filter(records::occurred_at.ge(timestamp(from)));
    }
    if let Some(until) = filter.until {
        query = query.filter(records::occurred_at.lt(timestamp(until)));
    }
    query
}

fn next(fetched: &[JournalRow], size: usize) -> Option<i64> {
    (fetched.len() > size).then(|| fetched[size - 1].ordinal)
}

fn result_category<'a>(
    definition: &'a ControlDefinition,
    result: &RunResult,
) -> Option<&'a Category> {
    match (&definition.results, &result.output) {
        (ResultSchema::Categorical { codes }, Some(RunOutput::Code(code))) => {
            codes.iter().find(|item| item.code == *code)
        }
        _ => None,
    }
}

type FindingDispositions = std::collections::BTreeMap<Disposition, usize>;

fn disposition_counts(records: Vec<AuditRecord>) -> (Option<Disposition>, FindingDispositions) {
    let mut run = None;
    let mut findings = FindingDispositions::new();
    for record in records {
        if let AuditEvent::Disposition {
            finding_id,
            disposition,
            ..
        } = record.input.event
        {
            if finding_id.is_none() {
                run = Some(disposition);
            } else {
                *findings.entry(disposition).or_insert(0) += 1;
            }
        }
    }
    (run, findings)
}

fn count_open(total: usize, dispositions: &FindingDispositions) -> Result<usize> {
    let disposed = dispositions
        .iter()
        .filter(|(state, _)| **state != Disposition::Reopened)
        .map(|(_, n)| n)
        .sum();
    total
        .checked_sub(disposed)
        .context("invalid audit finding projection")
}
