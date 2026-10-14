// @feature papercuts
// @feature persistence
// @spec docs/features/papercuts.md
// @spec docs/features/persistence.md
use super::super::schema::{
    papercut_duplicate_reviews, papercut_review_attempts, papercut_upvotes, papercuts,
};
use super::super::store::append_event;
use super::super::transaction::{prepared_immediate_transaction, transaction, TransactionMode};
use super::super::Store;
use super::mutations::{reopen_papercut, touch_papercut};
use super::records::{
    load_canonical_papercut, load_papercut_row, normalize_text, reporter_token_hash,
    validate_actor, validate_optional, validate_project, DuplicateReviewRow, NewDuplicateReview,
    NewPapercutReviewAttempt, PapercutRow, MAX_METADATA_BYTES, MAX_TEXT_BYTES,
};
use super::{PapercutDuplicateProposal, PapercutReviewAttemptRecord};
use crate::papercuts::{
    DuplicateReview, PapercutActor, PapercutReviewAttemptOutcome, PapercutReviewDecision,
};
use crate::persistence::backend::DbConnection;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

struct PreparedProposal<'a> {
    input: PapercutDuplicateProposal<'a>,
    rationale: String,
    question: String,
    now: String,
}

impl<'a> PreparedProposal<'a> {
    fn prepare(input: PapercutDuplicateProposal<'a>) -> Result<Self> {
        validate_proposal(&input)?;
        let rationale = normalize_text(
            "duplicate review rationale",
            input.rationale,
            MAX_TEXT_BYTES,
        )?;
        let question = normalize_text("duplicate review question", input.question, MAX_TEXT_BYTES)?;
        Ok(Self {
            input,
            rationale,
            question,
            now: Utc::now().to_rfc3339(),
        })
    }
}

struct ReviewResponseWrite<'a> {
    review_id: &'a str,
    decision: PapercutReviewDecision,
    response: String,
    reporter_token: &'a str,
    actor: &'a PapercutActor,
    now: String,
}

impl<'a> ReviewResponseWrite<'a> {
    fn prepare(
        review_id: &'a str,
        decision: PapercutReviewDecision,
        response: &str,
        reporter_token: &'a str,
        actor: &'a PapercutActor,
    ) -> Result<Self> {
        validate_actor(actor)?;
        if reporter_token.is_empty() {
            bail!("duplicate review reporter token cannot be empty");
        }
        let response = normalize_text("duplicate review response", response, MAX_TEXT_BYTES)?;
        Ok(Self {
            review_id,
            decision,
            response,
            reporter_token,
            actor,
            now: Utc::now().to_rfc3339(),
        })
    }
}

struct PreparedAttempt<'a> {
    input: PapercutReviewAttemptRecord<'a>,
    provider: String,
    model_id: Option<String>,
    detail: String,
    config_hash: String,
    now: String,
}

impl<'a> PreparedAttempt<'a> {
    fn prepare(input: PapercutReviewAttemptRecord<'a>) -> Result<Self> {
        validate_project(input.project_id)?;
        let provider = normalize_text(
            "papercut review provider",
            input.provider,
            MAX_METADATA_BYTES,
        )?;
        let model_id = input
            .model_id
            .map(|value| normalize_text("papercut review model", value, MAX_METADATA_BYTES))
            .transpose()?;
        let detail = normalize_text("papercut review detail", input.detail, MAX_TEXT_BYTES)?;
        let config_hash = normalize_text(
            "papercut review config hash",
            input.config_hash,
            MAX_METADATA_BYTES,
        )?;
        Ok(Self {
            input,
            provider,
            model_id,
            detail,
            config_hash,
            now: Utc::now().to_rfc3339(),
        })
    }
}

struct ConsolidationWrite<'a> {
    project_id: &'a str,
    source_id: &'a str,
    target_id: &'a str,
    rationale: &'a str,
    actor: &'a PapercutActor,
    now: &'a str,
}

struct MergePlan {
    source: PapercutRow,
    target: PapercutRow,
}

impl Store {
    pub(crate) fn propose_papercut_duplicate(
        &mut self,
        proposal: PapercutDuplicateProposal<'_>,
    ) -> Result<DuplicateReview> {
        let write = PreparedProposal::prepare(proposal)?;
        let review_id = transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| propose_transaction(connection, &write),
        )?;
        load_duplicate_review(&mut self.connection, &review_id)
    }

    pub fn respond_to_papercut_review(
        &mut self,
        review_id: &str,
        decision: PapercutReviewDecision,
        response: &str,
        reporter_token: &str,
        actor: &PapercutActor,
    ) -> Result<DuplicateReview> {
        let write =
            ReviewResponseWrite::prepare(review_id, decision, response, reporter_token, actor)?;
        transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| respond_transaction(connection, &write),
        )?;
        load_duplicate_review(&mut self.connection, review_id)
    }

    pub(crate) fn record_papercut_review_event(
        &mut self,
        record: PapercutReviewAttemptRecord<'_>,
    ) -> Result<()> {
        prepared_immediate_transaction(
            &mut self.connection,
            record,
            PreparedAttempt::prepare,
            record_attempt_transaction,
        )
    }
}

