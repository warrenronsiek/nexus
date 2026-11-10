// @feature code-map
// @spec docs/features/code-map.md
// @entrypoint MapSnapshot
// @entrypoint MapQuery
mod projection;
mod validation;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) use projection::{project, MapPoint, Placement};
pub(crate) use validation::{validate_concern_dimensions, MAX_BYTES, MAX_POINTS};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MapReducer {
    #[default]
    Sum,
    Max,
    Mean,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapSnapshot {
    pub id: String,
    pub producer: String,
    pub measure: String,
    pub title: String,
    pub unit: String,
    pub reducer: MapReducer,
    pub revision: String,
    pub observed_at: DateTime<Utc>,
    pub samples: Vec<MapSample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapSample {
    pub subject: String,
    pub value: f64,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapMeasure {
    #[default]
    Encounters,
    OpenConcerns,
    Published {
        producer: String,
        measure: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MapQuery {
    pub measure: MapMeasure,
    pub group_by: Option<String>,
    pub bucket: Option<String>,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MapSource {
    pub id: String,
    pub revision: String,
    pub observed_at: DateTime<Utc>,
    pub reporter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MapView {
    pub measure: MapMeasure,
    pub title: String,
    pub unit: String,
    pub reducer: MapReducer,
    pub size_by_samples: bool,
    pub source: Option<MapSource>,
    pub measures: Vec<MapMeasure>,
    pub measures_partial: bool,
    pub dimensions: Vec<String>,
    pub group_by: String,
    pub bucket: Option<String>,
    pub bucket_label: Option<String>,
    pub groups: Vec<MapGroup>,
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MapGroup {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub weight: f64,
    pub samples: usize,
    pub evidence: Vec<MapEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MapEvidence {
    pub subject: String,
    pub concern_id: Option<String>,
    pub reference: Option<String>,
}
