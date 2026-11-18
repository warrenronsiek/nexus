// @feature audit-registry
// @feature persistence
// @spec docs/features/audit-registry.md
// @spec docs/features/persistence.md
// @entrypoint Store::audit_command
mod queries;

use super::{
    backend::DbConnection, schema::audit_records as records,
    transaction::prepared_immediate_transaction, Store,
};
use crate::{audit::*, work_history::WorkActor};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use diesel::{dsl::max, prelude::*};

#[derive(Insertable, Queryable, Selectable)]
#[diesel(table_name = records)]
struct JournalRow {
    project_id: String,
    id: String,
    ordinal: i64,
    kind: String,
    control_key: String,
    control_version: i32,
    run_id: Option<String>,
    finding_id: Option<String>,
    occurred_at: String,
    recorded_at: String,
    actor_json: String,
    input_json: String,
}

impl JournalRow {
    fn decode(self) -> Result<AuditRecord> {
        Ok(AuditRecord {
            ordinal: self.ordinal,
            recorded_at: self.recorded_at.parse()?,
            actor: serde_json::from_str(&self.actor_json)?,
            input: serde_json::from_str(&self.input_json)?,
        })
    }

    fn control(&self) -> ControlVersion {
        ControlVersion {
            key: self.control_key.clone(),
            version: self.control_version,
        }
    }
}

impl Store {
    pub fn audit_command(
        &mut self,
        project: &str,
        actor: &WorkActor,
        action: &AuditAction,
    ) -> Result<AuditResponse> {
        let project = crate::config::canonical_project_id(project)
            .map_err(|_| invalid("invalid project identity"))?;
        match action {
            AuditAction::Append {
                record,
                expected_project,
            } => {
                ensure!(
                    expected_project
                        .as_deref()
                        .is_none_or(|expected| expected == project),
                    invalid("audit record is bound to a different project")
                );
                prepared_immediate_transaction(
                    &mut self.connection,
                    record,
                    |input| prepare(input, actor),
                    |connection, payload| append(connection, project, actor, record, payload),
                )
            }
            _ => {
                // These reads use an ordinal fence; later appends are invisible without locking writers.
                queries::read(&mut self.connection, project, action)
            }
        }
    }
}

fn timestamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn invalid(error: impl std::fmt::Display) -> AuditRejection {
    AuditRejection::Invalid(error.to_string())
}

fn conflict(message: &str) -> AuditRejection {
    AuditRejection::Conflict(message.to_owned())
}

fn prepare(record: &AuditInput, actor: &WorkActor) -> Result<String> {
    record.validate().map_err(invalid)?;
    actor.validate().map_err(invalid)?;
    let payload = serde_json::to_string(record)?;
    ensure!(
        payload.len() <= MAX_RECORD_BYTES,
        invalid("audit record exceeds 48 KiB")
    );
    Ok(payload)
}

fn high_water(connection: &mut DbConnection, project: &str) -> Result<i64> {
    Ok(records::table
        .filter(records::project_id.eq(project))
        .select(max(records::ordinal))
        .first::<Option<i64>>(connection)?
        .unwrap_or(0))
}

fn append(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    input: &AuditInput,
    payload: &str,
) -> Result<AuditResponse> {
    let row = match find(connection, project, &input.id, i64::MAX)? {
        Some(row) => {
            ensure!(
                row.input_json == payload,
                conflict("audit ID already has different content")
            );
            row
        }
        None => insert(connection, project, actor, input, payload)?,
    };
    Ok(AuditResponse::Appended {
        project_id: project.into(),
        record: Box::new(row.decode()?),
    })
}

fn insert(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    input: &AuditInput,
    payload: &str,
) -> Result<JournalRow> {
    let scope = Journal {
        connection,
        project,
    }
    .accept(input)?;
    let ordinal = high_water(connection, project)
        .and_then(|value| value.checked_add(1).context("audit ordinal exhausted"))?;
    let now = Utc::now();
    let row = JournalRow {
        project_id: project.into(),
        id: input.id.clone(),
        ordinal,
        kind: scope.kind.into(),
        control_key: scope.control.key,
        control_version: scope.control.version,
        run_id: scope.run_id.map(str::to_owned),
        finding_id: scope.finding_id.map(str::to_owned),
        occurred_at: timestamp(input.occurred_at),
        recorded_at: timestamp(now),
        actor_json: serde_json::to_string(actor)?,
        input_json: payload.into(),
    };
    diesel::insert_into(records::table)
        .values(&row)
        .execute(connection)?;
    super::dashboard::touch_project(connection, project, None, now)?;
    Ok(row)
}