fn validate_proposal(proposal: &PapercutDuplicateProposal<'_>) -> Result<()> {
    validate_project(proposal.project_id)?;
    validate_actor(proposal.reviewer)?;
    validate_optional("duplicate review provider", proposal.provider)?;
    validate_optional("duplicate review model", proposal.model_id)?;
    if proposal.papercut_id == proposal.canonical_id {
        bail!("a papercut cannot be proposed as a duplicate of itself");
    }
    Ok(())
}

fn propose_transaction(
    connection: &mut DbConnection,
    write: &PreparedProposal<'_>,
) -> Result<String> {
    if let Some(existing) =
        pending_review(connection, write.input.project_id, write.input.papercut_id)?
    {
        return Ok(existing.id);
    }
    let source = review_candidates(connection, write)?;
    insert_duplicate_review(connection, write, &source)
}

fn review_candidates(
    connection: &mut DbConnection,
    write: &PreparedProposal<'_>,
) -> Result<PapercutRow> {
    let source = load_papercut_row(connection, write.input.project_id, write.input.papercut_id)?;
    let target = load_papercut_row(connection, write.input.project_id, write.input.canonical_id)?;
    if source.status != "open" || target.status != "open" {
        bail!("duplicate review requires two open papercuts");
    }
    Ok(source)
}

fn insert_duplicate_review(
    connection: &mut DbConnection,
    write: &PreparedProposal<'_>,
    source: &PapercutRow,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    diesel::insert_into(papercut_duplicate_reviews::table)
        .values(NewDuplicateReview {
            id: &id,
            project_id: write.input.project_id,
            papercut_id: write.input.papercut_id,
            canonical_id: write.input.canonical_id,
            reporter_session_id: &source.creator_session_id,
            rationale: &write.rationale,
            question: &write.question,
            response: None,
            status: "pending_reporter",
            reviewer_agent: &write.input.reviewer.agent,
            reviewer_session_id: &write.input.reviewer.session_id,
            provider: write.input.provider,
            model_id: write.input.model_id,
            created_at: &write.now,
            responded_at: None,
        })
        .execute(connection)?;
    append_event(
        connection,
        write.input.project_id,
        Some(&write.input.reviewer.session_id),
        "papercut_duplicate_proposed",
        json!({
            "review_id": id,
            "papercut_id": write.input.papercut_id,
            "canonical_id": write.input.canonical_id,
            "reporter_session_id": source.creator_session_id,
        }),
    )?;
    Ok(id)
}

fn respond_transaction(
    connection: &mut DbConnection,
    write: &ReviewResponseWrite<'_>,
) -> Result<()> {
    let review = load_review_row(connection, write.review_id)?;
    authorize_response(connection, &review, write.reporter_token)?;
    if review.status == "pending_reporter" {
        finish_review(connection, &review, write)?;
    }
    Ok(())
}

fn authorize_response(
    connection: &mut DbConnection,
    review: &DuplicateReviewRow,
    reporter_token: &str,
) -> Result<()> {
    let source = load_papercut_row(connection, &review.project_id, &review.papercut_id)?;
    if reporter_token_hash(reporter_token) != source.reporter_token_hash {
        bail!("invalid duplicate review reporter token");
    }
    Ok(())
}

fn finish_review(
    connection: &mut DbConnection,
    review: &DuplicateReviewRow,
    write: &ReviewResponseWrite<'_>,
) -> Result<()> {
    let status = match write.decision {
        PapercutReviewDecision::Approve => "approved",
        PapercutReviewDecision::Reject => "rejected",
    };
    diesel::update(papercut_duplicate_reviews::table.find(write.review_id))
        .set((
            papercut_duplicate_reviews::status.eq(status),
            papercut_duplicate_reviews::response.eq(Some(&write.response)),
            papercut_duplicate_reviews::responded_at.eq(Some(&write.now)),
        ))
        .execute(connection)?;
    if write.decision == PapercutReviewDecision::Approve {
        consolidate_transaction(
            connection,
            ConsolidationWrite {
                project_id: &review.project_id,
                source_id: &review.papercut_id,
                target_id: &review.canonical_id,
                rationale: &review.rationale,
                actor: write.actor,
                now: &write.now,
            },
        )?;
    }
    append_event(
        connection,
        &review.project_id,
        Some(&write.actor.session_id),
        "papercut_duplicate_responded",
        json!({
            "review_id": write.review_id,
            "decision": write.decision,
            "response": write.response,
        }),
    )?;
    Ok(())
}

