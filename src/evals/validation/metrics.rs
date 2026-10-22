// @feature evals
// @spec docs/features/evals.md
use super::*;

impl ValidatePayload for MetricRevision {
    fn validate_payload(&self) -> Result<()> {
        identity("metric name", &self.name)?;
        for (name, value) in [
            ("metric description", &self.description),
            ("metric unit", &self.unit),
            ("metric value format", &self.value_format),
        ] {
            text(name, value)?;
        }
        if let Some(interpretation) = &self.interpretation {
            text("metric interpretation", interpretation)?;
        }
        validate_calculation(&self.calculation)
    }
}

fn validate_calculation(value: &MetricCalculation) -> Result<()> {
    for (name, description) in [
        ("calculation description", &value.description),
        ("calculation population", &value.population),
        ("calculation aggregation", &value.aggregation),
        ("calculation exclusions", &value.exclusions),
        ("calculation missing data", &value.missing_data),
    ] {
        text(name, description)?;
    }
    if let Some(formula) = &value.formula {
        text("calculation formula", formula)?;
    }
    Ok(())
}

impl ValidatePayload for Measurement {
    fn validate_payload(&self) -> Result<()> {
        if let MeasurementValue::Unavailable { reason } = &self.value {
            text("unavailable measurement reason", reason)?;
        }
        Ok(())
    }
}

impl RecordReferences for MetricRevision {
    fn append_references(&self, refs: &mut Vec<Reference>) {
        optional(
            refs,
            "value_schema",
            &self.value_schema,
            Some(EvalKind::Artifact),
        );
        links(
            refs,
            "calculation_source",
            &self.calculation.source_artifacts,
            Some(EvalKind::Artifact),
        );
    }
}

impl RecordReferences for Measurement {
    fn append_references(&self, refs: &mut Vec<Reference>) {
        self.subject.append_references(refs, "measured_subject");
        link(
            refs,
            "metric_definition",
            &self.metric_id,
            Some(EvalKind::Metric),
        );
        links(refs, "measurement_input", &self.inputs, None);
        links(
            refs,
            "measurement_evidence",
            &self.evidence,
            Some(EvalKind::Artifact),
        );
        optional(
            refs,
            "remeasures",
            &self.replaces,
            Some(EvalKind::Measurement),
        );
        if let MeasurementValue::Artifact { artifact_id } = &self.value {
            link(
                refs,
                "measurement_value",
                artifact_id,
                Some(EvalKind::Artifact),
            );
        }
    }
}
