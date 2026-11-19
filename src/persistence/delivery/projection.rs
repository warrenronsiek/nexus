// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::{rows::*, *};
use crate::persistence::schema::{
    delivery_features, delivery_origins, delivery_presence, delivery_records, delivery_relations,
    delivery_targets,
};
use anyhow::{bail, Context};

pub(super) fn load(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
) -> Result<Option<DeliveryRecord>> {
    delivery_records::table
        .find((project, subject.kind.as_str(), &subject.id))
        .select(RecordRow::as_select())
        .first(connection)
        .optional()?
        .map(|r| r.record())
        .transpose()
}
fn required(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
) -> Result<DeliveryRecord> {
    load(connection, project, subject)?
        .with_context(|| format!("unknown delivery {} {}", subject.kind.as_str(), subject.id))
}
fn fresh(body: DeliveryBody, observation: &DeliveryObservation) -> DeliveryRecord {
    let subject = observation
        .input
        .body
        .subject()
        .expect("record event has a subject");
    let (title, repository, record_state) = match &body {
        DeliveryBody::Contribution { contribution, .. } => (
            contribution.title.clone(),
            contribution.repository.clone(),
            "registered".into(),
        ),
        DeliveryBody::PullRequest { pull_request, .. } => (
            pull_request.title.clone(),
            pull_request.repository.clone(),
            "open".into(),
        ),
        DeliveryBody::Rollout { rollout } => (
            rollout.title.clone(),
            rollout.repository.clone(),
            "active".into(),
        ),
        DeliveryBody::Deployment { deployment } => (
            format!("{} attempt {}", deployment.environment, deployment.attempt),
            deployment.repository.clone(),
            state(&deployment.outcome),
        ),
        DeliveryBody::ReleaseRequirements { requirements } => (
            requirements.revision.commit.clone(),
            requirements.revision.repository.clone(),
            "recorded".into(),
        ),
        DeliveryBody::EnvironmentInventory { inventory } => (
            inventory.environment.clone(),
            inventory.repository.clone(),
            "observed".into(),
        ),
        DeliveryBody::EnvironmentAdvisory { advisory } => {
            (advisory.title.clone(), String::new(), "reported".into())
        }
        DeliveryBody::Remediation {
            remediation,
            state: current,
            ..
        } => (
            remediation.title.clone(),
            remediation.repository.clone(),
            state(current),
        ),
    };
    DeliveryRecord {
        subject,
        title,
        repository,
        state: record_state,
        updated_at: observation.input.occurred_at,
        assignment_event_id: None,
        composition_event_id: matches!(&body, DeliveryBody::PullRequest { .. })
            .then(|| observation.input.id.clone()),
        shepherd: None,
        shepherd_presence: None,
        body,
    }
}

