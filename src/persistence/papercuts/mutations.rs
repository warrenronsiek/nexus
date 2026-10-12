// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
// @entrypoint Store::report_papercut
// @boundary persisted-event-json
use super::super::schema::{papercut_upvotes, papercuts};
use super::super::store::append_event;
use super::super::transaction::{transaction, TransactionMode};
use super::super::Store;
use super::records::{
    canonical_papercut, load_canonical_papercut, load_papercut_row, new_reporter_token,
    normalize_description, normalize_text, papercut_fingerprint, reporter_token_hash,
    validate_actor, validate_project, NewPapercut, NewPapercutUpvote, PapercutRow,
    PapercutUpvoteRow, MAX_TEXT_BYTES, MAX_TITLE_BYTES,
};
use super::registry;
use crate::papercuts::{Papercut, PapercutActor, PapercutReportResult, PapercutUpvote};
use anyhow::{Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde_json::{json, Value};
use uuid::Uuid;

struct ReportWrite<'a> {
    project_id: &'a str,
    title: String,
    description: String,
    fingerprint: String,
    actor: &'a PapercutActor,
    now: String,
}

impl<'a> ReportWrite<'a> {
    fn prepare(
        project_id: &'a str,
        title: &str,
        description: &str,
        actor: &'a PapercutActor,
    ) -> Result<Self> {
        validate_project(project_id)?;
        validate_actor(actor)?;
        let title = normalize_text("papercut title", title, MAX_TITLE_BYTES)?;
        let description = normalize_description(description)?;
        let fingerprint = papercut_fingerprint(&title, &description)?;
        Ok(Self {
            project_id,
            title,
            description,
            fingerprint,
            actor,
            now: Utc::now().to_rfc3339(),
        })
    }
}

struct ReportTarget {
    canonical: PapercutRow,
    reported_id: String,
    reporter_token: Option<String>,
}

struct ReportOutcome {
    papercut_id: String,
    reporter_token: Option<String>,
}

struct UpvoteWrite<'a> {
    project_id: &'a str,
    papercut_id: &'a str,
    note: Option<String>,
    actor: &'a PapercutActor,
    now: String,
}

impl<'a> UpvoteWrite<'a> {
    fn prepare(
        project_id: &'a str,
        papercut_id: &'a str,
        note: Option<&str>,
        actor: &'a PapercutActor,
    ) -> Result<Self> {
        validate_project(project_id)?;
        validate_actor(actor)?;
        let note = note
            .map(|value| normalize_text("papercut upvote note", value, MAX_TEXT_BYTES))
            .transpose()?;
        Ok(Self {
            project_id,
            papercut_id,
            note,
            actor,
            now: Utc::now().to_rfc3339(),
        })
    }
}

struct UpvoteOutcome {
    canonical_id: String,
    upvote_id: String,
}

struct ResolutionWrite<'a> {
    project_id: &'a str,
    papercut_id: &'a str,
    resolution: String,
    actor: &'a PapercutActor,
    now: String,
}

impl<'a> ResolutionWrite<'a> {
    fn prepare(
        project_id: &'a str,
        papercut_id: &'a str,
        resolution: &str,
        actor: &'a PapercutActor,
    ) -> Result<Self> {
        validate_project(project_id)?;
        validate_actor(actor)?;
        let resolution = normalize_text("papercut resolution", resolution, MAX_TEXT_BYTES)?;
        Ok(Self {
            project_id,
            papercut_id,
            resolution,
            actor,
            now: Utc::now().to_rfc3339(),
        })
    }
}

struct OccurrenceWrite<'a> {
    project_id: &'a str,
    canonical: &'a PapercutRow,
    reported_id: &'a str,
    actor: &'a PapercutActor,
    now: &'a str,
    kind: OccurrenceKind<'a>,
}

#[derive(Clone, Copy)]
enum OccurrenceKind<'a> {
    Report { created: bool, title: &'a str },
    Upvote { note: Option<&'a str> },
}

impl<'a> OccurrenceKind<'a> {
    fn note(self) -> Option<&'a str> {
        match self {
            Self::Report { .. } => None,
            Self::Upvote { note } => note,
        }
    }

    fn event(self, papercut_id: &str, reported_id: &str) -> (&'static str, Value) {
        match self {
            Self::Report { created, title } => (
                "papercut_reported",
                json!({
                    "papercut_id": papercut_id,
                    "reported_papercut_id": reported_id,
                    "created": created,
                    "title": title,
                }),
            ),
            Self::Upvote { note } => (
                "papercut_upvoted",
                json!({
                    "papercut_id": papercut_id,
                    "reported_papercut_id": reported_id,
                    "note": note,
                }),
            ),
        }
    }
}

