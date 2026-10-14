// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
use super::projections::load_issue;
use super::records::NewSosLifecycle;
use crate::config::canonical_project_id;
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{sos_issue_lifecycle, sos_issues};
use crate::persistence::store::append_event;
use crate::persistence::transaction::{transaction, TransactionMode};
use crate::persistence::Store;
use crate::sos::{
    required_text, validate_actor, SosActor, SosIssue, SosIssueStatus, SosIssueTransition,
    MAX_SOS_TEXT_BYTES,
};
use anyhow::Result;
use chrono::Utc;
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum IssueClosure<'a> {
    Resolved(&'a str),
    Deprecated(&'a str),
}

impl<'a> IssueClosure<'a> {
    fn reason(self) -> &'a str {
        match self {
            Self::Resolved(reason) | Self::Deprecated(reason) => reason,
        }
    }

    fn state(self) -> (SosIssueStatus, SosIssueTransition) {
        match self {
            Self::Resolved(_) => (SosIssueStatus::Resolved, SosIssueTransition::Resolved),
            Self::Deprecated(_) => (SosIssueStatus::Deprecated, SosIssueTransition::Deprecated),
        }
    }
}

struct CloseRequest<'a> {
    project_id: &'a str,
    issue_id: &'a str,
    actor: &'a SosActor,
    closure: IssueClosure<'a>,
}

struct PreparedClosure<'a> {
    project_id: &'a str,
    issue_id: &'a str,
    actor: &'a SosActor,
    reason: String,
    status: SosIssueStatus,
    transition: SosIssueTransition,
    created_at: String,
}

impl<'a> PreparedClosure<'a> {
    fn new(request: CloseRequest<'a>) -> Result<Self> {
        let project_id = canonical_project_id(request.project_id)
            .map_err(|_| anyhow::anyhow!("SOS project_id must contain 1 to 256 bytes"))?;
        validate_actor(request.actor)?;
        let reason = required_text(
            "SOS issue closure reason",
            request.closure.reason(),
            MAX_SOS_TEXT_BYTES,
        )?;
        let (status, transition) = request.closure.state();
        Ok(Self {
            project_id,
            issue_id: request.issue_id,
            actor: request.actor,
            reason,
            status,
            transition,
            created_at: Utc::now().to_rfc3339(),
        })
    }
}

macro_rules! close_method {
    ($name:ident, $variant:ident) => {
        pub fn $name(
            &mut self,
            project_id: &str,
            issue_id: &str,
            reason: &str,
            actor: &SosActor,
        ) -> Result<SosIssue> {
            self.close_sos_issue(CloseRequest {
                project_id,
                issue_id,
                actor,
                closure: IssueClosure::$variant(reason),
            })
        }
    };
}

impl Store {
    close_method!(resolve_sos_issue, Resolved);
    close_method!(deprecate_sos_issue, Deprecated);

    fn close_sos_issue(&mut self, request: CloseRequest<'_>) -> Result<SosIssue> {
        let closure = PreparedClosure::new(request)?;
        transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| apply_close(connection, &closure),
        )?;
        load_issue(&mut self.connection, closure.project_id, closure.issue_id)
    }
}

fn apply_close(connection: &mut DbConnection, closure: &PreparedClosure<'_>) -> Result<()> {
    let issue = load_issue(connection, closure.project_id, closure.issue_id)?;
    if issue.status == closure.status {
        return Ok(());
    }
    diesel::update(
        sos_issues::table
            .filter(sos_issues::project_id.eq(closure.project_id))
            .find(closure.issue_id),
    )
    .set((
        sos_issues::status.eq(closure.status.to_string()),
        sos_issues::resolution.eq(Some(&closure.reason)),
        sos_issues::updated_at.eq(&closure.created_at),
        sos_issues::closed_at.eq(Some(&closure.created_at)),
    ))
    .execute(connection)?;
    insert_lifecycle(
        connection,
        LifecycleWrite {
            project_id: closure.project_id,
            issue_id: closure.issue_id,
            transition: closure.transition,
            reason: &closure.reason,
            actor: closure.actor,
            created_at: &closure.created_at,
        },
    )?;
    append_event(
        connection,
        closure.project_id,
        Some(&closure.actor.session_id),
        "sos_issue_closed",
        json!({"issue_id": closure.issue_id, "status": closure.status}),
    )?;
    Ok(())
}

pub(super) struct LifecycleWrite<'a> {
    pub(super) project_id: &'a str,
    pub(super) issue_id: &'a str,
    pub(super) transition: SosIssueTransition,
    pub(super) reason: &'a str,
    pub(super) actor: &'a SosActor,
    pub(super) created_at: &'a str,
}

pub(super) fn insert_lifecycle(
    connection: &mut DbConnection,
    write: LifecycleWrite<'_>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    diesel::insert_into(sos_issue_lifecycle::table)
        .values(NewSosLifecycle {
            id: &id,
            project_id: write.project_id,
            issue_id: write.issue_id,
            transition: &write.transition.to_string(),
            reason: write.reason,
            actor_agent: &write.actor.agent,
            actor_session_id: &write.actor.session_id,
            actor_model_id: write.actor.model_id.as_deref(),
            actor_config_hash: &write.actor.config_hash,
            created_at: write.created_at,
        })
        .execute(connection)?;
    Ok(())
}
