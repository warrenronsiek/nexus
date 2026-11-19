// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::{projection, rows::*, *};
use crate::persistence::schema::{delivery_presence, delivery_records, delivery_relations};
use crate::persistence::window::WindowBudget;
use anyhow::Context;
use serde::{
    de::{value::Error as LabelError, IntoDeserializer},
    Deserialize,
};
use std::collections::BTreeSet;
mod environment;
pub(super) mod origins;
mod selection;
fn limit(value: Option<u32>) -> i64 {
    i64::from(value.unwrap_or(24).clamp(1, 64))
}
fn cursor(value: &str) -> Result<(&str, &str)> {
    value.split_once('\0').context("invalid delivery cursor")
}
fn key(record: &DeliveryRecord, query: &DeliveryQuery) -> String {
    let subject = format!("{}\0{}", record.subject.kind.as_str(), record.subject.id);
    match query.updated_since {
        Some(_) => format!("{}\0{subject}", record.updated_at.to_rfc3339()),
        None => subject,
    }
}
pub(super) fn read(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    action: &DeliveryAction,
) -> Result<DeliveryResponse> {
    match action {
        DeliveryAction::List { query, .. } => list(connection, project, query, None),
        DeliveryAction::Inbox {
            recipient,
            after,
            limit,
            ..
        } => {
            let agent = recipient.clone().unwrap_or(AgentIdentity {
                agent: actor.agent.clone(),
                session_id: actor.session_id.clone(),
            });
            agent.validate()?;
            list(
                connection,
                project,
                &DeliveryQuery {
                    kind: Some(DeliveryKind::Remediation),
                    after: after.clone(),
                    limit: *limit,
                    ..Default::default()
                },
                Some(&agent),
            )
        }
        DeliveryAction::Get { subject, .. } => detail(connection, project, subject),
        DeliveryAction::ResolveOrigin { query, .. } => origins::resolve(connection, project, query),
        DeliveryAction::EnvironmentContext {
            environment: name,
            candidate,
            pull_request_id,
            ..
        } => environment::context(
            connection,
            project,
            name,
            candidate.as_ref(),
            pull_request_id.as_deref(),
        ),
        DeliveryAction::CheckHistory {
            repository,
            check,
            since,
            ..
        } => environment::check_history(connection, project, repository, check, *since),
        DeliveryAction::RemediationStatus { id, .. } => {
            super::remediation::status(connection, project, id)
        }
        DeliveryAction::Events {
            subject,
            after,
            limit,
            ..
        } => events(connection, project, Some(subject), after.as_deref(), *limit),
        DeliveryAction::Export { after, limit, .. } => {
            events(connection, project, None, after.as_deref(), *limit)
        }
        DeliveryAction::Watches { .. } => watches(connection, project),
        _ => unreachable!("writes dispatched before reads"),
    }
}
fn watches(connection: &mut DbConnection, project: &str) -> Result<DeliveryResponse> {
    let (mut watches, truncated) = crate::persistence::delivery_watch::list(connection, project)?;
    for watch in &mut watches {
        watch.promotion_readiness =
            environment::promotion_readiness(connection, project, &watch.registration)?;
    }
    Ok(DeliveryResponse::Watches { watches, truncated })
}

