// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
use super::super::models::parse_time;
use super::super::schema::{
    papercut_duplicate_reviews, papercut_review_attempts, papercut_upvotes, papercuts,
};
use crate::papercuts::{
    DuplicateReview, DuplicateReviewStatus, Papercut, PapercutActor, PapercutReviewAttempt,
    PapercutReviewAttemptOutcome, PapercutStatus, PapercutUpvote,
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::collections::HashSet;
use std::str::FromStr;
use uuid::Uuid;

pub(super) const MAX_TITLE_BYTES: usize = 256;
pub(super) const MAX_TEXT_BYTES: usize = 8 * 1024;
pub(super) const MAX_METADATA_BYTES: usize = 256;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = papercuts)]
pub(super) struct PapercutRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) reporter_token_hash: String,
    pub(super) status: String,
    pub(super) consolidated_into: Option<String>,
    pub(super) resolution: Option<String>,
    pub(super) creator_agent: String,
    pub(super) creator_session_id: String,
    pub(super) creator_model_id: Option<String>,
    pub(super) creator_config_hash: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) resolved_at: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = papercuts)]
pub(super) struct NewPapercut<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) title: &'a str,
    pub(super) description: &'a str,
    pub(super) fingerprint: &'a str,
    pub(super) reporter_token_hash: &'a str,
    pub(super) status: &'a str,
    pub(super) consolidated_into: Option<&'a str>,
    pub(super) resolution: Option<&'a str>,
    pub(super) creator_agent: &'a str,
    pub(super) creator_session_id: &'a str,
    pub(super) creator_model_id: Option<&'a str>,
    pub(super) creator_config_hash: &'a str,
    pub(super) created_at: &'a str,
    pub(super) updated_at: &'a str,
    pub(super) resolved_at: Option<&'a str>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = papercut_upvotes)]
pub(super) struct PapercutUpvoteRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) papercut_id: String,
    pub(super) reported_papercut_id: String,
    pub(super) reporter_agent: String,
    pub(super) reporter_session_id: String,
    pub(super) reporter_model_id: Option<String>,
    pub(super) reporter_config_hash: String,
    pub(super) note: Option<String>,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = papercut_upvotes)]
pub(super) struct NewPapercutUpvote<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) papercut_id: &'a str,
    pub(super) reported_papercut_id: &'a str,
    pub(super) reporter_agent: &'a str,
    pub(super) reporter_session_id: &'a str,
    pub(super) reporter_model_id: Option<&'a str>,
    pub(super) reporter_config_hash: &'a str,
    pub(super) note: Option<&'a str>,
    pub(super) created_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = papercut_duplicate_reviews)]
pub(super) struct DuplicateReviewRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) papercut_id: String,
    pub(super) canonical_id: String,
    pub(super) reporter_session_id: String,
    pub(super) rationale: String,
    pub(super) question: String,
    pub(super) response: Option<String>,
    pub(super) status: String,
    pub(super) reviewer_agent: String,
    pub(super) reviewer_session_id: String,
    pub(super) provider: Option<String>,
    pub(super) model_id: Option<String>,
    pub(super) created_at: String,
    pub(super) responded_at: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = papercut_duplicate_reviews)]
pub(super) struct NewDuplicateReview<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) papercut_id: &'a str,
    pub(super) canonical_id: &'a str,
    pub(super) reporter_session_id: &'a str,
    pub(super) rationale: &'a str,
    pub(super) question: &'a str,
    pub(super) response: Option<&'a str>,
    pub(super) status: &'a str,
    pub(super) reviewer_agent: &'a str,
    pub(super) reviewer_session_id: &'a str,
    pub(super) provider: Option<&'a str>,
    pub(super) model_id: Option<&'a str>,
    pub(super) created_at: &'a str,
    pub(super) responded_at: Option<&'a str>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = papercut_review_attempts)]
pub(super) struct PapercutReviewAttemptRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) papercut_id: String,
    pub(super) outcome: String,
    pub(super) provider: String,
    pub(super) model_id: Option<String>,
    pub(super) detail: String,
    pub(super) config_hash: String,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = papercut_review_attempts)]
pub(super) struct NewPapercutReviewAttempt<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) papercut_id: &'a str,
    pub(super) outcome: &'a str,
    pub(super) provider: &'a str,
    pub(super) model_id: Option<&'a str>,
    pub(super) detail: &'a str,
    pub(super) config_hash: &'a str,
    pub(super) created_at: &'a str,
}

impl PapercutRow {
    pub(super) fn into_domain(
        self,
        (upvote_count, last_upvoted_at): (u64, Option<DateTime<Utc>>),
    ) -> Result<Papercut> {
        Ok(Papercut {
            id: self.id,
            project_id: self.project_id,
            title: self.title,
            description: self.description,
            status: PapercutStatus::from_str(&self.status).map_err(anyhow::Error::msg)?,
            consolidated_into: self.consolidated_into,
            resolution: self.resolution,
            creator_agent: self.creator_agent,
            creator_session_id: self.creator_session_id,
            creator_model_id: self.creator_model_id,
            creator_config_hash: self.creator_config_hash,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
            resolved_at: self.resolved_at.as_deref().map(parse_time).transpose()?,
            upvote_count,
            last_upvoted_at,
        })
    }
}

