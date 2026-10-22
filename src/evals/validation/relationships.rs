// @feature evals
// @spec docs/features/evals.md
use super::*;

macro_rules! relationships {
    ($kind:ty, $value:ident, $refs:ident, $body:block) => {
        impl RecordReferences for $kind {
            fn append_references(&self, refs: &mut Vec<Reference>) {
                let $value = self;
                let $refs = refs;
                $body
            }
        }
    };
}
relationships!(Change, v, refs, {
    for evidence in &v.evidence {
        match evidence {
            EvidenceLink::Record { id } => link(refs, "evidence", id, None),
            EvidenceLink::External { snapshot, .. } => {
                optional(refs, "source_snapshot", snapshot, Some(EvalKind::Artifact))
            }
        }
    }
    links(refs, "supersedes", &v.supersedes, Some(EvalKind::Change));
});
relationships!(RepositoryRevision, v, refs, {
    for entry in &v.overrides {
        link(
            refs,
            "file_override",
            &entry.artifact_id,
            Some(EvalKind::Artifact),
        );
    }
});
relationships!(ModelRevision, _v, _refs, {});
relationships!(Artifact, _v, _refs, {});
relationships!(CaseRevision, v, refs, {
    links(refs, "input", &v.inputs, Some(EvalKind::Artifact));
    links(refs, "derived_from", &v.derived_from, Some(EvalKind::Case));
});
relationships!(SuiteRevision, v, refs, {
    for member in &v.cases {
        link(refs, "case", &member.case_id, Some(EvalKind::Case));
        links(refs, "grader", &member.grader_ids, Some(EvalKind::Grader));
    }
});
relationships!(GraderRevision, v, refs, {
    match &v.grading {
        GraderMethod::Code { artifact_id } => {
            link(refs, "grader_source", artifact_id, Some(EvalKind::Artifact))
        }
        GraderMethod::Rubric {
            artifact_id,
            model_id,
        } => {
            link(refs, "rubric", artifact_id, Some(EvalKind::Artifact));
            optional(refs, "judge_model", model_id, Some(EvalKind::Model));
        }
        GraderMethod::Human { .. } => {}
    }
    links(refs, "calibration", &v.calibration, None);
});
relationships!(TargetRevision, v, refs, {
    links(refs, "artifact", &v.artifacts, Some(EvalKind::Artifact));
    for component in &v.components {
        optional(
            refs,
            "repository",
            &component.repository_id,
            Some(EvalKind::Repository),
        );
        match &component.model {
            ModelAssignment::Fixed { model_id } => {
                link(refs, "model", model_id, Some(EvalKind::Model))
            }
            ModelAssignment::Policy { artifact_id } => {
                link(refs, "model_policy", artifact_id, Some(EvalKind::Artifact))
            }
            ModelAssignment::Unspecified { .. } => {}
        }
        links(
            refs,
            "component_artifact",
            &component.artifacts,
            Some(EvalKind::Artifact),
        );
    }
});
relationships!(Experiment, v, refs, {
    optional(
        refs,
        "parent_experiment",
        &v.parent_id,
        Some(EvalKind::Experiment),
    );
    link(refs, "baseline", &v.baseline_id, Some(EvalKind::Target));
    link(refs, "candidate", &v.candidate_id, Some(EvalKind::Target));
    link(refs, "suite", &v.suite_id, Some(EvalKind::Suite));
});
relationships!(Run, v, refs, {
    link(refs, "target", &v.target_id, Some(EvalKind::Target));
    link(refs, "suite", &v.suite_id, Some(EvalKind::Suite));
    optional(
        refs,
        "experiment",
        &v.experiment_id,
        Some(EvalKind::Experiment),
    );
    links(
        refs,
        "selected_case",
        &v.selected_cases,
        Some(EvalKind::Case),
    );
});
relationships!(Trial, v, refs, {
    link(refs, "run", &v.run_id, Some(EvalKind::Run));
    link(refs, "case", &v.case_id, Some(EvalKind::Case));
    links(refs, "artifact", &v.artifacts, Some(EvalKind::Artifact));
});
relationships!(Attempt, v, refs, {
    link(refs, "trial", &v.trial_id, Some(EvalKind::Trial));
    optional(refs, "retry_of", &v.parent_id, Some(EvalKind::Attempt));
    links(refs, "artifact", &v.artifacts, Some(EvalKind::Artifact));
});
relationships!(Call, v, refs, {
    link(refs, "attempt", &v.attempt_id, Some(EvalKind::Attempt));
    optional(refs, "parent_call", &v.parent_id, Some(EvalKind::Call));
    link(refs, "model", &v.model_id, Some(EvalKind::Model));
    for entry in &v.context {
        link(
            refs,
            "context",
            &entry.artifact_id,
            Some(EvalKind::Artifact),
        );
    }
    for tool in &v.tools {
        optional(
            refs,
            "tool_definition",
            &tool.definition_artifact,
            Some(EvalKind::Artifact),
        );
    }
    links(
        refs,
        "response",
        &v.response_artifacts,
        Some(EvalKind::Artifact),
    );
});
relationships!(Trace, v, refs, {
    link(refs, "attempt", &v.attempt_id, Some(EvalKind::Attempt));
    optional(refs, "call", &v.call_id, Some(EvalKind::Call));
    links(refs, "cause", &v.causes, None);
    match &v.detail {
        TraceEvent::Tool {
            input_artifact,
            output_artifact,
            ..
        } => {
            optional(refs, "tool_input", input_artifact, Some(EvalKind::Artifact));
            optional(
                refs,
                "tool_output",
                output_artifact,
                Some(EvalKind::Artifact),
            );
        }
        TraceEvent::Handoff {
            payload_artifact, ..
        } => optional(
            refs,
            "dispatch_payload",
            payload_artifact,
            Some(EvalKind::Artifact),
        ),
        TraceEvent::Outcome {
            evidence_artifacts, ..
        } => links(
            refs,
            "outcome",
            evidence_artifacts,
            Some(EvalKind::Artifact),
        ),
    }
});
impl AssessmentSubject {
    pub(super) fn append_references(&self, refs: &mut Vec<Reference>, relation: &str) {
        match self {
            Self::Run { id } => link(refs, relation, id, Some(EvalKind::Run)),
            Self::Trial { id } => link(refs, relation, id, Some(EvalKind::Trial)),
            Self::Attempt { id } => link(refs, relation, id, Some(EvalKind::Attempt)),
            Self::Call { id } => link(refs, relation, id, Some(EvalKind::Call)),
            Self::Component {
                attempt_id,
                call_ids,
                ..
            } => {
                link(refs, relation, attempt_id, Some(EvalKind::Attempt));
                links(refs, "component_call", call_ids, Some(EvalKind::Call));
            }
            Self::Routing { trace_id } => link(refs, relation, trace_id, Some(EvalKind::Trace)),
        }
    }
}
relationships!(Grade, v, refs, {
    v.subject.append_references(refs, "subject");
    link(refs, "grader", &v.grader_id, Some(EvalKind::Grader));
    links(
        refs,
        "grade_evidence",
        &v.evidence,
        Some(EvalKind::Artifact),
    );
    optional(refs, "regrades", &v.replaces, Some(EvalKind::Grade));
});
relationships!(Comparison, v, refs, {
    links(refs, "baseline", &v.baseline_ids, None);
    links(refs, "candidate", &v.candidate_ids, None);
    links(refs, "grade", &v.grade_ids, Some(EvalKind::Grade));
    links(
        refs,
        "metric_definition",
        &v.metric_ids,
        Some(EvalKind::Metric),
    );
    links(
        refs,
        "measurement",
        &v.measurement_ids,
        Some(EvalKind::Measurement),
    );
});
relationships!(Decision, v, refs, {
    link(refs, "subject", &v.subject_id, None);
    links(
        refs,
        "comparison",
        &v.comparison_ids,
        Some(EvalKind::Comparison),
    );
    optional(refs, "supersedes", &v.supersedes, Some(EvalKind::Decision));
});
relationships!(Selection, v, refs, {
    optional(refs, "selected_revision", &v.revision_id, None);
    optional(
        refs,
        "previous_selection",
        &v.previous_id,
        Some(EvalKind::Selection),
    );
    link(refs, "motivation", &v.change_id, Some(EvalKind::Change));
});
relationships!(Verification, v, refs, {
    link(
        refs,
        "verified_artifact",
        &v.artifact_id,
        Some(EvalKind::Artifact),
    );
});
relationships!(Lifecycle, v, refs, {
    let kind = match &v.subject {
        LifecycleSubject::Run { .. } => EvalKind::Run,
        LifecycleSubject::Trial { .. } => EvalKind::Trial,
        LifecycleSubject::Attempt { .. } => EvalKind::Attempt,
    };
    link(refs, "lifecycle_subject", v.subject.record_id(), Some(kind));
    optional(
        refs,
        "previous_lifecycle",
        &v.previous_id,
        Some(EvalKind::Lifecycle),
    );
    links(
        refs,
        "lifecycle_evidence",
        &v.artifacts,
        Some(EvalKind::Artifact),
    );
});
