// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::*;
pub(super) fn record(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let DeliveryEvent::PullRequestCompositionRecorded {
        head,
        expected_composition,
        contribution_ids,
        ..
    } = &observation.input.body
    else {
        unreachable!("typed composition dispatch")
    };
    let project = observation.project_id.as_str();
    let mut record = required(
        connection,
        project,
        &observation.input.body.subject().unwrap(),
    )?;
    if record.composition_event_id != *expected_composition {
        bail!(DeliveryRejection::Conflict(
            "pull request composition changed; reread before updating membership".into()
        ))
    }
    let DeliveryBody::PullRequest { pull_request, .. } = &mut record.body else {
        unreachable!("pull request key")
    };
    if pull_request.head != *head {
        bail!(DeliveryRejection::Conflict(
            "pull request head changed; record composition for the current head".into()
        ))
    }
    pull_request.contribution_ids = contribution_ids.clone();
    replace_contribution_links(connection, project, &record.subject, contribution_ids)?;
    record.composition_event_id = Some(observation.input.id.clone());
    record.updated_at = observation.input.occurred_at;
    save(connection, project, &record)
}
fn replace_contribution_links(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
    ids: &[String],
) -> Result<()> {
    diesel::delete(
        delivery_relations::table
            .filter(delivery_relations::project_id.eq(project))
            .filter(delivery_relations::source_kind.eq(subject.kind.as_str()))
            .filter(delivery_relations::source_id.eq(&subject.id))
            .filter(delivery_relations::target_kind.eq("contribution")),
    )
    .execute(connection)?;
    for id in ids {
        link(connection, project, subject, DeliveryKind::Contribution, id)?;
    }
    Ok(())
}
