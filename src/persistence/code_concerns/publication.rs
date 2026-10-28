// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use super::{counts::stamp, declarations, duplicates, indexing, projection, queries, rows::*};
use crate::code_concerns::*;
use crate::persistence::{backend::DbConnection, schema::concern_observations as o};
use anyhow::{ensure, Result};
use chrono::Utc;
use diesel::prelude::*;
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    Local,
    Imported,
}
pub(super) struct WriteContext<'a> {
    pub project: &'a str,
    pub actor: &'a ConcernActor,
    pub imports: &'a [ConcernObservation],
    pub token: Option<&'a str>,
}
pub(super) struct Publication<'a> {
    context: WriteContext<'a>,
    inputs: &'a [ConcernObservationInput],
}
struct Accepted {
    receipt: ConcernReceipt,
    changed: Option<String>,
}
#[derive(Default)]
struct Batch {
    receipts: Vec<ConcernReceipt>,
    touched: HashSet<String>,
    inserted: usize,
}
impl Batch {
    fn accept(&mut self, item: Accepted) {
        if let Some(id) = item.changed {
            self.touched.insert(id);
            self.inserted += 1;
        }
        self.receipts.push(item.receipt);
    }
}
impl<'a> Publication<'a> {
    pub fn prepare(
        context: WriteContext<'a>,
        inputs: &'a [ConcernObservationInput],
    ) -> Result<Self> {
        let publication = Self { context, inputs };
        publication
            .validate()
            .map_err(|e| ConcernRejection::Invalid(e.to_string()))?;
        Ok(publication)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.context.project.is_empty()
                && self.context.project.len() <= 512
                && !self.inputs.is_empty()
                && self.inputs.len() <= 32,
            "invalid project or batch size"
        );
        self.context.actor.validate()?;
        validate_inputs(self.inputs)?;
        self.context
            .imports
            .iter()
            .try_for_each(ConcernObservation::validate)
    }
    pub fn publish(&self, connection: &mut DbConnection) -> Result<ConcernResponse> {
        let mut batch = Batch::default();
        for input in self.inputs {
            batch.accept(self.publish_one(connection, input)?);
        }
        self.refresh(connection, &batch.touched)?;
        Ok(ConcernResponse::Recorded {
            inserted: batch.inserted,
            replayed: self.inputs.len() - batch.inserted,
            receipts: batch.receipts,
        })
    }
    fn refresh(&self, connection: &mut DbConnection, touched: &HashSet<String>) -> Result<()> {
        if !self.context.imports.is_empty() {
            super::snapshot::reindex_known(connection, self.context.project)?;
        }
        touched
            .iter()
            .try_for_each(|id| projection::refresh(connection, self.context.project, id))
    }
    fn publish_one(
        &self,
        connection: &mut DbConnection,
        input: &ConcernObservationInput,
    ) -> Result<Accepted> {
        let row = self.row(input)?;
        match replay(connection, &row)? {
            Some(receipt) => Ok(Accepted {
                receipt,
                changed: None,
            }),
            None => self.publish_new(connection, input, row),
        }
    }
    fn publish_new(
        &self,
        connection: &mut DbConnection,
        input: &ConcernObservationInput,
        mut row: ObservationRow,
    ) -> Result<Accepted> {
        let origin = if self.context.imports.iter().any(|v| v.input.id == input.id) {
            Origin::Imported
        } else {
            Origin::Local
        };
        let secret = declarations::declare(connection, &mut row, origin)?;
        resolve_target(connection, &mut row, origin)?;
        if origin == Origin::Local {
            duplicates::authorize(connection, &row.project_id, input, self.context.token)?;
        }
        persist(connection, row, secret)
    }
    fn row(&self, input: &ConcernObservationInput) -> Result<ObservationRow> {
        let payload = serde_json::to_string(input)?;
        let original = self.context.imports.iter().find(|v| v.input.id == input.id);
        let source = original.map(|old| {
            ConcernSource::retained(
                old.source.as_ref(),
                &old.project_id,
                old.recorded_at,
                &old.provenance,
            )
        });
        Ok(ObservationRow {
            project_id: self.context.project.into(),
            id: input.id.clone(),
            concern_id: input.concern_id.clone(),
            canonical_id: original
                .and_then(|v| v.canonical_id.clone())
                .unwrap_or_else(|| input.concern_id.clone()),
            kind: input.body.kind().into(),
            occurred_at: input.occurred_at.map(stamp),
            observed_at: stamp(input.observed_at),
            recorded_at: stamp(Utc::now()),
            effective_time: stamp(input.occurred_at.unwrap_or(input.observed_at)),
            content_hash: blake3::hash(payload.as_bytes()).to_hex().to_string(),
            payload_json: payload,
            provenance_json: serde_json::to_string(self.context.actor)?,
            source_json: source.as_ref().map(serde_json::to_string).transpose()?,
            reporter_token_hash: original.and_then(|v| v.reporter_token_hash.clone()),
        })
    }
}
fn validate_inputs(inputs: &[ConcernObservationInput]) -> Result<()> {
    let mut ids = HashSet::new();
    for input in inputs {
        input.validate()?;
        ensure!(ids.insert(&input.id), "duplicate event in batch");
    }
    Ok(())
}
fn replay(connection: &mut DbConnection, row: &ObservationRow) -> Result<Option<ConcernReceipt>> {
    let existing = o::table
        .find((&row.project_id, &row.id))
        .select(ObservationRow::as_select())
        .first::<ObservationRow>(connection)
        .optional()?;
    existing
        .map(|old| {
            ensure!(
                old.content_hash == row.content_hash,
                ConcernRejection::Conflict(row.id.clone())
            );
            old.receipt()
        })
        .transpose()
}
fn resolve_target(
    connection: &mut DbConnection,
    row: &mut ObservationRow,
    origin: Origin,
) -> Result<()> {
    match queries::lookup(connection, &row.project_id, &row.concern_id)? {
        Some(concern) => row.canonical_id = concern.canonical_id,
        None => ensure!(
            origin == Origin::Imported,
            ConcernRejection::Invalid("unknown concern; report it first".into())
        ),
    };
    Ok(())
}
fn persist(
    connection: &mut DbConnection,
    row: ObservationRow,
    secret: Option<String>,
) -> Result<Accepted> {
    diesel::insert_into(o::table)
        .values(&row)
        .execute(connection)?;
    indexing::index(connection, &row)?;
    let mut receipt = row.receipt()?;
    receipt.reporter_token = secret;
    Ok(Accepted {
        receipt,
        changed: Some(row.canonical_id),
    })
}
