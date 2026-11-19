// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::*;
pub(super) fn contribution_registered(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::ContributionRegistered { contribution: c } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();

    let record = fresh(
        DeliveryBody::Contribution {
            contribution: c.clone(),
            mappings: vec![],
        },
        observation,
    );
    new_record(connection, project, &record)?;
    if let Some(revision) = &c.head_revision {
        target(
            connection,
            project,
            &record.subject,
            "revision",
            &revision.commit.to_ascii_lowercase(),
        )?;
    }
    if let Some(branch) = &c.branch {
        target(connection, project, &record.subject, "branch", branch)?;
    }
    for feature in &c.features {
        diesel::insert_into(delivery_features::table)
            .values((
                delivery_features::project_id.eq(project),
                delivery_features::contribution_id.eq(&c.id),
                delivery_features::feature.eq(feature),
            ))
            .on_conflict_do_nothing()
            .execute(connection)?;
    }

    Ok(())
}
pub(super) fn revision_mapped(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::RevisionMapped { mapping: m } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();
    let input = &observation.input;

    let mut record = required(connection, project, &input.body.subject().unwrap())?;
    if record.repository != m.revision.repository {
        bail!(DeliveryRejection::Invalid(
            "mapping repository mismatch".into()
        ))
    }
    if let DeliveryBody::Contribution { mappings, .. } = &mut record.body {
        if mappings.iter().any(|old| old.id == m.id) {
            bail!(DeliveryRejection::Conflict(
                "mapping already registered".into()
            ))
        }
        if mappings.len() >= 256 {
            bail!(DeliveryRejection::Invalid(
                "contribution mapping bound reached".into()
            ))
        }
        mappings.push(m.clone());
    }
    target(
        connection,
        project,
        &record.subject,
        "revision",
        &m.revision.commit.to_ascii_lowercase(),
    )?;
    for (index, hunk) in m.hunks.iter().enumerate() {
        diesel::insert_into(delivery_origins::table)
            .values(OriginRow::from_hunk(project, m, index, hunk))
            .execute(connection)?;
    }
    record.updated_at = input.occurred_at;
    save(connection, project, &record)?;

    Ok(())
}
pub(super) fn pull_request_registered(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::PullRequestRegistered { pull_request: p } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();

    reserve_provider_identity(connection, project, p)?;
    let record = fresh(
        DeliveryBody::PullRequest {
            pull_request: p.clone(),
            snapshot: None,
        },
        observation,
    );
    new_record(connection, project, &record)?;
    replace_pull_request_targets(connection, project, &record)?;
    for id in &p.contribution_ids {
        link(
            connection,
            project,
            &record.subject,
            DeliveryKind::Contribution,
            id,
        )?;
    }
    if let Some(id) = &p.rollout_id {
        link(
            connection,
            project,
            &record.subject,
            DeliveryKind::Rollout,
            id,
        )?;
    }

    Ok(())
}
pub(super) fn pull_request_observed(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::PullRequestObserved { snapshot: s, .. } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();
    let input = &observation.input;

    let mut record = required(connection, project, &input.body.subject().unwrap())?;
    if record.repository != s.head.repository || record.repository != s.base.repository {
        bail!(DeliveryRejection::Invalid(
            "snapshot repository mismatch".into()
        ))
    }
    if let DeliveryBody::PullRequest {
        pull_request,
        snapshot,
    } = &mut record.body
    {
        if snapshot
            .as_ref()
            .is_none_or(|old| s.observed_at > old.observed_at)
        {
            pull_request.base = s.base.clone();
            pull_request.head = s.head.clone();
            pull_request.base_ref = s.base_ref.clone();
            pull_request.head_ref = s.head_ref.clone();
            *snapshot = Some(s.clone());
            record.state = state(&s.state);
            record.updated_at = s.observed_at;
            replace_pull_request_targets(connection, project, &record)?;
            save(connection, project, &record)?;
        }
    }

    Ok(())
}
pub(super) fn rollout_registered(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::RolloutRegistered { rollout: r } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();

    let record = fresh(DeliveryBody::Rollout { rollout: r.clone() }, observation);
    new_record(connection, project, &record)?;
    for environment in &r.environments {
        target(
            connection,
            project,
            &record.subject,
            "environment",
            environment,
        )?;
    }
    for id in &r.contribution_ids {
        link(
            connection,
            project,
            &record.subject,
            DeliveryKind::Contribution,
            id,
        )?;
    }
    for id in &r.pull_request_ids {
        link(
            connection,
            project,
            &record.subject,
            DeliveryKind::PullRequest,
            id,
        )?;
    }

    Ok(())
}
pub(super) fn deployment_recorded(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::DeploymentRecorded { deployment: d } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();

    let record = fresh(
        DeliveryBody::Deployment {
            deployment: d.clone(),
        },
        observation,
    );
    new_record(connection, project, &record)?;
    target(
        connection,
        project,
        &record.subject,
        "environment",
        &d.environment,
    )?;
    link(
        connection,
        project,
        &record.subject,
        DeliveryKind::Rollout,
        &d.rollout_id,
    )?;

    Ok(())
}
pub(super) fn shepherd_assigned(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::ShepherdAssigned {
        subject,
        expected_assignment,
        agent,
        reason,
    } = &observation.input.body
    else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();
    let input = &observation.input;

    let mut record = required(connection, project, subject)?;
    if &record.assignment_event_id != expected_assignment {
        bail!(DeliveryRejection::Conflict(
            "shepherd assignment changed; reread before handoff".into()
        ))
    }
    record.assignment_event_id = Some(input.id.clone());
    record.shepherd = agent.as_ref().map(|agent| ShepherdAssignment {
        event_id: input.id.clone(),
        agent: agent.clone(),
        assigned_at: input.occurred_at,
        reason: reason.clone(),
    });
    record.updated_at = input.occurred_at;
    save(connection, project, &record)?;

    Ok(())
}
pub(super) fn agent_presence_observed(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::AgentPresenceObserved { presence: p } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();

    let timestamp = p.observed_at.to_rfc3339();
    let payload = serde_json::to_string(p)?;
    let old = delivery_presence::table
        .find((project, &p.agent.agent, &p.agent.session_id))
        .select(delivery_presence::observed_at)
        .first::<String>(connection)
        .optional()?;
    if old.is_none_or(|value| value < timestamp) {
        diesel::insert_into(delivery_presence::table)
            .values((
                delivery_presence::project_id.eq(project),
                delivery_presence::agent.eq(&p.agent.agent),
                delivery_presence::session_id.eq(&p.agent.session_id),
                delivery_presence::observed_at.eq(&timestamp),
                delivery_presence::payload_json.eq(&payload),
            ))
            .on_conflict((
                delivery_presence::project_id,
                delivery_presence::agent,
                delivery_presence::session_id,
            ))
            .do_update()
            .set((
                delivery_presence::observed_at.eq(&timestamp),
                delivery_presence::payload_json.eq(&payload),
            ))
            .execute(connection)?;
    }

    Ok(())
}
pub(super) fn remediation_requested(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::RemediationRequested { remediation: r } = &observation.input.body else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();

    let record = fresh(
        DeliveryBody::Remediation {
            remediation: r.clone(),
            state: RemediationState::Requested,
            reason: None,
            fixing_revision: None,
        },
        observation,
    );
    new_record(connection, project, &record)?;
    for (kind, id) in [
        (DeliveryKind::PullRequest, &r.pull_request_id),
        (DeliveryKind::Rollout, &r.rollout_id),
        (DeliveryKind::Contribution, &r.contribution_id),
        (DeliveryKind::Deployment, &r.deployment_id),
    ] {
        if let Some(id) = id {
            link(connection, project, &record.subject, kind, id)?;
        }
    }

    Ok(())
}
pub(super) fn remediation_transitioned(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::RemediationTransitioned {
        expected_state,
        state: new,
        reason,
        fixing_revision,
        ..
    } = &observation.input.body
    else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();
    let input = &observation.input;

    let mut record = required(connection, project, &input.body.subject().unwrap())?;
    if let DeliveryBody::Remediation {
        state: current,
        reason: stored,
        fixing_revision: fix,
        ..
    } = &mut record.body
    {
        if current != expected_state {
            bail!(DeliveryRejection::Conflict(
                "remediation state changed".into()
            ))
        };
        if !valid_transition(*current, *new) {
            bail!(DeliveryRejection::Invalid(
                "invalid remediation transition".into()
            ))
        };
        *current = *new;
        *stored = Some(reason.clone());
        *fix = fixing_revision.clone();
        record.state = state(new);
        record.updated_at = input.occurred_at;
        save(connection, project, &record)?;
    }

    Ok(())
}