fn consolidate_transaction(
    connection: &mut DbConnection,
    write: ConsolidationWrite<'_>,
) -> Result<()> {
    let plan = plan_consolidation(connection, &write)?;
    reopen_papercut(connection, &plan.target, write.actor, write.now)?;
    apply_consolidation(connection, &plan.source, &plan.target, write.now)?;
    append_event(
        connection,
        write.project_id,
        Some(&write.actor.session_id),
        "papercut_consolidated",
        json!({
            "papercut_id": plan.source.id,
            "canonical_id": plan.target.id,
            "rationale": write.rationale,
            "actor": write.actor,
        }),
    )?;
    Ok(())
}

fn plan_consolidation(
    connection: &mut DbConnection,
    write: &ConsolidationWrite<'_>,
) -> Result<MergePlan> {
    let source = load_papercut_row(connection, write.project_id, write.source_id)?;
    if source.status != "open" {
        bail!("only an open papercut can be consolidated");
    }
    let target = load_canonical_papercut(connection, write.project_id, write.target_id)?;
    if source.id == target.id {
        bail!("a papercut cannot be consolidated into itself");
    }
    Ok(MergePlan { source, target })
}

fn apply_consolidation(
    connection: &mut DbConnection,
    source: &PapercutRow,
    target: &PapercutRow,
    now: &str,
) -> Result<()> {
    diesel::update(papercuts::table.filter(papercuts::consolidated_into.eq(&source.id)))
        .set((
            papercuts::consolidated_into.eq(Some(&target.id)),
            papercuts::updated_at.eq(now),
        ))
        .execute(connection)?;
    diesel::update(papercuts::table.find(&source.id))
        .set((
            papercuts::status.eq("consolidated"),
            papercuts::consolidated_into.eq(Some(&target.id)),
            papercuts::resolution.eq(None::<String>),
            papercuts::resolved_at.eq(None::<String>),
            papercuts::updated_at.eq(now),
        ))
        .execute(connection)?;
    diesel::update(papercut_upvotes::table.filter(papercut_upvotes::papercut_id.eq(&source.id)))
        .set(papercut_upvotes::papercut_id.eq(&target.id))
        .execute(connection)?;
    touch_papercut(connection, &target.id, now)
}

fn record_attempt_transaction(
    connection: &mut DbConnection,
    write: &PreparedAttempt<'_>,
) -> Result<()> {
    let _ = load_papercut_row(connection, write.input.project_id, write.input.papercut_id)?;
    let attempt_id = Uuid::new_v4().to_string();
    let outcome = write.input.outcome.to_string();
    diesel::insert_into(papercut_review_attempts::table)
        .values(NewPapercutReviewAttempt {
            id: &attempt_id,
            project_id: write.input.project_id,
            papercut_id: write.input.papercut_id,
            outcome: &outcome,
            provider: &write.provider,
            model_id: write.model_id.as_deref(),
            detail: &write.detail,
            config_hash: &write.config_hash,
            created_at: &write.now,
        })
        .execute(connection)?;
    append_event(
        connection,
        write.input.project_id,
        None,
        review_event_kind(write.input.outcome),
        json!({
            "attempt_id": attempt_id,
            "papercut_id": write.input.papercut_id,
            "outcome": outcome,
            "provider": write.provider,
            "model_id": write.model_id,
            "detail": write.detail,
            "config_hash": write.config_hash,
        }),
    )
}

fn review_event_kind(outcome: PapercutReviewAttemptOutcome) -> &'static str {
    match outcome {
        PapercutReviewAttemptOutcome::Distinct => "papercut_reviewed",
        PapercutReviewAttemptOutcome::Failed => "papercut_review_failed",
    }
}

fn pending_review(
    connection: &mut DbConnection,
    project_id: &str,
    papercut_id: &str,
) -> Result<Option<DuplicateReviewRow>> {
    papercut_duplicate_reviews::table
        .filter(papercut_duplicate_reviews::project_id.eq(project_id))
        .filter(papercut_duplicate_reviews::papercut_id.eq(papercut_id))
        .filter(papercut_duplicate_reviews::status.eq("pending_reporter"))
        .select(DuplicateReviewRow::as_select())
        .first(connection)
        .optional()
        .map_err(Into::into)
}

fn load_review_row(connection: &mut DbConnection, review_id: &str) -> Result<DuplicateReviewRow> {
    papercut_duplicate_reviews::table
        .find(review_id)
        .select(DuplicateReviewRow::as_select())
        .first::<DuplicateReviewRow>(connection)
        .with_context(|| format!("unknown papercut duplicate review {review_id}"))
}

fn load_duplicate_review(
    connection: &mut DbConnection,
    review_id: &str,
) -> Result<DuplicateReview> {
    load_review_row(connection, review_id)?.into_domain()
}