fn list(
    connection: &mut DbConnection,
    project: &str,
    query: &DeliveryQuery,
    recipient: Option<&AgentIdentity>,
) -> Result<DeliveryResponse> {
    let selection = selection::records(project, query, recipient)?;
    let cap = limit(query.limit);
    let ordered = match query.updated_since {
        Some(_) => selection.order((
            delivery_records::updated_at.desc(),
            delivery_records::kind.asc(),
            delivery_records::id.asc(),
        )),
        None => selection.order((delivery_records::kind.asc(), delivery_records::id.asc())),
    };
    let rows = ordered
        .limit(cap + 1)
        .select(RecordRow::as_select())
        .load(connection)?;
    let mut items = Vec::new();
    let mut budget = WindowBudget::new(cap as usize, 512 * 1024);
    let mut partial = rows.len() as i64 > cap;
    for row in rows.into_iter().take(cap as usize) {
        let record = enrich_presence(connection, project, row.record()?)?;
        if !budget.include(&record)? {
            partial = true;
            break;
        }
        items.push(record);
    }
    let next_cursor = if partial {
        items.last().map(|record| key(record, query))
    } else {
        None
    };
    Ok(DeliveryResponse::Page { items, next_cursor })
}
fn observations(
    connection: &mut DbConnection,
    project: &str,
    subject: Option<&DeliverySubject>,
    after: Option<&str>,
    cap: i64,
) -> Result<Vec<ObservationRow>> {
    let mut selection = delivery_observations::table
        .filter(delivery_observations::project_id.eq(project))
        .into_boxed();
    if let Some(subject) = subject {
        selection = selection
            .filter(delivery_observations::subject_kind.eq(subject.kind.as_str()))
            .filter(delivery_observations::subject_id.eq(&subject.id));
    }
    if let Some(after) = after {
        selection = selection.filter(
            delivery_observations::ordinal
                .gt(after.parse::<i64>().context("invalid event cursor")?),
        );
    }
    Ok(selection
        .order(delivery_observations::ordinal.asc())
        .limit(cap)
        .select(ObservationRow::as_select())
        .load(connection)?)
}
fn events(
    connection: &mut DbConnection,
    project: &str,
    subject: Option<&DeliverySubject>,
    after: Option<&str>,
    max: Option<u32>,
) -> Result<DeliveryResponse> {
    let cap = limit(max);
    let rows = observations(connection, project, subject, after, cap + 1)?;
    let mut partial = rows.len() as i64 > cap;
    let mut output = Vec::new();
    let mut budget = WindowBudget::new(cap as usize, 512 * 1024);
    let mut next = None;
    for row in rows.into_iter().take(cap as usize) {
        let observation = row.observation()?;
        if !budget.include(&observation)? {
            partial = true;
            break;
        }
        next = Some(row.ordinal.to_string());
        output.push(observation);
    }
    Ok(DeliveryResponse::Events {
        observations: output,
        next_cursor: if partial { next } else { None },
    })
}
fn related(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
) -> Result<(Vec<DeliveryRecord>, bool)> {
    let outgoing = relation_keys(connection, project, subject, RelationDirection::Outgoing)?;
    let incoming = relation_keys(connection, project, subject, RelationDirection::Incoming)?;
    let ids = outgoing
        .into_iter()
        .chain(incoming)
        .collect::<BTreeSet<_>>();
    let mut partial = ids.len() > 64;
    let mut records = Vec::new();
    let mut budget = WindowBudget::new(64, 128 * 1024);
    for (kind, id) in ids.into_iter().take(64) {
        let kind = DeliveryKind::deserialize(kind.into_deserializer())
            .map_err(|_: LabelError| anyhow::anyhow!("invalid stored delivery kind"))?;
        if let Some(record) = projection::load(connection, project, &DeliverySubject { kind, id })?
        {
            if !budget.include(&record)? {
                partial = true;
                break;
            }
            records.push(record);
        }
    }
    Ok((records, partial))
}
fn detail(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
) -> Result<DeliveryResponse> {
    subject.validate()?;
    let record = projection::load(connection, project, subject)?
        .map(|r| enrich_presence(connection, project, r).map(Box::new))
        .transpose()?;
    let (related, partial) = related(connection, project, subject)?;
    let preview = history_preview(connection, project, subject)?;
    Ok(DeliveryResponse::Detail {
        record,
        related,
        observations: preview.observations,
        truncated: partial || preview.truncated,
    })
}
struct HistoryPreview {
    observations: Vec<DeliveryObservation>,
    truncated: bool,
}
fn history_preview(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
) -> Result<HistoryPreview> {
    let history = observations(connection, project, Some(subject), None, 9)?;
    let mut result = HistoryPreview {
        observations: vec![],
        truncated: history.len() > 8,
    };
    let mut budget = WindowBudget::new(8, 128 * 1024);
    for row in history.into_iter().take(8) {
        let observation = row.observation()?;
        if !budget.include(&observation)? {
            result.truncated = true;
            break;
        }
        result.observations.push(observation);
    }
    Ok(result)
}
fn enrich_presence(
    connection: &mut DbConnection,
    project: &str,
    mut record: DeliveryRecord,
) -> Result<DeliveryRecord> {
    record.shepherd_presence = record
        .shepherd
        .as_ref()
        .map(|a| presence(connection, project, &a.agent))
        .transpose()?
        .flatten();
    Ok(record)
}
#[derive(Clone, Copy)]
enum RelationDirection {
    Incoming,
    Outgoing,
}
fn relation_keys(
    connection: &mut DbConnection,
    project: &str,
    subject: &DeliverySubject,
    direction: RelationDirection,
) -> Result<Vec<(String, String)>> {
    let selection = delivery_relations::table.filter(delivery_relations::project_id.eq(project));
    let rows = match direction {
        RelationDirection::Outgoing => selection
            .filter(delivery_relations::source_kind.eq(subject.kind.as_str()))
            .filter(delivery_relations::source_id.eq(&subject.id))
            .select((
                delivery_relations::target_kind,
                delivery_relations::target_id,
            ))
            .order((
                delivery_relations::target_kind.asc(),
                delivery_relations::target_id.asc(),
            ))
            .limit(65)
            .load(connection)?,
        RelationDirection::Incoming => selection
            .filter(delivery_relations::target_kind.eq(subject.kind.as_str()))
            .filter(delivery_relations::target_id.eq(&subject.id))
            .select((
                delivery_relations::source_kind,
                delivery_relations::source_id,
            ))
            .order((
                delivery_relations::source_kind.asc(),
                delivery_relations::source_id.asc(),
            ))
            .limit(65)
            .load(connection)?,
    };
    Ok(rows)
}

pub(super) fn presence(
    connection: &mut DbConnection,
    project: &str,
    agent: &AgentIdentity,
) -> Result<Option<AgentPresence>> {
    delivery_presence::table
        .find((project, &agent.agent, &agent.session_id))
        .select(delivery_presence::payload_json)
        .first::<String>(connection)
        .optional()?
        .map(|p| {
            let mut presence: AgentPresence = serde_json::from_str(&p)?;
            if presence.reachability == Reachability::Available
                && Utc::now()
                    .signed_duration_since(presence.observed_at)
                    .num_seconds()
                    > 300
            {
                presence.reachability = Reachability::Unknown;
            }
            Ok(presence)
        })
        .transpose()
}
