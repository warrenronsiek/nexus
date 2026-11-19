// @feature persistence
// @feature delivery-coordination
// @feature code-concerns
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
// @spec docs/features/code-concerns.md
use super::{projection, queries, rows::RecordRow, *};
use crate::persistence::schema::{delivery_records, sessions};
use serde::Serialize;
mod concern;

pub(super) fn request(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    draft: &RemediationDraft,
) -> Result<DeliveryResponse> {
    let id = draft
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let fingerprint = fingerprint(&(actor.agent.as_str(), actor.session_id.as_str(), draft))?;
    match find(connection, project, &id)? {
        Some((prior, state)) if prior.request_fingerprint.as_deref() == Some(&fingerprint) => {
            response(connection, project, prior, state)
        }
        Some(_) => Err(DeliveryRejection::Conflict(id).into()),
        None => create_request(
            connection,
            project,
            actor,
            draft,
            RequestKey { id, fingerprint },
        ),
    }
}

struct RequestKey {
    id: String,
    fingerprint: String,
}

fn create_request(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    draft: &RemediationDraft,
    key: RequestKey,
) -> Result<DeliveryResponse> {
    let remediation = resolve_request(connection, project, draft, key)?;
    let input = observation(
        "request",
        &remediation.id,
        DeliveryEvent::RemediationRequested {
            remediation: remediation.clone(),
        },
    );
    record(connection, project, actor, &[input], &[])?;
    response(
        connection,
        project,
        remediation,
        RemediationState::Requested,
    )
}

fn resolve_request(
    connection: &mut DbConnection,
    project: &str,
    draft: &RemediationDraft,
    key: RequestKey,
) -> Result<Remediation> {
    let query = concern::origin(connection, project, draft)?;
    if !query.has_selector() {
        bail!(DeliveryRejection::Invalid(
            "remediation requires an origin selector".into()
        ));
    }
    let DeliveryResponse::Origins {
        matches,
        gaps,
        matched_by,
        qualification,
        ..
    } = queries::origins::resolve(connection, project, &query)?
    else {
        unreachable!("origin query returns origin evidence")
    };
    let matched = select_origin(&matches, &gaps, draft.recipient.as_ref())?;
    let contribution = &matched.contribution;
    let pr_id = match query.pull_request_number {
        Some(number) => {
            queries::origins::provider_pull_request(connection, project, &query, number)?
        }
        None => query.pull_request_id,
    };
    Ok(Remediation {
        id: key.id,
        repository: contribution.repository.clone(),
        title: draft.title.clone(),
        recipient: contribution.origin.clone(),
        pull_request_id: pr_id,
        rollout_id: query.rollout_id,
        contribution_id: Some(contribution.id.clone()),
        deployment_id: None,
        concern_id: draft.concern_id.clone(),
        activity_id: contribution.activity_id.clone(),
        failure_revision: query
            .revision
            .or_else(|| matched.mapping.as_ref().map(|m| m.revision.clone())),
        evidence: draft.evidence.clone(),
        request: draft.message.clone(),
        request_fingerprint: Some(key.fingerprint),
        origin_evidence: matched_by.map(|matched_by| RemediationOrigin {
            matched_by,
            qualification,
        }),
    })
}

fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}

fn select_origin<'a>(
    matches: &'a [OriginMatch],
    gaps: &[String],
    recipient: Option<&AgentIdentity>,
) -> Result<&'a OriginMatch> {
    let error = |message: &str| DeliveryRejection::Invalid(message.into());
    let mut eligible = matches
        .iter()
        .filter(|item| recipient.is_none_or(|recipient| &item.contribution.origin == recipient));
    let first = eligible
        .next()
        .ok_or_else(|| error("no matching origin for remediation"))?;
    if recipient.is_none()
        && (!gaps.is_empty()
            || eligible.any(|item| {
                item.contribution.origin != first.contribution.origin
                    || item.contribution.repository != first.contribution.repository
            }))
    {
        return Err(error(
            "origin is ambiguous or partial; resolve the evidence and select a recipient",
        )
        .into());
    }
    if recipient.is_some()
        && eligible.any(|item| item.contribution.repository != first.contribution.repository)
    {
        return Err(error(
            "the recipient has matches in several repositories; select one repository",
        )
        .into());
    }
    Ok(first)
}