fn state<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("typed enum serialization")
        .as_str()
        .expect("string enum")
        .into()
}
fn save(connection: &mut DbConnection, project: &str, record: &DeliveryRecord) -> Result<()> {
    let row = RecordRow::encode(project, record)?;
    diesel::insert_into(delivery_records::table)
        .values(&row)
        .on_conflict((
            delivery_records::project_id,
            delivery_records::kind,
            delivery_records::id,
        ))
        .do_update()
        .set((
            delivery_records::repository.eq(&row.repository),
            delivery_records::title.eq(&row.title),
            delivery_records::state.eq(&row.state),
            delivery_records::updated_at.eq(&row.updated_at),
            delivery_records::recipient_agent.eq(&row.recipient_agent),
            delivery_records::recipient_session.eq(&row.recipient_session),
            delivery_records::payload_json.eq(&row.payload_json),
        ))
        .execute(connection)?;
    Ok(())
}
fn link(
    connection: &mut DbConnection,
    project: &str,
    source: &DeliverySubject,
    kind: DeliveryKind,
    id: &str,
) -> Result<()> {
    let row = RelationRow {
        project_id: project,
        source_kind: source.kind.as_str(),
        source_id: &source.id,
        target_kind: kind.as_str(),
        target_id: id,
    };
    diesel::insert_into(delivery_relations::table)
        .values(row)
        .on_conflict_do_nothing()
        .execute(connection)?;
    Ok(())
}
fn target(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
    kind: &str,
    value: &str,
) -> Result<()> {
    diesel::insert_into(delivery_targets::table)
        .values((
            delivery_targets::project_id.eq(project),
            delivery_targets::kind.eq(subject.kind.as_str()),
            delivery_targets::subject_id.eq(&subject.id),
            delivery_targets::target_kind.eq(kind),
            delivery_targets::target.eq(value),
        ))
        .on_conflict_do_nothing()
        .execute(connection)?;
    Ok(())
}
fn new_record(connection: &mut DbConnection, project: &str, record: &DeliveryRecord) -> Result<()> {
    if load(connection, project, &record.subject)?.is_some() {
        bail!(DeliveryRejection::Conflict(format!(
            "{} already registered",
            record.subject.id
        )))
    }
    save(connection, project, record)
}
mod composition;
mod events;
pub(super) fn apply(
    connection: &mut DbConnection,
    project: &str,
    observation: &DeliveryObservation,
) -> Result<()> {
    debug_assert_eq!(project, observation.project_id);
    match &observation.input.body {
        DeliveryEvent::ContributionRegistered { .. } => {
            events::contribution_registered(connection, observation)
        }
        DeliveryEvent::RevisionMapped { .. } => events::revision_mapped(connection, observation),
        DeliveryEvent::PullRequestRegistered { .. } => {
            events::pull_request_registered(connection, observation)
        }
        DeliveryEvent::PullRequestCompositionRecorded { .. } => {
            composition::record(connection, observation)
        }
        DeliveryEvent::PullRequestObserved { .. } => {
            events::pull_request_observed(connection, observation)
        }
        DeliveryEvent::RolloutRegistered { .. } => {
            events::rollout_registered(connection, observation)
        }
        DeliveryEvent::DeploymentFinished { .. } => {
            events::deployment_finished(connection, observation)
        }
        DeliveryEvent::DeploymentRecorded { .. } => {
            events::deployment_recorded(connection, observation)
        }
        DeliveryEvent::ShepherdAssigned { .. } => {
            events::shepherd_assigned(connection, observation)
        }
        DeliveryEvent::AgentPresenceObserved { .. } => {
            events::agent_presence_observed(connection, observation)
        }
        DeliveryEvent::RemediationRequested { .. } => {
            events::remediation_requested(connection, observation)
        }
        DeliveryEvent::RemediationTransitioned { .. } => {
            events::remediation_transitioned(connection, observation)
        }
        DeliveryEvent::ReleaseRequirementsRecorded { .. }
        | DeliveryEvent::EnvironmentInventoryObserved { .. }
        | DeliveryEvent::EnvironmentAdvisoryReported { .. }
        | DeliveryEvent::WatchRegistered { .. }
        | DeliveryEvent::WatchStopped { .. } => apply_context(connection, observation),
    }
}

fn apply_context(connection: &mut DbConnection, observation: &DeliveryObservation) -> Result<()> {
    match &observation.input.body {
        DeliveryEvent::ReleaseRequirementsRecorded { requirements } => context_record(
            connection,
            observation,
            DeliveryBody::ReleaseRequirements {
                requirements: requirements.clone(),
            },
            "revision",
            &requirements.revision.commit.to_ascii_lowercase(),
        ),
        DeliveryEvent::EnvironmentInventoryObserved { inventory } => context_record(
            connection,
            observation,
            DeliveryBody::EnvironmentInventory {
                inventory: inventory.clone(),
            },
            "environment",
            &inventory.environment,
        ),
        DeliveryEvent::EnvironmentAdvisoryReported { advisory } => context_record(
            connection,
            observation,
            DeliveryBody::EnvironmentAdvisory {
                advisory: advisory.clone(),
            },
            "environment",
            &advisory.environment,
        ),
        DeliveryEvent::WatchRegistered { .. } | DeliveryEvent::WatchStopped { .. } => {
            crate::persistence::delivery_watch::apply_watch_event(connection, observation)
        }
        _ => unreachable!("core delivery event"),
    }
}

fn context_record(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
    body: DeliveryBody,
    target_kind: &str,
    value: &str,
) -> Result<()> {
    let project = &observation.project_id;
    let record = fresh(body, observation);
    new_record(connection, project, &record)?;
    target(connection, project, &record.subject, target_kind, value)
}

fn valid_transition(from: RemediationState, to: RemediationState) -> bool {
    use RemediationState::*;
    matches!(
        (from, to),
        (
            Requested,
            Acknowledged | InProgress | Declined | Unreachable | Canceled
        ) | (Acknowledged, InProgress | Declined | Unreachable | Canceled)
            | (InProgress, Resolved | Unreachable | Canceled)
            | (Unreachable, Requested | Canceled)
    )
}