pub(super) fn deployment_finished(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::DeploymentFinished {
        expected_outcome,
        outcome,
        ..
    } = &observation.input.body
    else {
        unreachable!("typed event dispatch")
    };
    let project = observation.project_id.as_str();
    let mut record = required(
        connection,
        project,
        &observation.input.body.subject().unwrap(),
    )?;
    let DeliveryBody::Deployment { deployment } = &mut record.body else {
        unreachable!("deployment key")
    };
    if deployment.outcome != *expected_outcome {
        bail!(DeliveryRejection::Conflict(
            "deployment outcome changed".into()
        ))
    }
    if !matches!(
        deployment.outcome,
        DeploymentOutcome::Running | DeploymentOutcome::Unknown
    ) || !matches!(
        outcome,
        DeploymentOutcome::Succeeded | DeploymentOutcome::Failed | DeploymentOutcome::Canceled
    ) {
        bail!(DeliveryRejection::Invalid(
            "deployment finish requires an unfinished attempt and terminal outcome".into()
        ))
    }
    deployment.outcome = *outcome;
    record.state = state(outcome);
    record.updated_at = observation.input.occurred_at;
    save(connection, project, &record)
}

fn replace_pull_request_targets(
    connection: &mut DbConnection,
    project: &str,
    record: &DeliveryRecord,
) -> Result<()> {
    let DeliveryBody::PullRequest { pull_request, .. } = &record.body else {
        unreachable!("pull request projection")
    };
    diesel::delete(
        delivery_targets::table
            .filter(delivery_targets::project_id.eq(project))
            .filter(delivery_targets::kind.eq(record.subject.kind.as_str()))
            .filter(delivery_targets::subject_id.eq(&record.subject.id))
            .filter(delivery_targets::target_kind.eq("branch")),
    )
    .execute(connection)?;
    for branch in [&pull_request.base_ref, &pull_request.head_ref]
        .into_iter()
        .flatten()
    {
        target(connection, project, &record.subject, "branch", branch)?;
    }
    Ok(())
}

fn reserve_provider_identity(
    connection: &mut DbConnection,
    project: &str,
    pr: &PullRequest,
) -> Result<()> {
    let identity = crate::delivery::provider_pr_identity(&pr.provider, &pr.repository, pr.number);
    let existing = delivery_targets::table
        .filter(delivery_targets::project_id.eq(project))
        .filter(delivery_targets::target_kind.eq("provider_pr"))
        .filter(delivery_targets::target.eq(&identity))
        .select(delivery_targets::subject_id)
        .first::<String>(connection)
        .optional()?;
    if let Some(existing) = existing {
        bail!(DeliveryRejection::Conflict(format!(
            "pull request already registered as {existing}; use the existing identity"
        )))
    }
    diesel::insert_into(delivery_targets::table)
        .values((
            delivery_targets::project_id.eq(project),
            delivery_targets::kind.eq("pull_request"),
            delivery_targets::subject_id.eq(&pr.id),
            delivery_targets::target_kind.eq("provider_pr"),
            delivery_targets::target.eq(&identity),
        ))
        .execute(connection)?;
    Ok(())
}
