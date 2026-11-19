// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::*;
use crate::persistence::{
    backend::DbBackend,
    schema::{delivery_features, delivery_targets},
};
type Selection<'a> = delivery_records::BoxedQuery<'a, DbBackend>;
pub(super) fn records<'a>(
    project: &'a str,
    query: &'a DeliveryQuery,
    recipient: Option<&'a AgentIdentity>,
) -> Result<Selection<'a>> {
    let mut selection = delivery_records::table
        .filter(delivery_records::project_id.eq(project))
        .into_boxed();
    if let Some(kind) = &query.kind {
        selection = selection.filter(delivery_records::kind.eq(kind.as_str()));
    }
    if let Some(repo) = &query.repository {
        selection = selection.filter(delivery_records::repository.eq(repo));
    }
    if let Some(since) = query.updated_since {
        selection = selection.filter(delivery_records::updated_at.ge(since.to_rfc3339()));
    }
    if let Some(state) = &query.state {
        selection = selection.filter(delivery_records::state.eq(state));
    }
    if let Some(agent) = recipient {
        selection = pending_for(selection, agent);
    }
    if let Some(feature) = &query.feature {
        selection = by_feature(selection, project, feature);
    }
    if let Some(target) = &query.target {
        selection = by_target(selection, project, target);
    }
    if let Some(branch) = &query.branch {
        selection = by_branch(selection, project, branch);
    }
    match &query.after {
        Some(after) => seek(selection, query, after),
        None => Ok(selection),
    }
}

fn seek<'a>(
    selection: Selection<'a>,
    query: &'a DeliveryQuery,
    after: &'a str,
) -> Result<Selection<'a>> {
    let (time, subject) = match query.updated_since {
        Some(_) => {
            let (time, subject) = cursor(after)?;
            (Some(time), subject)
        }
        None => (None, after),
    };
    let (kind, id) = cursor(subject)?;
    let after_subject = delivery_records::kind.gt(kind).or(delivery_records::kind
        .eq(kind)
        .and(delivery_records::id.gt(id)));
    Ok(match time {
        Some(time) => selection.filter(
            delivery_records::updated_at
                .lt(time)
                .or(delivery_records::updated_at.eq(time).and(after_subject)),
        ),
        None => selection.filter(after_subject),
    })
}

fn by_feature<'a>(selection: Selection<'a>, project: &'a str, feature: &'a str) -> Selection<'a> {
    let contributions = delivery_features::table
        .filter(delivery_features::project_id.eq(project))
        .filter(delivery_features::feature.eq(feature))
        .select(delivery_features::contribution_id);
    let linked = delivery_relations::table
        .filter(delivery_relations::project_id.eq(project))
        .filter(delivery_relations::target_kind.eq("contribution"))
        .filter(delivery_relations::target_id.eq_any(contributions));
    selection.filter(
        delivery_records::kind
            .eq("contribution")
            .and(
                delivery_records::id.eq_any(
                    delivery_features::table
                        .filter(delivery_features::project_id.eq(project))
                        .filter(delivery_features::feature.eq(feature))
                        .select(delivery_features::contribution_id),
                ),
            )
            .or(diesel::dsl::exists(
                linked
                    .filter(delivery_relations::source_kind.eq(delivery_records::kind))
                    .filter(delivery_relations::source_id.eq(delivery_records::id)),
            )),
    )
}
fn by_target<'a>(selection: Selection<'a>, project: &'a str, target: &'a str) -> Selection<'a> {
    selection.filter(diesel::dsl::exists(
        delivery_targets::table
            .filter(delivery_targets::project_id.eq(project))
            .filter(delivery_targets::kind.eq(delivery_records::kind))
            .filter(delivery_targets::subject_id.eq(delivery_records::id))
            .filter(delivery_targets::target.eq(target)),
    ))
}
fn by_branch<'a>(selection: Selection<'a>, project: &'a str, branch: &'a str) -> Selection<'a> {
    selection.filter(diesel::dsl::exists(
        delivery_targets::table
            .filter(delivery_targets::project_id.eq(project))
            .filter(delivery_targets::kind.eq(delivery_records::kind))
            .filter(delivery_targets::subject_id.eq(delivery_records::id))
            .filter(delivery_targets::target.eq(branch))
            .filter(delivery_targets::target_kind.eq("branch")),
    ))
}

fn pending_for<'a>(selection: Selection<'a>, agent: &'a AgentIdentity) -> Selection<'a> {
    selection
        .filter(delivery_records::recipient_agent.eq(&agent.agent))
        .filter(delivery_records::recipient_session.eq(&agent.session_id))
        .filter(delivery_records::state.ne_all(["resolved", "declined", "canceled"]))
}