impl Store {
    pub fn report_papercut(
        &mut self,
        project_id: &str,
        title: &str,
        description: &str,
        actor: &PapercutActor,
    ) -> Result<PapercutReportResult> {
        let write = ReportWrite::prepare(project_id, title, description, actor)?;
        let outcome = transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| report_transaction(connection, &write),
        )?;
        let papercut =
            registry::load_papercut(&mut self.connection, project_id, &outcome.papercut_id)?;
        Ok(PapercutReportResult {
            papercut,
            created: outcome.reporter_token.is_some(),
            reporter_token: outcome.reporter_token,
        })
    }

    pub fn upvote_papercut(
        &mut self,
        project_id: &str,
        papercut_id: &str,
        note: Option<&str>,
        actor: &PapercutActor,
    ) -> Result<(Papercut, PapercutUpvote)> {
        let write = UpvoteWrite::prepare(project_id, papercut_id, note, actor)?;
        let outcome = transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| upvote_transaction(connection, &write),
        )?;
        let papercut =
            registry::load_papercut(&mut self.connection, project_id, &outcome.canonical_id)?;
        let upvote = load_upvote(&mut self.connection, &outcome.upvote_id)?;
        Ok((papercut, upvote))
    }

    pub fn remove_papercut(
        &mut self,
        project_id: &str,
        papercut_id: &str,
        resolution: &str,
        actor: &PapercutActor,
    ) -> Result<Papercut> {
        let write = ResolutionWrite::prepare(project_id, papercut_id, resolution, actor)?;
        let canonical_id = transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| resolve_transaction(connection, &write),
        )?;
        registry::load_papercut(&mut self.connection, project_id, &canonical_id)
    }
}

fn report_transaction(
    connection: &mut SqliteConnection,
    write: &ReportWrite<'_>,
) -> Result<ReportOutcome> {
    let target = resolve_report_target(connection, write)?;
    let created = target.reporter_token.is_some();
    append_occurrence(
        connection,
        OccurrenceWrite {
            project_id: write.project_id,
            canonical: &target.canonical,
            reported_id: &target.reported_id,
            actor: write.actor,
            now: &write.now,
            kind: OccurrenceKind::Report {
                created,
                title: &write.title,
            },
        },
    )?;
    Ok(ReportOutcome {
        papercut_id: target.canonical.id,
        reporter_token: target.reporter_token,
    })
}

fn resolve_report_target(
    connection: &mut SqliteConnection,
    write: &ReportWrite<'_>,
) -> Result<ReportTarget> {
    if let Some(row) = find_fingerprint(connection, write.project_id, &write.fingerprint)? {
        let reported_id = row.id.clone();
        return Ok(ReportTarget {
            canonical: canonical_papercut(connection, row)?,
            reported_id,
            reporter_token: None,
        });
    }
    let id = Uuid::new_v4().to_string();
    let token = new_reporter_token();
    let token_hash = reporter_token_hash(&token);
    insert_papercut(connection, write, &id, &token_hash)?;
    Ok(ReportTarget {
        canonical: load_papercut_row(connection, write.project_id, &id)?,
        reported_id: id,
        reporter_token: Some(token),
    })
}

fn upvote_transaction(
    connection: &mut SqliteConnection,
    write: &UpvoteWrite<'_>,
) -> Result<UpvoteOutcome> {
    let canonical = load_canonical_papercut(connection, write.project_id, write.papercut_id)?;
    let upvote_id = append_occurrence(
        connection,
        OccurrenceWrite {
            project_id: write.project_id,
            canonical: &canonical,
            reported_id: write.papercut_id,
            actor: write.actor,
            now: &write.now,
            kind: OccurrenceKind::Upvote {
                note: write.note.as_deref(),
            },
        },
    )?;
    Ok(UpvoteOutcome {
        canonical_id: canonical.id,
        upvote_id,
    })
}

fn resolve_transaction(
    connection: &mut SqliteConnection,
    write: &ResolutionWrite<'_>,
) -> Result<String> {
    let canonical = load_canonical_papercut(connection, write.project_id, write.papercut_id)?;
    if canonical.status == "resolved" {
        return Ok(canonical.id);
    }
    diesel::update(papercuts::table.find(&canonical.id))
        .set((
            papercuts::status.eq("resolved"),
            papercuts::resolution.eq(Some(&write.resolution)),
            papercuts::resolved_at.eq(Some(&write.now)),
            papercuts::updated_at.eq(&write.now),
        ))
        .execute(connection)?;
    append_event(
        connection,
        write.project_id,
        Some(&write.actor.session_id),
        "papercut_resolved",
        json!({
            "papercut_id": canonical.id,
            "resolution": write.resolution,
            "actor": write.actor,
        }),
    )?;
    Ok(canonical.id)
}

