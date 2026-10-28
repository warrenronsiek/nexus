// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{projection, queries, rows::*};
use crate::code_concerns::*;
use crate::persistence::{
    backend::DbConnection,
    schema::{
        code_concerns as c, concern_decisions as d, concern_encounters as e,
        concern_observations as o,
    },
};
use anyhow::{ensure, Result};
use diesel::prelude::*;
use subtle::ConstantTimeEq;

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name=d)]
struct DecisionRow {
    project_id: String,
    proposal_id: String,
    event_id: String,
    concern_id: String,
    decision: String,
}
pub(super) fn clear_index(connection: &mut DbConnection) -> Result<()> {
    diesel::delete(d::table).execute(connection)?;
    Ok(())
}
pub(super) fn authorize(
    connection: &mut DbConnection,
    project: &str,
    input: &ConcernObservationInput,
    token: Option<&str>,
) -> Result<()> {
    match &input.body {
        ConcernEvent::DuplicateProposed {
            target_concern_id, ..
        } => authorize_proposal(connection, project, &input.concern_id, target_concern_id)?,
        ConcernEvent::DuplicateResponded { proposal_id, .. } => {
            authorize_response(connection, project, input, proposal_id, token)?
        }
        _ => (),
    };
    Ok(())
}
fn required_concern(connection: &mut DbConnection, project: &str, id: &str) -> Result<ConcernRow> {
    queries::lookup(connection, project, id)?
        .ok_or_else(|| ConcernRejection::Invalid("unknown duplicate subject".into()).into())
}
fn authorize_proposal(
    connection: &mut DbConnection,
    project: &str,
    source_id: &str,
    target_id: &str,
) -> Result<()> {
    let source = required_concern(connection, project, source_id)?;
    let target = required_concern(connection, project, target_id)?;
    ensure!(
        source.canonical_id == source.id && target.canonical_id != source.id,
        ConcernRejection::Invalid("invalid duplicate proposal".into())
    );
    Ok(())
}
fn authorize_response(
    connection: &mut DbConnection,
    project: &str,
    input: &ConcernObservationInput,
    proposal: &str,
    token: Option<&str>,
) -> Result<()> {
    let source = required_concern(connection, project, &input.concern_id)?;
    verify_capability(&source, token)?;
    verify_proposal(connection, project, &input.concern_id, proposal)?;
    ensure!(
        !diesel::select(diesel::dsl::exists(d::table.find((project, proposal))))
            .get_result::<bool>(connection)?,
        ConcernRejection::Invalid("duplicate proposal already answered".into())
    );
    Ok(())
}
fn verify_capability(source: &ConcernRow, token: Option<&str>) -> Result<()> {
    ensure!(
        source.canonical_id == source.id,
        ConcernRejection::Invalid("source concern is already consolidated".into())
    );
    let expected = source
        .reporter_token_hash
        .as_deref()
        .ok_or_else(|| ConcernRejection::Invalid("source has no reporter capability".into()))?;
    let actual = blake3::hash(token.unwrap_or("").as_bytes())
        .to_hex()
        .to_string();
    ensure!(
        bool::from(actual.as_bytes().ct_eq(expected.as_bytes())),
        ConcernRejection::Invalid("reporter authorization required".into())
    );
    Ok(())
}
fn verify_proposal(
    connection: &mut DbConnection,
    project: &str,
    source: &str,
    id: &str,
) -> Result<()> {
    let proposal = o::table
        .find((project, id))
        .select(ObservationRow::as_select())
        .first::<ObservationRow>(connection)
        .optional()?
        .ok_or_else(|| ConcernRejection::Invalid("unknown proposal".into()))?;
    ensure!(
        proposal.concern_id == source && proposal.kind == "duplicate_proposed",
        ConcernRejection::Invalid("proposal does not belong to source concern".into())
    );
    Ok(())
}
pub(super) fn index(
    connection: &mut DbConnection,
    row: &ObservationRow,
    body: &ConcernEvent,
) -> Result<()> {
    if let ConcernEvent::DuplicateResponded {
        proposal_id,
        decision,
        ..
    } = body
    {
        record_decision(connection, row, proposal_id, *decision)?;
        if *decision == ConcernDuplicateDecision::Approve {
            consolidate(connection, row, proposal_id)?;
        }
    }
    Ok(())
}
fn record_decision(
    connection: &mut DbConnection,
    row: &ObservationRow,
    proposal: &str,
    decision: ConcernDuplicateDecision,
) -> Result<()> {
    let existing = d::table
        .find((&row.project_id, proposal))
        .select(DecisionRow::as_select())
        .first::<DecisionRow>(connection)
        .optional()?;
    if let Some(existing) = existing {
        ensure!(
            existing.event_id == row.id,
            ConcernRejection::Conflict("duplicate proposal has several responses".into())
        );
        return Ok(());
    }
    let record = DecisionRow {
        project_id: row.project_id.clone(),
        proposal_id: proposal.into(),
        event_id: row.id.clone(),
        concern_id: row.concern_id.clone(),
        decision: match decision {
            ConcernDuplicateDecision::Approve => "approve",
            ConcernDuplicateDecision::Reject => "reject",
        }
        .into(),
    };
    diesel::insert_into(d::table)
        .values(record)
        .execute(connection)?;
    Ok(())
}
fn consolidation_target(
    connection: &mut DbConnection,
    row: &ObservationRow,
    proposal_id: &str,
) -> Result<Option<String>> {
    let proposal = o::table
        .find((&row.project_id, proposal_id))
        .select(ObservationRow::as_select())
        .first::<ObservationRow>(connection)
        .optional()?;
    let Some(proposal) = proposal else {
        return Ok(None);
    };
    let ConcernEvent::DuplicateProposed {
        target_concern_id, ..
    } = proposal.observation()?.input.body
    else {
        return Ok(None);
    };
    Ok(queries::lookup(connection, &row.project_id, &target_concern_id)?.map(|v| v.canonical_id))
}
fn consolidate(
    connection: &mut DbConnection,
    row: &ObservationRow,
    proposal_id: &str,
) -> Result<()> {
    let Some(target) = consolidation_target(connection, row, proposal_id)? else {
        return Ok(());
    };
    ensure!(
        target != row.concern_id,
        ConcernRejection::Invalid("consolidation cycle".into())
    );
    move_associations(connection, row, &target)?;
    projection::refresh(connection, &row.project_id, &target)
}
fn move_associations(
    connection: &mut DbConnection,
    row: &ObservationRow,
    target: &str,
) -> Result<()> {
    diesel::update(
        c::table
            .filter(c::project_id.eq(&row.project_id))
            .filter(c::canonical_id.eq(&row.concern_id)),
    )
    .set((c::canonical_id.eq(target), c::status.eq("consolidated")))
    .execute(connection)?;
    diesel::update(
        e::table
            .filter(e::project_id.eq(&row.project_id))
            .filter(e::concern_id.eq(&row.concern_id)),
    )
    .set(e::concern_id.eq(target))
    .execute(connection)?;
    Ok(())
}
