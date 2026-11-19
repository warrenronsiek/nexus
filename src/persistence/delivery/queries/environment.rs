// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::*;
use crate::delivery::validation::{git, text};
use crate::persistence::backend::DbBackend;
use crate::persistence::schema::delivery_targets;
use crate::work_history::GitRevision;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

pub(super) fn context(
    connection: &mut DbConnection,
    project: &str,
    environment: &str,
    candidate: Option<&GitRevision>,
    pull_request_id: Option<&str>,
) -> Result<DeliveryResponse> {
    let now = Utc::now();
    text(environment, 256)?;
    let candidate = candidate_revision(connection, project, candidate, pull_request_id)?;
    let (requirements_id, dependencies) =
        assess_dependencies(connection, project, environment, candidate.as_ref(), now)?;
    let (advisories, advisories_partial) = advisories(connection, project, environment, now)?;
    Ok(DeliveryResponse::EnvironmentContext {
        environment: environment.into(),
        candidate,
        requirements_id,
        dependencies,
        advisories,
        advisories_partial,
    })
}

pub(super) fn promotion_readiness(
    connection: &mut DbConnection,
    project: &str,
    registration: &DeliveryWatchRegistration,
) -> Result<Option<PromotionReadiness>> {
    let Some(environment) = &registration.promotion_environment else {
        return Ok(None);
    };
    let subject = DeliverySubject {
        kind: DeliveryKind::PullRequest,
        id: registration.pull_request_id.clone(),
    };
    let Some(record) =
        projection::load(connection, project, &subject)?.filter(|record| record.state == "open")
    else {
        return Ok(None);
    };
    let DeliveryBody::PullRequest {
        pull_request,
        snapshot,
    } = record.body
    else {
        unreachable!("registered promotion watch refers to a pull request");
    };
    let candidate = snapshot.map_or(pull_request.head, |snapshot| snapshot.head);
    let (requirements_id, dependencies) = assess_dependencies(
        connection,
        project,
        environment,
        Some(&candidate),
        Utc::now(),
    )?;
    let missing = dependencies
        .iter()
        .filter(|item| matches!(item.status, DependencyStatus::Missing))
        .count();
    let unknown = dependencies
        .iter()
        .filter(|item| matches!(item.status, DependencyStatus::Unknown))
        .count();
    Ok(Some(PromotionReadiness {
        environment: environment.clone(),
        candidate,
        requirements_id,
        missing,
        unknown,
    }))
}

fn assess_dependencies(
    connection: &mut DbConnection,
    project: &str,
    environment: &str,
    candidate: Option<&GitRevision>,
    now: DateTime<Utc>,
) -> Result<(Option<String>, Vec<DependencyAssessment>)> {
    let requirements = candidate
        .map(|revision| requirements(connection, project, revision))
        .transpose()?
        .flatten();
    Ok(match requirements {
        Some(requirements) => {
            let dependencies = dependencies(
                connection,
                project,
                environment,
                &requirements.requires,
                now,
            )?;
            (Some(requirements.id), dependencies)
        }
        None => (None, Vec::new()),
    })
}

