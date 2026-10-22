// @feature evals
// @spec docs/features/evals.md
// @boundary caller-measured-text: inline values are opaque caller representations; Nexus never parses or coerces their declared format.
use super::{AssessmentSubject, Revision};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// The caller's calculation recipe. Nexus records this description without executing it.
pub struct MetricCalculation {
    pub description: String,
    pub formula: Option<String>,
    #[serde(default)]
    pub source_artifacts: Vec<String>,
    pub population: String,
    pub aggregation: String,
    pub exclusions: String,
    pub missing_data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Immutable meaning and representation of a caller-defined metric, independent of its objective.
pub struct MetricRevision {
    #[serde(flatten)]
    pub revision: Revision,
    pub name: String,
    pub description: String,
    /// Any caller-defined unit, including seconds, currencies, categories or unitless quantities.
    pub unit: String,
    /// Describes how to read the exact value text or artifact, such as decimal, JSON or CSV.
    pub value_format: String,
    /// Optional format documentation; Nexus does not validate measurements against this artifact.
    pub value_schema: Option<String>,
    pub calculation: MetricCalculation,
    pub interpretation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "storage", rename_all = "snake_case", deny_unknown_fields)]
pub enum MeasurementValue {
    /// Exact caller representation, including empty text; never parsed or numerically coerced.
    Inline { text: String },
    /// A value too large or unsuitable for inline text, retained through its immutable artifact.
    Artifact { artifact_id: String },
    /// Missing data is distinct from a literal value such as "null", "false" or "0".
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub metric_id: String,
    pub subject: AssessmentSubject,
    pub value: MeasurementValue,
    pub explanation: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub replaces: Option<String>,
}
