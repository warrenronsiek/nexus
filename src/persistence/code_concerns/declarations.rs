// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{counts::status_name, publication::Origin, queries, rows::*};
use crate::code_concerns::*;
use crate::persistence::{backend::DbConnection, schema::code_concerns as c};
use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use diesel::prelude::*;

struct Definition {
    title: String,
    description: String,
    hypothesis: Option<String>,
    impact: Option<String>,
    fingerprint: String,
    created_at: DateTime<Utc>,
}
impl Definition {
    fn load(row: &ObservationRow) -> Result<Option<Self>> {
        let observation = row.observation()?;
        let ConcernEvent::Reported {
            title,
            description,
            hypothesis,
            impact,
            encounter,
        } = observation.input.body
        else {
            return Ok(None);
        };
        let fingerprint = fingerprint(&title, &description, &encounter)?;
        Ok(Some(Self {
            title,
            description,
            hypothesis,
            impact,
            fingerprint,
            created_at: observation.input.observed_at,
        }))
    }
    fn summary(&self, row: &ObservationRow) -> ConcernSummary {
        ConcernSummary {
            id: row.concern_id.clone(),
            canonical_id: row.canonical_id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            hypothesis: self.hypothesis.clone(),
            impact: self.impact.clone(),
            status: if row.canonical_id == row.concern_id {
                ConcernStatus::Open
            } else {
                ConcernStatus::Consolidated
            },
            counts: ConcernCounts::default(),
            last_observed_at: Some(self.created_at),
            created_at: self.created_at,
            resolved_at: None,
            lifecycle_qualification: None,
            areas: Vec::new(),
            areas_partial: false,
        }
    }
    fn insert(&self, connection: &mut DbConnection, row: &ObservationRow) -> Result<()> {
        let summary = self.summary(row);
        let record = ConcernRow {
            project_id: row.project_id.clone(),
            id: row.concern_id.clone(),
            canonical_id: row.canonical_id.clone(),
            fingerprint: self.fingerprint.clone(),
            title: self.title.clone(),
            status: status_name(summary.status).into(),
            last_observed_at: row.observed_at.clone(),
            work_session_count: 0,
            returns_count: 0,
            summary_json: serde_json::to_string(&summary)?,
            reporter_token_hash: row.reporter_token_hash.clone(),
        };
        diesel::insert_into(c::table)
            .values(record)
            .execute(connection)?;
        Ok(())
    }
}
fn fingerprint(title: &str, description: &str, encounter: &ConcernEncounter) -> Result<String> {
    let normalize = |value: &str| {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let mut identities = encounter
        .areas
        .iter()
        .flat_map(|v| {
            std::iter::once(v.area.identity())
                .chain(v.memberships.iter().map(ConcernArea::identity))
        })
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();
    Ok(crate::identity::stable_id(&[
        "code-concern",
        &normalize(title),
        &normalize(description),
        &serde_json::to_string(&identities)?,
    ]))
}
pub(super) fn declare(
    connection: &mut DbConnection,
    row: &mut ObservationRow,
    origin: Origin,
) -> Result<Option<String>> {
    let Some(definition) = Definition::load(row)? else {
        return Ok(None);
    };
    match queries::lookup(connection, &row.project_id, &row.concern_id)? {
        Some(existing) => {
            ensure!(
                existing.fingerprint == definition.fingerprint,
                ConcernRejection::Conflict(row.concern_id.clone())
            );
            row.canonical_id = existing.canonical_id;
            Ok(None)
        }
        None => create(connection, row, &definition, origin),
    }
}
fn create(
    connection: &mut DbConnection,
    row: &mut ObservationRow,
    definition: &Definition,
    origin: Origin,
) -> Result<Option<String>> {
    if origin == Origin::Local {
        row.canonical_id = c::table
            .filter(c::project_id.eq(&row.project_id))
            .filter(c::fingerprint.eq(&definition.fingerprint))
            .filter(c::id.eq(c::canonical_id))
            .select(c::id)
            .first::<String>(connection)
            .optional()?
            .unwrap_or_else(|| row.concern_id.clone());
    }
    let secret = reporter_token(row, origin);
    definition.insert(connection, row)?;
    Ok(secret)
}
fn reporter_token(row: &mut ObservationRow, origin: Origin) -> Option<String> {
    if row.canonical_id != row.concern_id
        || row.reporter_token_hash.is_some()
        || origin == Origin::Imported
    {
        return None;
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    row.reporter_token_hash = Some(blake3::hash(token.as_bytes()).to_hex().to_string());
    Some(token)
}
