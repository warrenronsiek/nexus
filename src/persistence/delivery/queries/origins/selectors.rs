// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::*;
use crate::work_history::GitRevision;

pub(super) fn attempts(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
) -> Result<Vec<(OriginBasis, OriginQuery)>> {
    let revision = match &query.revision_prefix {
        Some(prefix) => expand_revision(connection, project, query, prefix)?,
        None => query.revision.clone(),
    };
    let pr_id = match query.pull_request_number {
        Some(number) => provider_pull_request(connection, project, query, number)?,
        None => query.pull_request_id.clone(),
    };
    let pinned = OriginQuery {
        revision,
        pull_request_id: pr_id,
        revision_prefix: None,
        pull_request_number: None,
        pull_request_provider: None,
        ..query.clone()
    };
    let fallback = query.fallback
        || query.branch.is_some()
        || query.pull_request_number.is_some()
        || query.revision_prefix.is_some();
    Ok(if fallback {
        fallback_queries(&pinned, query)
    } else {
        vec![(basis(&pinned), pinned)]
    })
}

fn basis(query: &OriginQuery) -> OriginBasis {
    match query {
        q if q.revision.is_some() => OriginBasis::Revision,
        q if q.fingerprint.is_some() => OriginBasis::Fingerprint,
        q if q.pull_request_id.is_some() => OriginBasis::PullRequest,
        q if q.rollout_id.is_some() => OriginBasis::Rollout,
        q if q.branch.is_some() => OriginBasis::Branch,
        q if q.feature.is_some() => OriginBasis::Feature,
        _ => OriginBasis::Repository,
    }
}

fn fallback_queries(
    pinned: &OriginQuery,
    supplied: &OriginQuery,
) -> Vec<(OriginBasis, OriginQuery)> {
    let base = OriginQuery {
        repository: pinned
            .repository
            .clone()
            .or_else(|| pinned.revision.as_ref().map(|r| r.repository.clone())),
        limit: pinned.limit,
        ..Default::default()
    };
    let code = OriginQuery {
        revision: pinned.revision.clone(),
        fingerprint: pinned.fingerprint.clone(),
        path: pinned.path.clone(),
        line_start: pinned.line_start,
        line_end: pinned.line_end,
        ..base.clone()
    };
    let mut present = fallback_subjects(pinned, &base);
    if code.revision.is_some() || code.fingerprint.is_some() {
        present.insert(0, (basis(&code), code));
    }
    if present.is_empty()
        && supplied.revision_prefix.is_none()
        && supplied.pull_request_number.is_none()
    {
        return vec![(OriginBasis::Repository, base)];
    }
    present
}

fn fallback_subjects(pinned: &OriginQuery, base: &OriginQuery) -> Vec<(OriginBasis, OriginQuery)> {
    let selectors = [
        (OriginBasis::PullRequest, &pinned.pull_request_id),
        (OriginBasis::Rollout, &pinned.rollout_id),
        (OriginBasis::Branch, &pinned.branch),
        (OriginBasis::Feature, &pinned.feature),
    ];
    selectors
        .into_iter()
        .filter_map(|(basis, value)| {
            value.as_ref().map(|value| {
                let value = Some(value.clone());
                let mut query = base.clone();
                match basis {
                    OriginBasis::PullRequest => query.pull_request_id = value,
                    OriginBasis::Rollout => query.rollout_id = value,
                    OriginBasis::Branch => query.branch = value,
                    OriginBasis::Feature => query.feature = value,
                    _ => unreachable!("composition or broad origin"),
                }
                (basis, query)
            })
        })
        .collect()
}

pub(in crate::persistence::delivery) fn provider_pull_request(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
    number: u64,
) -> Result<Option<String>> {
    let provider = query.pull_request_provider.as_deref().unwrap_or("github");
    let repository = query.repository.as_deref().expect("validated repository");
    let identity = crate::delivery::provider_pr_identity(provider, repository, number);
    Ok(delivery_targets::table
        .filter(delivery_targets::project_id.eq(project))
        .filter(delivery_targets::target_kind.eq("provider_pr"))
        .filter(delivery_targets::target.eq(identity))
        .select(delivery_targets::subject_id)
        .first(connection)
        .optional()?)
}

fn expand_revision(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
    prefix: &str,
) -> Result<Option<GitRevision>> {
    let repository = query.repository.as_deref().expect("validated repository");
    let prefix = prefix.to_ascii_lowercase();
    let commits = revision_targets(project, repository)
        .filter(delivery_targets::target.ge(&prefix))
        .filter(delivery_targets::target.lt(format!("{prefix}g")))
        .select(delivery_targets::target)
        .distinct()
        .limit(2)
        .load::<String>(connection)?;
    match commits.as_slice() {
        [] => Ok(None),
        [commit] => Ok(Some(GitRevision {
            repository: repository.into(),
            object_format: if commit.len() == 64 { "sha256" } else { "sha1" }.into(),
            commit: commit.clone(),
        })),
        _ => Err(DeliveryRejection::Invalid(
            "ambiguous revision prefix; supply more digits".into(),
        )
        .into()),
    }
}

pub(super) fn revision_targets<'a>(
    project: &'a str,
    repository: &'a str,
) -> delivery_targets::BoxedQuery<'a, crate::persistence::backend::DbBackend> {
    delivery_targets::table
        .filter(delivery_targets::project_id.eq(project))
        .filter(delivery_targets::kind.eq("contribution"))
        .filter(delivery_targets::target_kind.eq("revision"))
        .filter(
            delivery_targets::subject_id.eq_any(
                delivery_records::table
                    .filter(delivery_records::project_id.eq(project))
                    .filter(delivery_records::kind.eq("contribution"))
                    .filter(delivery_records::repository.eq(repository))
                    .select(delivery_records::id),
            ),
        )
        .into_boxed()
}
