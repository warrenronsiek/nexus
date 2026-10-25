// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
// @boundary chart-numbers: caller text is never changed; conservative optional chart approximations reject precision loss and non-finite values.
use super::RecordCache;
use crate::evals::*;
use anyhow::Result;
use std::collections::BTreeMap;
pub(super) fn observe(
    cache: &mut RecordCache<'_>,
    series: &mut BTreeMap<String, EvalDashboardSeries>,
    record: &EvalRecordInput,
    run: &EvalRecordInput,
    superseded: bool,
) -> Result<()> {
    let Some((definition_id, subject, value, replaces)) = result_value(record) else {
        return Ok(());
    };
    let Some(context) = cache.get(definition_id)?.as_ref().and_then(metric_context) else {
        return Ok(());
    };
    let scope = subject_scope(subject);
    let key = format!("{definition_id}:{scope}");
    let entry = series
        .entry(key.clone())
        .or_insert_with(|| EvalDashboardSeries {
            id: key,
            label: context.label,
            unit: context.unit,
            definition_id: definition_id.into(),
            description: context.description,
            calculation: context.calculation,
            subject_scope: scope,
            superseded_count: 0,
            points: vec![],
        });
    if superseded {
        entry.superseded_count += 1;
        return Ok(());
    }
    let EvalBody::Run(run_body) = &run.body else {
        return Ok(());
    };
    let numeric_value = match &value {
        MeasurementValue::Inline { text } if context.numeric => chart_number(text),
        _ => None,
    };
    entry.points.push(EvalDashboardPoint {
        record_id: record.id.clone(),
        run_id: run.id.clone(),
        suite_revision_id: run_body.suite_id.clone(),
        target_revision_id: run_body.target_id.clone(),
        occurred_at: record.occurred_at,
        value,
        numeric_value,
        replaces,
    });
    Ok(())
}
type ResultValue<'a> = (
    &'a str,
    &'a AssessmentSubject,
    MeasurementValue,
    Option<String>,
);
fn result_value(record: &EvalRecordInput) -> Option<ResultValue<'_>> {
    match &record.body {
        EvalBody::Measurement(value) => Some((
            &value.metric_id,
            &value.subject,
            value.value.clone(),
            value.replaces.clone(),
        )),
        EvalBody::Grade(value) => Some((
            &value.grader_id,
            &value.subject,
            value.score.map_or_else(
                || MeasurementValue::Unavailable {
                    reason: "No numeric grade reported".into(),
                },
                |score| MeasurementValue::Inline {
                    text: score.to_string(),
                },
            ),
            value.replaces.clone(),
        )),
        _ => None,
    }
}
fn subject_scope(subject: &AssessmentSubject) -> String {
    match subject {
        AssessmentSubject::Run { .. } => "run".into(),
        AssessmentSubject::Trial { .. } => "trial".into(),
        AssessmentSubject::Attempt { .. } => "attempt".into(),
        AssessmentSubject::Call { .. } => "call".into(),
        AssessmentSubject::Component { instance, .. } => format!("component:{instance}"),
        AssessmentSubject::Routing { .. } => "routing".into(),
    }
}
struct MetricPresentation {
    label: String,
    unit: String,
    description: String,
    calculation: String,
    numeric: bool,
}
fn metric_context(record: &EvalRecordInput) -> Option<MetricPresentation> {
    match &record.body {
        EvalBody::Metric(value) => Some(MetricPresentation {
            label: value.name.clone(),
            unit: value.unit.clone(),
            description: value.description.clone(),
            calculation: serde_json::to_string(&value.calculation).ok()?,
            numeric: matches!(
                value.value_format.trim().to_ascii_lowercase().as_str(),
                "number" | "decimal" | "integer" | "float" | "numeric" | "json number"
            ),
        }),
        EvalBody::Grader(value) => Some(MetricPresentation {
            label: value.metric.name.clone(),
            unit: value.metric.unit.clone(),
            description: format!("Caller grade using immutable grader {}", record.id),
            calculation: serde_json::to_string(&value.grading).ok()?,
            numeric: true,
        }),
        _ => None,
    }
}

fn chart_number(text: &str) -> Option<f64> {
    let number: serde_json::Number = serde_json::from_str(text).ok()?;
    if number.to_string() != text {
        return None;
    }
    let value = number.as_f64()?;
    if !value.is_finite() || value.abs() > 9_007_199_254_740_991.0 {
        return None;
    }
    Some(value)
}
pub(super) fn bound_output(view: &mut EvalDashboard) -> Result<()> {
    let mut limited = false;
    view.series
        .sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    for series in &mut view.series {
        series.points.sort_by(|a, b| {
            b.occurred_at
                .cmp(&a.occurred_at)
                .then(a.record_id.cmp(&b.record_id))
        });
        limited |= bound(&mut series.points, 512, 256 * 1024)?;
        series.points.reverse();
    }
    limited |= bound(&mut view.series, 64, 448 * 1024)?;
    for group in &mut view.groups {
        limited |= bound_group(group)?;
    }
    if let Some(index) = view
        .groups
        .iter()
        .position(|group| Some(&group.id) == view.selected_group_id.as_ref())
    {
        let selected = view.groups.remove(index);
        view.groups.insert(0, selected);
    }
    limited |= bound(&mut view.groups, 512, 192 * 1024)?;
    if limited {
        view.truncated = true;
        view.notices.push("display_limit: groups/series/points are partial, keeping recent results; aggregate counts retain the scanned population".into());
    }
    Ok(())
}
fn bound_group(group: &mut EvalDashboardGroup) -> Result<bool> {
    let mut limited = bound(&mut group.components, 64, 16 * 1024)?;
    limited |= bound(&mut group.suite_revision_ids, 128, 32 * 1024)?;
    limited |= bound(&mut group.target_revision_ids, 128, 32 * 1024)?;
    limited |= bound(&mut group.repository, 128, 32 * 1024)?;
    limited |= bound(&mut group.models, 128, 32 * 1024)?;
    Ok(limited)
}
fn bound<T: serde::Serialize>(items: &mut Vec<T>, limit: usize, byte_limit: usize) -> Result<bool> {
    let mut bytes = 0;
    let mut kept = 0;
    for item in items.iter().take(limit) {
        let length = serde_json::to_vec(item)?.len();
        if bytes + length > byte_limit {
            break;
        }
        bytes += length;
        kept += 1;
    }
    let truncated = kept < items.len();
    items.truncate(kept);
    Ok(truncated)
}