pub(super) fn acknowledge(
    connection: &mut DbConnection,
    project: &str,
    actor: &WorkActor,
    id: &str,
) -> Result<DeliveryResponse> {
    let (remediation, state) = required(connection, project, id)?;
    if actor.agent != remediation.recipient.agent
        || actor.session_id != remediation.recipient.session_id
    {
        bail!(DeliveryRejection::Invalid(
            "only the recorded recipient can acknowledge receipt".into()
        ));
    }
    if state == RemediationState::Requested {
        let input = observation(
            "ack",
            id,
            DeliveryEvent::RemediationTransitioned {
                remediation_id: id.into(),
                expected_state: state,
                state: RemediationState::Acknowledged,
                reason: "Recipient confirmed receipt".into(),
                fixing_revision: None,
            },
        );
        record(connection, project, actor, &[input], &[])?;
    }
    status(connection, project, id)
}

fn observation(operation: &str, id: &str, body: DeliveryEvent) -> DeliveryObservationInput {
    DeliveryObservationInput {
        id: operation_id(operation, id),
        occurred_at: Utc::now(),
        body,
    }
}

fn operation_id(operation: &str, id: &str) -> String {
    crate::identity::stable_id(&["delivery-remediation", operation, id])
}

fn find(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
) -> Result<Option<(Remediation, RemediationState)>> {
    let subject = DeliverySubject {
        kind: DeliveryKind::Remediation,
        id: id.into(),
    };
    subject.validate()?;
    Ok(
        projection::load(connection, project, &subject)?.map(|record| {
            let DeliveryBody::Remediation {
                remediation, state, ..
            } = record.body
            else {
                unreachable!("remediation subject has a remediation body");
            };
            (remediation, state)
        }),
    )
}

fn required(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
) -> Result<(Remediation, RemediationState)> {
    find(connection, project, id)?
        .ok_or_else(|| DeliveryRejection::Invalid("remediation not found".into()).into())
}

pub(super) fn status(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
) -> Result<DeliveryResponse> {
    let (remediation, state) = required(connection, project, id)?;
    response(connection, project, remediation, state)
}

fn response(
    connection: &mut DbConnection,
    project: &str,
    remediation: Remediation,
    state: RemediationState,
) -> Result<DeliveryResponse> {
    let receipt = delivery_observations::table
        .find((project, operation_id("ack", &remediation.id)))
        .select(ObservationRow::as_select())
        .first::<ObservationRow>(connection)
        .optional()?;
    let verified = receipt
        .map(|receipt| receipt.observation())
        .transpose()?
        .is_some_and(|receipt| {
            let sender = receipt
                .source
                .as_ref()
                .map_or(&receipt.provenance, |source| &source.provenance);
            sender.agent == remediation.recipient.agent
                && sender.session_id == remediation.recipient.session_id
                && matches!(receipt.input.body, DeliveryEvent::RemediationTransitioned {
                remediation_id, state: RemediationState::Acknowledged, ..
            } if remediation_id == remediation.id)
        });
    let delivery = match (verified, state) {
        (true, _) => RemediationDelivery::Received,
        (false, RemediationState::Requested) => RemediationDelivery::Queued,
        _ => RemediationDelivery::Unknown,
    };
    let presence = queries::presence(connection, project, &remediation.recipient)?;
    let recipient_last_seen_at = sessions::table
        .find((project, &remediation.recipient.session_id))
        .filter(sessions::agent.eq(&remediation.recipient.agent))
        .select(sessions::last_seen_at)
        .first::<String>(connection)
        .optional()?
        .map(|value| value.parse())
        .transpose()?;
    Ok(DeliveryResponse::Remediation {
        recipient_last_seen_at,
        remediation: Box::new(remediation),
        state,
        delivery,
        presence,
    })
}

pub(super) fn pending_notices(
    connection: &mut DbConnection,
    project: &str,
    agent: &str,
    session: &str,
) -> Result<Vec<RemediationNotice>> {
    let rows = delivery_records::table
        .filter(delivery_records::project_id.eq(project))
        .filter(delivery_records::kind.eq("remediation"))
        .filter(delivery_records::recipient_agent.eq(agent))
        .filter(delivery_records::recipient_session.eq(session))
        .filter(delivery_records::state.eq("requested"))
        .order(delivery_records::id.asc())
        .limit(4)
        .select(RecordRow::as_select())
        .load::<RecordRow>(connection)?;
    rows.into_iter()
        .map(|row| {
            let DeliveryBody::Remediation { remediation, .. } = row.record()?.body else {
                unreachable!("remediation row has a remediation body");
            };
            Ok(RemediationNotice {
                id: remediation.id,
                title: remediation.title,
                message: crate::coordination::domain::truncate_utf8(remediation.request, 2048),
            })
        })
        .collect()
}
