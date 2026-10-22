// @feature evals
// @spec docs/features/evals.md
use super::*;
use std::collections::HashSet;

macro_rules! validate {
    ($kind:ty,$value:ident,$body:block)=>{
        impl ValidatePayload for $kind {
            fn validate_payload(&self)->Result<()> {
                let $value=self;
                $body
                Ok(())
            }
        }
    };
}
validate!(Change, v, {
    match &v.motivation {
        Motivation::Recorded {
            problem,
            expected_behavior,
            rationale,
            ..
        } => {
            text("problem", problem)?;
            text("expected behavior", expected_behavior)?;
            text("rationale", rationale)?;
        }
        Motivation::Missing { reason } => text("missing historical rationale reason", reason)?,
    }
});
validate!(RepositoryRevision, v, {
    identity("repository", &v.repository)?;
    oid(&v.commit, v.object_format)?;
    for entry in &v.overrides {
        path(&entry.path)?;
    }
});
validate!(ModelRevision, v, {
    identity("model label", &v.label)?;
    identity("provider", &v.provider)?;
    identity("requested model", &v.requested_model)?;
});
validate!(CaseRevision, v, {
    text("expected behavior", &v.expected_behavior)?;
});
validate!(SuiteRevision, v, {
    let mut seen = HashSet::new();
    for member in &v.cases {
        if !member.weight.is_finite() || member.weight < 0.0 || !seen.insert(&member.case_id) {
            bail!("suite members must be unique with nonnegative finite weights");
        }
    }
    text("aggregation", &v.aggregation)?;
});
validate!(TargetRevision, v, {
    let mut seen = HashSet::new();
    for component in &v.components {
        identity("component id", &component.id)?;
        if !seen.insert(&component.id) {
            bail!("target component IDs must be unique");
        }
    }
    if v.components.is_empty() {
        bail!("target must declare at least one component");
    }
});
validate!(Experiment, v, {
    text("hypothesis", &v.hypothesis)?;
    text("experiment plan", &v.plan)?;
});
validate!(Run, v, {
    text("runner", &v.runner)?;
    text("selection rule", &v.selection_rule)?;
});
validate!(Attempt, v, {
    if v.finished_at.is_some_and(|end| end < v.started_at) {
        bail!("attempt finish precedes start");
    }
    if v.parent_id.is_some()
        && v.retry_reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        bail!("retry requires an explicit reason");
    }
});
validate!(Call, v, {
    identity("component", &v.component)?;
    identity("instance", &v.instance)?;
});
validate!(Grade, v, {
    identity("grading pass", &v.pass_id)?;
    if v.score.is_some_and(|score| !score.is_finite()) {
        bail!("grade score must be finite");
    }
});
validate!(Decision, v, {
    text("decision reason", &v.reason)?;
});
validate!(Selection, v, {
    identity("selection scope", &v.scope)?;
});
validate!(Artifact, v, {
    identity("media type", &v.media_type)?;
    if v.backings.is_empty() {
        bail!("artifact requires a Git/LFS reference or verified blob backing");
    }
    if v.digest
        .as_deref()
        .is_some_and(|digest| !digest_is_valid(digest))
    {
        bail!("artifact digest must be SHA-256");
    }
    for backing in &v.backings {
        validate_backing(backing)?;
    }
});
validate!(Verification, v, {
    identity("verification producer", &v.producer)?;
    validate_backing(&v.backing)?;
});
validate!(GraderRevision, _v, {});
validate!(Trial, _v, {});
validate!(Trace, _v, {});
validate!(Comparison, _v, {});
validate!(Lifecycle, _v, {});