fn find(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    cutoff: i64,
) -> Result<Option<JournalRow>> {
    Ok(records::table
        .find((project, id))
        .filter(records::ordinal.le(cutoff))
        .select(JournalRow::as_select())
        .first(connection)
        .optional()?)
}

struct Scope<'a> {
    kind: &'static str,
    control: ControlVersion,
    run_id: Option<&'a str>,
    finding_id: Option<&'a str>,
}

impl<'a> Scope<'a> {
    fn control(kind: &'static str, control: &ControlVersion) -> Self {
        Self {
            kind,
            control: control.clone(),
            run_id: None,
            finding_id: None,
        }
    }

    fn run(kind: &'static str, control: ControlVersion, id: &'a str) -> Self {
        Self {
            kind,
            control,
            run_id: Some(id),
            finding_id: None,
        }
    }
}

struct Journal<'a> {
    connection: &'a mut DbConnection,
    project: &'a str,
}

impl Journal<'_> {
    fn accept<'input>(&mut self, input: &'input AuditInput) -> Result<Scope<'input>> {
        match &input.event {
            AuditEvent::Define {
                control,
                previous,
                definition,
            } => {
                self.check_control_head(
                    control,
                    previous.as_deref(),
                    definition.effective_at,
                    "define",
                )?;
                Ok(Scope::control("define", control))
            }
            AuditEvent::Retire {
                control,
                previous,
                effective_at,
                ..
            } => {
                self.check_control_head(control, Some(previous), *effective_at, "retire")?;
                Ok(Scope::control("retire", control))
            }
            AuditEvent::Run { control, run } => {
                self.check_run(control, run)?;
                Ok(Scope::run("run", control.clone(), &input.id))
            }
            AuditEvent::CorrectRun {
                run_id,
                previous,
                result,
                ..
            } => {
                let control = self.check_correction(run_id, previous, result)?;
                Ok(Scope::run("correct_run", control, run_id))
            }
            AuditEvent::Disposition {
                run_id,
                finding_id,
                previous,
                disposition,
                ..
            } => {
                let control = self.check_disposition(
                    run_id,
                    finding_id.as_deref(),
                    previous.as_deref(),
                    *disposition,
                )?;
                Ok(Scope {
                    finding_id: finding_id.as_deref(),
                    ..Scope::run("disposition", control, run_id)
                })
            }
            AuditEvent::Verification {
                run_id,
                disposition_id,
                ..
            } => self.check_verification(run_id, disposition_id),
        }
    }

    fn check_control_head(
        &mut self,
        control: &ControlVersion,
        previous: Option<&str>,
        effective_at: DateTime<Utc>,
        kind: &str,
    ) -> Result<()> {
        let head = queries::control_head(self.connection, self.project, &control.key, i64::MAX)?;
        ensure!(
            head.as_ref().map(|row| row.id.as_str()) == previous,
            conflict("audit control predecessor changed")
        );
        match head {
            None => ensure!(
                kind == "define" && control.version == 1,
                invalid("first control version must be 1")
            ),
            Some(row) => {
                let version = if kind == "define" {
                    row.control_version.checked_add(1)
                } else {
                    ensure!(
                        row.kind == "define",
                        conflict("audit control is already retired")
                    );
                    Some(row.control_version)
                };
                ensure!(
                    Some(control.version) == version,
                    conflict("unexpected control version")
                );
                let previous_effective_at = match row.decode()?.input.event {
                    AuditEvent::Define { definition, .. } => definition.effective_at,
                    AuditEvent::Retire { effective_at, .. } => effective_at,
                    _ => unreachable!("control head includes only definitions and retirement"),
                };
                ensure!(
                    effective_at >= previous_effective_at,
                    invalid("effective date precedes predecessor")
                );
            }
        }
        Ok(())
    }

    fn check_run(&mut self, control: &ControlVersion, run: &AuditRun) -> Result<()> {
        self.check_result(control, &run.result)?;
        if let Some(retry_of) = &run.retry_of {
            let (previous_control, prior) = self.base_run(retry_of)?;
            ensure!(
                prior.subject == run.subject && previous_control.key == control.key,
                invalid("retry must use the same control key and subject")
            );
        }
        Ok(())
    }

    fn check_result(&mut self, control: &ControlVersion, result: &RunResult) -> Result<()> {
        let definition = queries::definition(self.connection, self.project, control, i64::MAX)?
            .ok_or_else(|| invalid("audit control version was not registered"))?;
        definition_data(&definition)
            .validate_result(result)
            .map_err(invalid)?;
        for evidence in result
            .evidence
            .iter()
            .chain(result.findings.iter().flat_map(|f| &f.evidence))
        {
            self.check_blob(evidence)?;
        }
        Ok(())
    }

    fn check_blob(&mut self, evidence: &AuditEvidence) -> Result<()> {
        if let AuditEvidence::StoredBlob { digest, .. } = evidence {
            use super::schema::eval_blobs;
            let verified = eval_blobs::table
                .find((self.project, digest))
                .select(eval_blobs::verified)
                .first::<bool>(self.connection)
                .optional()?;
            ensure!(
                verified == Some(true),
                invalid("audit evidence requires a verified project blob")
            );
        }
        Ok(())
    }

    fn base_run(&mut self, run_id: &str) -> Result<(ControlVersion, AuditRun)> {
        let row = find(self.connection, self.project, run_id, i64::MAX)?
            .ok_or_else(|| invalid("audit run was not found"))?;
        let AuditEvent::Run { control, run } = row.decode()?.input.event else {
            return Err(invalid("audit link must target an original run").into());
        };
        Ok((control, run))
    }

    fn check_correction(
        &mut self,
        run_id: &str,
        previous: &str,
        result: &RunResult,
    ) -> Result<ControlVersion> {
        let (control, run) = self.base_run(run_id)?;
        let correction =
            queries::latest_correction(self.connection, self.project, run_id, i64::MAX)?;
        let (head, old) = correction
            .as_ref()
            .map(|record| (record.input.id.as_str(), result_data(record)))
            .unwrap_or((run_id, &run.result));
        ensure!(head == previous, conflict("audit run predecessor changed"));
        ensure!(
            old.findings
                .iter()
                .all(|old| result.findings.iter().any(|new| new.id == old.id)),
            invalid("a correction must retain existing finding IDs")
        );
        run.subject
            .validate_for_status(result.status)
            .map_err(invalid)?;
        self.check_result(&control, result)?;
        Ok(control)
    }

    fn check_disposition(
        &mut self,
        run_id: &str,
        finding_id: Option<&str>,
        previous: Option<&str>,
        disposition: Disposition,
    ) -> Result<ControlVersion> {
        let (control, run) = self.base_run(run_id)?;
        if let Some(id) = finding_id {
            let correction =
                queries::latest_correction(self.connection, self.project, run_id, i64::MAX)?;
            let result = correction.as_ref().map_or(&run.result, result_data);
            ensure!(
                result.findings.iter().any(|finding| finding.id == id),
                invalid("finding not found on audit run")
            );
        }
        let head = queries::latest_disposition(self.connection, self.project, run_id, finding_id)?;
        ensure!(
            head.as_ref().map(|record| record.input.id.as_str()) == previous,
            conflict("audit disposition predecessor changed")
        );
        let was_closed = head.as_ref().is_some_and(|record| {
            !matches!(
                record.input.event,
                AuditEvent::Disposition {
                    disposition: Disposition::Reopened,
                    ..
                }
            )
        });
        ensure!(
            disposition != Disposition::Reopened || was_closed,
            invalid("only a disposed target can be reopened")
        );
        Ok(control)
    }

    fn check_verification<'input>(
        &mut self,
        run_id: &'input str,
        disposition_id: &str,
    ) -> Result<Scope<'input>> {
        let target = find(self.connection, self.project, disposition_id, i64::MAX)?
            .ok_or_else(|| invalid("audit disposition was not found"))?;
        ensure!(
            target.kind == "disposition" && target.run_id.as_deref() == Some(run_id),
            invalid("verification must reference a disposition on this run")
        );
        Ok(Scope::run("verification", target.control(), run_id))
    }
}

fn definition_data(record: &AuditRecord) -> &ControlDefinition {
    let AuditEvent::Define { definition, .. } = &record.input.event else {
        unreachable!("queried a control definition");
    };
    definition
}

fn result_data(record: &AuditRecord) -> &RunResult {
    match &record.input.event {
        AuditEvent::Run { run, .. } => &run.result,
        AuditEvent::CorrectRun { result, .. } => result,
        _ => unreachable!("queried an audit result"),
    }
}