impl PapercutUpvoteRow {
    pub(super) fn into_domain(self) -> Result<PapercutUpvote> {
        Ok(PapercutUpvote {
            id: self.id,
            project_id: self.project_id,
            papercut_id: self.papercut_id,
            reported_papercut_id: self.reported_papercut_id,
            reporter_agent: self.reporter_agent,
            reporter_session_id: self.reporter_session_id,
            reporter_model_id: self.reporter_model_id,
            reporter_config_hash: self.reporter_config_hash,
            note: self.note,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

impl DuplicateReviewRow {
    pub(super) fn into_domain(self) -> Result<DuplicateReview> {
        Ok(DuplicateReview {
            id: self.id,
            project_id: self.project_id,
            papercut_id: self.papercut_id,
            canonical_id: self.canonical_id,
            reporter_session_id: self.reporter_session_id,
            rationale: self.rationale,
            question: self.question,
            response: self.response,
            status: DuplicateReviewStatus::from_str(&self.status).map_err(anyhow::Error::msg)?,
            reviewer_agent: self.reviewer_agent,
            reviewer_session_id: self.reviewer_session_id,
            provider: self.provider,
            model_id: self.model_id,
            created_at: parse_time(&self.created_at)?,
            responded_at: self.responded_at.as_deref().map(parse_time).transpose()?,
        })
    }
}

impl PapercutReviewAttemptRow {
    pub(super) fn into_domain(self) -> Result<PapercutReviewAttempt> {
        Ok(PapercutReviewAttempt {
            id: self.id,
            project_id: self.project_id,
            papercut_id: self.papercut_id,
            outcome: PapercutReviewAttemptOutcome::from_str(&self.outcome)
                .map_err(anyhow::Error::msg)?,
            provider: self.provider,
            model_id: self.model_id,
            detail: self.detail,
            config_hash: self.config_hash,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

pub(super) fn load_papercut_row(
    connection: &mut SqliteConnection,
    project_id: &str,
    papercut_id: &str,
) -> Result<PapercutRow> {
    papercuts::table
        .filter(papercuts::project_id.eq(project_id))
        .filter(papercuts::id.eq(papercut_id))
        .select(PapercutRow::as_select())
        .first::<PapercutRow>(connection)
        .optional()?
        .with_context(|| format!("unknown papercut {papercut_id} in project {project_id}"))
}

pub(super) fn canonical_papercut(
    connection: &mut SqliteConnection,
    mut row: PapercutRow,
) -> Result<PapercutRow> {
    let mut visited = HashSet::new();
    while row.status == "consolidated" {
        if !visited.insert(row.id.clone()) {
            bail!("papercut consolidation cycle includes {}", row.id);
        }
        let project_id = row.project_id.clone();
        let target = row
            .consolidated_into
            .as_deref()
            .context("consolidated papercut has no canonical target")?;
        row = load_papercut_row(connection, &project_id, target)?;
    }
    Ok(row)
}

pub(super) fn load_canonical_papercut(
    connection: &mut SqliteConnection,
    project_id: &str,
    papercut_id: &str,
) -> Result<PapercutRow> {
    let row = load_papercut_row(connection, project_id, papercut_id)?;
    canonical_papercut(connection, row)
}

pub(super) fn new_reporter_token() -> String {
    Uuid::new_v4().simple().to_string()
}

pub(super) fn reporter_token_hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

pub(super) fn papercut_fingerprint(title: &str, description: &str) -> Result<String> {
    let identity = serde_json::to_vec(&(title.to_lowercase(), description.to_lowercase()))?;
    Ok(blake3::hash(&identity).to_hex().to_string())
}

pub(super) fn normalize_text(field: &str, value: &str, max_bytes: usize) -> Result<String> {
    if value.chars().any(|character| character == '\0') {
        bail!("{field} cannot contain NUL characters");
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        bail!("{field} cannot be empty");
    }
    if normalized.len() > max_bytes {
        bail!("{field} exceeds the {max_bytes}-byte limit");
    }
    Ok(normalized)
}

pub(super) fn normalize_description(value: &str) -> Result<String> {
    if value.chars().any(|character| character == '\0') {
        bail!("papercut description cannot contain NUL characters");
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() > MAX_TEXT_BYTES {
        bail!("papercut description exceeds the {MAX_TEXT_BYTES}-byte limit");
    }
    Ok(normalized)
}

pub(super) fn validate_project(project_id: &str) -> Result<()> {
    if project_id.trim().is_empty() {
        bail!("papercut project id cannot be empty");
    }
    Ok(())
}

pub(super) fn validate_actor(actor: &PapercutActor) -> Result<()> {
    for (field, value) in [
        ("agent", actor.agent.as_str()),
        ("session id", actor.session_id.as_str()),
        ("config hash", actor.config_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("papercut actor {field} cannot be empty");
        }
    }
    validate_optional("papercut actor model", actor.model_id.as_deref())
}

pub(super) fn validate_optional(field: &str, value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        bail!("{field} cannot be empty when present");
    }
    Ok(())
}
