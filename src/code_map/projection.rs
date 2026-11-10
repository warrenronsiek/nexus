// @feature code-map
// @spec docs/features/code-map.md
use super::*;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Placement {
    pub key: String,
    pub label: String,
}

impl Placement {
    pub fn value(key: &str, label: &str) -> Self {
        Self {
            key: format!("value:{key}"),
            label: label.into(),
        }
    }

    fn special(key: &str, label: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

pub(crate) struct MapPoint {
    pub subject: Placement,
    pub value: f64,
    pub dimensions: BTreeMap<String, BTreeSet<Placement>>,
    pub evidence: MapEvidence,
}

impl From<MapSample> for MapPoint {
    fn from(sample: MapSample) -> Self {
        Self {
            subject: Placement::value(&sample.subject, &sample.subject),
            value: sample.value,
            dimensions: sample
                .dimensions
                .into_iter()
                .map(|(key, value)| (key, BTreeSet::from([Placement::value(&value, &value)])))
                .collect(),
            evidence: MapEvidence {
                subject: sample.subject,
                concern_id: None,
                reference: sample.evidence,
            },
        }
    }
}

impl MapPoint {
    fn placement(&self, dimension: &str) -> Placement {
        if dimension == "subject" {
            return self.subject.clone();
        }
        match self.dimensions.get(dimension) {
            Some(values) if values.len() > 1 => Placement::special("shared", "Shared"),
            Some(values) if !values.is_empty() => {
                values.first().expect("nonempty placement").clone()
            }
            _ => Placement::special("unattributed", "Unattributed"),
        }
    }
}

pub(crate) fn project(view: &mut MapView, points: &[MapPoint], bucket: Option<&str>) -> Result<()> {
    let mut groups = BTreeMap::<Placement, Vec<&MapPoint>>::new();
    for point in points {
        let placement = point.placement(&view.group_by);
        let leaf = match bucket {
            Some(key) if key == placement.key => {
                view.bucket_label.get_or_insert(placement.label);
                point.subject.clone()
            }
            Some(_) => continue,
            None => placement,
        };
        groups.entry(leaf).or_default().push(point);
    }
    view.bucket = bucket.map(str::to_owned);
    view.size_by_samples = view.reducer != MapReducer::Sum
        || points.iter().any(|point| point.value < 0.0)
        || !points.iter().any(|point| point.value > 0.0);
    view.groups = groups
        .into_iter()
        .map(|(placement, points)| rollup(view, placement, &points))
        .collect();
    bound_response(view)
}

fn bound_response(view: &mut MapView) -> Result<()> {
    const RESPONSE_BUDGET: usize = 768 * 1024;
    let mut groups = std::mem::take(&mut view.groups);
    groups.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.key.cmp(&b.key))
    });
    view.partial |= groups.len() > 64;
    groups.truncate(64);
    let header = serde_json::to_vec(view)?.len() + groups.len();
    let per_group = RESPONSE_BUDGET.saturating_sub(header) / groups.len().max(1);
    for mut group in groups {
        view.partial |= trim_evidence(&mut group, per_group)?;
        if serde_json::to_vec(&group)?.len() <= per_group {
            view.groups.push(group);
        } else {
            view.partial = true;
        }
    }
    Ok(())
}

fn trim_evidence(group: &mut MapGroup, budget: usize) -> Result<bool> {
    let mut bytes = serde_json::to_vec(group)?.len();
    let mut trimmed = false;
    while bytes > budget {
        let Some(evidence) = group.evidence.pop() else {
            break;
        };
        let separator = usize::from(!group.evidence.is_empty());
        bytes -= serde_json::to_vec(&evidence)?.len() + separator;
        trimmed = true;
    }
    Ok(trimmed)
}

fn rollup(view: &MapView, placement: Placement, points: &[&MapPoint]) -> MapGroup {
    let values = points.iter().map(|point| point.value);
    let value = match view.reducer {
        MapReducer::Sum => values.sum(),
        MapReducer::Max => values.reduce(f64::max).expect("nonempty group"),
        MapReducer::Mean => values.sum::<f64>() / points.len() as f64,
    };
    let mut concerns = HashSet::new();
    MapGroup {
        key: placement.key,
        label: placement.label,
        value,
        weight: if view.size_by_samples {
            points.len() as f64
        } else {
            value
        },
        samples: points.len(),
        evidence: points
            .iter()
            .filter(|point| {
                point
                    .evidence
                    .concern_id
                    .as_deref()
                    .is_none_or(|id| concerns.insert(id))
            })
            .take(6)
            .map(|point| point.evidence.clone())
            .collect(),
    }
}
