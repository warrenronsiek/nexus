// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
use super::*;

pub(in super::super) fn action(command: ConcernCliCommand) -> Result<ConcernAction> {
    if let ConcernCliCommand::Respond(args) = command {
        return respond(args);
    }
    let observation = match command {
        ConcernCliCommand::Report(args) => report(args),
        ConcernCliCommand::Upvote(args) => upvote(args),
        command @ (ConcernCliCommand::Remedy(_) | ConcernCliCommand::Review(_)) => {
            decision_note(command)
        }
        ConcernCliCommand::Evidence(args) => attach(args),
        ConcernCliCommand::Resolve(args) => resolve(args),
        ConcernCliCommand::Reopen(args) => reopen(args),
        _ => unreachable!("nonrecording command selected earlier"),
    }?;
    observation.validate()?;
    Ok(ConcernAction::Record {
        observations: vec![observation],
    })
}
fn report(args: ReportArgs) -> Result<ConcernObservationInput> {
    Ok(args.observation.build(
        args.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        ConcernEvent::Reported {
            title: args.title,
            description: args.description,
            hypothesis: args.hypothesis,
            impact: args.impact,
            encounter: args.encounter.build()?,
        },
    ))
}
fn upvote(args: UpvoteArgs) -> Result<ConcernObservationInput> {
    let mut encounter = args.encounter.build()?;
    if let Some(note) = args.note {
        encounter.symptom = Some(note)
    }
    Ok(args.observation.build(
        args.concern_id,
        ConcernEvent::Encountered {
            encounter,
            fix_relation: args.fix_relation.map(|v| json_enum(&v)).transpose()?,
        },
    ))
}
fn decision_note(command: ConcernCliCommand) -> Result<ConcernObservationInput> {
    let (metadata, id, event) = match command {
        ConcernCliCommand::Remedy(args) => (
            args.observation,
            args.concern_id,
            ConcernEvent::RemedyRecorded {
                encounter_ids: args.encounter_id,
                what: args.what,
                why: args.why,
                outcome: args.outcome,
                evidence: evidence(args.evidence_json)?,
            },
        ),
        ConcernCliCommand::Review(args) => (
            args.observation,
            args.concern_id,
            ConcernEvent::ReviewRecorded {
                encounter_ids: args.encounter_id,
                hypothesis: args.hypothesis,
                rationale: args.rationale,
                alternatives: args.alternative,
                evidence: evidence(args.evidence_json)?,
            },
        ),
        _ => unreachable!("only evidence-backed decision notes"),
    };
    Ok(metadata.build(id, event))
}
fn attach(args: EvidenceArgs) -> Result<ConcernObservationInput> {
    Ok(args.observation.build(
        args.concern_id,
        ConcernEvent::EvidenceAdded {
            encounter_id: args.encounter_id,
            note: args.note,
            evidence: evidence(args.evidence_json)?,
        },
    ))
}
fn resolve(args: ResolveArgs) -> Result<ConcernObservationInput> {
    Ok(args.observation.build(
        args.concern_id,
        ConcernEvent::Resolved {
            reason: args.reason,
            fixing_location: args
                .fixing_location_json
                .as_deref()
                .map(parse_json)
                .transpose()?,
            evidence: evidence(args.evidence_json)?,
        },
    ))
}
fn reopen(args: ReopenArgs) -> Result<ConcernObservationInput> {
    Ok(args.observation.build(
        args.concern_id,
        ConcernEvent::Reopened {
            reason: args.reason,
            evidence: evidence(args.evidence_json)?,
        },
    ))
}
fn respond(args: RespondArgs) -> Result<ConcernAction> {
    let reporter_token = String::from_utf8(read(&args.token_file, 1024)?)?
        .trim()
        .to_owned();
    Ok(ConcernAction::Respond {
        reporter_token,
        observation: Box::new(args.observation.build(
            args.concern_id,
            ConcernEvent::DuplicateResponded {
                proposal_id: args.proposal_id,
                decision: json_enum(&args.decision)?,
                reason: args.reason,
            },
        )),
    })
}