fn append_occurrence(
    connection: &mut SqliteConnection,
    write: OccurrenceWrite<'_>,
) -> Result<String> {
    reopen_papercut(connection, write.canonical, write.actor, write.now)?;
    let upvote_id = insert_upvote(connection, &write)?;
    touch_papercut(connection, &write.canonical.id, write.now)?;
    let (event_kind, payload) = write.kind.event(&write.canonical.id, write.reported_id);
    append_event(
        connection,
        write.project_id,
        Some(&write.actor.session_id),
        event_kind,
        payload,
    )?;
    Ok(upvote_id)
}

fn insert_papercut(
    connection: &mut SqliteConnection,
    write: &ReportWrite<'_>,
    id: &str,
    token_hash: &str,
) -> Result<()> {
    diesel::insert_into(papercuts::table)
        .values(NewPapercut {
            id,
            project_id: write.project_id,
            title: &write.title,
            description: &write.description,
            fingerprint: &write.fingerprint,
            reporter_token_hash: token_hash,
            status: "open",
            consolidated_into: None,
            resolution: None,
            creator_agent: &write.actor.agent,
            creator_session_id: &write.actor.session_id,
            creator_model_id: write.actor.model_id.as_deref(),
            creator_config_hash: &write.actor.config_hash,
            created_at: &write.now,
            updated_at: &write.now,
            resolved_at: None,
        })
        .execute(connection)?;
    Ok(())
}

fn insert_upvote(connection: &mut SqliteConnection, write: &OccurrenceWrite<'_>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    diesel::insert_into(papercut_upvotes::table)
        .values(NewPapercutUpvote {
            id: &id,
            project_id: write.project_id,
            papercut_id: &write.canonical.id,
            reported_papercut_id: write.reported_id,
            reporter_agent: &write.actor.agent,
            reporter_session_id: &write.actor.session_id,
            reporter_model_id: write.actor.model_id.as_deref(),
            reporter_config_hash: &write.actor.config_hash,
            note: write.kind.note(),
            created_at: write.now,
        })
        .execute(connection)?;
    Ok(id)
}

fn find_fingerprint(
    connection: &mut SqliteConnection,
    project_id: &str,
    fingerprint: &str,
) -> Result<Option<PapercutRow>> {
    let rows = papercuts::table
        .filter(papercuts::project_id.eq(project_id))
        .filter(papercuts::fingerprint.eq(fingerprint))
        .order((papercuts::updated_at.desc(), papercuts::id.asc()))
        .select(PapercutRow::as_select())
        .load::<PapercutRow>(connection)?;
    let open_index = rows.iter().position(|row| row.status == "open");
    Ok(match open_index {
        Some(index) => rows.into_iter().nth(index),
        None => rows.into_iter().next(),
    })
}

pub(super) fn reopen_papercut(
    connection: &mut SqliteConnection,
    papercut: &PapercutRow,
    actor: &PapercutActor,
    now: &str,
) -> Result<()> {
    if papercut.status == "resolved" {
        diesel::update(papercuts::table.find(&papercut.id))
            .set((
                papercuts::status.eq("open"),
                papercuts::resolution.eq(None::<String>),
                papercuts::resolved_at.eq(None::<String>),
                papercuts::updated_at.eq(now),
            ))
            .execute(connection)?;
        append_event(
            connection,
            &papercut.project_id,
            Some(&actor.session_id),
            "papercut_reopened",
            json!({
                "papercut_id": papercut.id,
                "actor": actor,
            }),
        )?;
    }
    Ok(())
}

pub(super) fn touch_papercut(
    connection: &mut SqliteConnection,
    papercut_id: &str,
    now: &str,
) -> Result<()> {
    diesel::update(papercuts::table.find(papercut_id))
        .set(papercuts::updated_at.eq(now))
        .execute(connection)?;
    Ok(())
}

fn load_upvote(connection: &mut SqliteConnection, upvote_id: &str) -> Result<PapercutUpvote> {
    papercut_upvotes::table
        .find(upvote_id)
        .select(PapercutUpvoteRow::as_select())
        .first::<PapercutUpvoteRow>(connection)
        .optional()?
        .with_context(|| format!("unknown papercut upvote {upvote_id}"))?
        .into_domain()
}