fn advisories(
    connection: &mut DbConnection,
    project: &str,
    environment: &str,
    now: DateTime<Utc>,
) -> Result<(Vec<EnvironmentAdvisory>, bool)> {
    let rows = environment_records(
        project,
        DeliveryKind::EnvironmentAdvisory,
        "environment",
        environment,
    )
    .limit(129)
    .select(RecordRow::as_select())
    .load::<RecordRow>(connection)?;
    let advisories_partial = rows.len() > 128;
    let mut advisories = rows
        .into_iter()
        .take(128)
        .map(|row| row.record())
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|record| match record.body {
            DeliveryBody::EnvironmentAdvisory { advisory }
                if advisory.expires_at > now && record.updated_at <= now =>
            {
                Some(advisory)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let advisories_partial =
        crate::persistence::window::bound_items(&mut advisories, 64, 256 * 1024)?
            || advisories_partial;
    Ok((advisories, advisories_partial))
}

fn candidate_revision(
    connection: &mut DbConnection,
    project: &str,
    candidate: Option<&GitRevision>,
    pull_request_id: Option<&str>,
) -> Result<Option<GitRevision>> {
    match (candidate, pull_request_id) {
        (Some(_), Some(_)) => bail!(DeliveryRejection::Invalid(
            "select a candidate revision or a pull request".into()
        )),
        (Some(candidate), None) => {
            git(candidate)?;
            Ok(Some(candidate.clone()))
        }
        (None, None) => Ok(None),
        (None, Some(id)) => {
            let subject = DeliverySubject {
                kind: DeliveryKind::PullRequest,
                id: id.into(),
            };
            subject.validate()?;
            let Some(DeliveryBody::PullRequest {
                pull_request,
                snapshot,
            }) = projection::load(connection, project, &subject)?.map(|record| record.body)
            else {
                bail!(DeliveryRejection::Invalid("pull request not found".into()));
            };
            Ok(Some(
                snapshot.map_or(pull_request.head, |snapshot| snapshot.head),
            ))
        }
    }
}

fn requirements(
    connection: &mut DbConnection,
    project: &str,
    revision: &GitRevision,
) -> Result<Option<ReleaseRequirements>> {
    let commit = revision.commit.to_ascii_lowercase();
    let row = environment_records(
        project,
        DeliveryKind::ReleaseRequirements,
        "revision",
        &commit,
    )
    .filter(delivery_records::repository.eq(&revision.repository))
    .filter(delivery_records::updated_at.le(Utc::now().to_rfc3339()))
    .select(RecordRow::as_select())
    .first::<RecordRow>(connection)
    .optional()?;
    match row
        .map(|row| row.record())
        .transpose()?
        .map(|record| record.body)
    {
        Some(DeliveryBody::ReleaseRequirements { requirements }) => Ok(Some(requirements)),
        _ => Ok(None),
    }
}

fn dependencies(
    connection: &mut DbConnection,
    project: &str,
    environment: &str,
    requirements: &[EnvironmentKey],
    now: DateTime<Utc>,
) -> Result<Vec<DependencyAssessment>> {
    let mut inventories = BTreeMap::<&str, Option<EnvironmentInventory>>::new();
    requirements
        .iter()
        .map(|requirement| {
            let repository = requirement.repository.as_str();
            if let std::collections::btree_map::Entry::Vacant(entry) = inventories.entry(repository)
            {
                let row = environment_records(
                    project,
                    DeliveryKind::EnvironmentInventory,
                    "environment",
                    environment,
                )
                .filter(delivery_records::repository.eq(repository))
                .filter(delivery_records::updated_at.le(now.to_rfc3339()))
                .select(RecordRow::as_select())
                .first::<RecordRow>(connection)
                .optional()?;
                let inventory =
                    row.map(|row| row.record())
                        .transpose()?
                        .and_then(|record| match record.body {
                            DeliveryBody::EnvironmentInventory { inventory }
                                if inventory.expires_at > now =>
                            {
                                Some(inventory)
                            }
                            _ => None,
                        });
                entry.insert(inventory);
            }
            let inventory = inventories[repository].as_ref();
            let status = match inventory {
                Some(inventory) if inventory.keys.contains(&requirement.key) => {
                    DependencyStatus::Present
                }
                Some(inventory) if inventory.complete => DependencyStatus::Missing,
                _ => DependencyStatus::Unknown,
            };
            Ok(DependencyAssessment {
                requirement: requirement.clone(),
                status,
                inventory_id: inventory.map(|inventory| inventory.id.clone()),
            })
        })
        .collect()
}

fn environment_records<'a>(
    project: &'a str,
    kind: DeliveryKind,
    target_kind: &'static str,
    target: &'a str,
) -> delivery_records::BoxedQuery<'a, DbBackend> {
    delivery_records::table
        .filter(delivery_records::project_id.eq(project))
        .filter(delivery_records::kind.eq(kind.as_str()))
        .filter(
            delivery_records::id.eq_any(
                delivery_targets::table
                    .filter(delivery_targets::project_id.eq(project))
                    .filter(delivery_targets::kind.eq(kind.as_str()))
                    .filter(delivery_targets::target_kind.eq(target_kind))
                    .filter(delivery_targets::target.eq(target))
                    .select(delivery_targets::subject_id),
            ),
        )
        .order((
            delivery_records::updated_at.desc(),
            delivery_records::id.asc(),
        ))
        .into_boxed()
}

pub(super) fn check_history(
    connection: &mut DbConnection,
    project: &str,
    repository: &str,
    check: &str,
    since: DateTime<Utc>,
) -> Result<DeliveryResponse> {
    text(repository, 512)?;
    text(check, 512)?;
    let rows = delivery_observations::table
        .filter(delivery_observations::project_id.eq(project))
        .filter(delivery_observations::subject_kind.eq("pull_request"))
        .filter(delivery_observations::occurred_at.ge(since.to_rfc3339()))
        .filter(
            delivery_observations::subject_id.eq_any(
                delivery_records::table
                    .filter(delivery_records::project_id.eq(project))
                    .filter(delivery_records::kind.eq("pull_request"))
                    .filter(delivery_records::repository.eq(repository))
                    .select(delivery_records::id),
            ),
        )
        .order((
            delivery_observations::occurred_at.desc(),
            delivery_observations::ordinal.desc(),
        ))
        .limit(513)
        .select(ObservationRow::as_select())
        .load::<ObservationRow>(connection)?;
    let partial = rows.len() > 512;
    let mut pull_request_ids = BTreeSet::new();
    for row in rows.into_iter().take(512) {
        let observation = row.observation()?;
        if let DeliveryEvent::PullRequestObserved {
            pull_request_id,
            snapshot,
        } = observation.input.body
        {
            if failed_check(&snapshot, repository, check, since) {
                pull_request_ids.insert(pull_request_id);
            }
        }
    }
    Ok(DeliveryResponse::CheckHistory {
        pull_request_ids: pull_request_ids.into_iter().collect(),
        partial,
    })
}

fn failed_check(
    snapshot: &PullRequestSnapshot,
    repository: &str,
    check: &str,
    since: DateTime<Utc>,
) -> bool {
    snapshot.observed_at >= since
        && snapshot.head.repository == repository
        && snapshot.checks.iter().any(|evidence| {
            evidence.name == check
                && evidence.state == CheckState::Failed
                && evidence.revision == snapshot.head
        })
}
