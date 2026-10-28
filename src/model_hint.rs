// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @boundary optional-model-hint: hosts can substitute an empty string for unavailable model metadata; preserve that absence without choosing a model.
use serde::{Deserialize, Deserializer};

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.filter(|value| !value.trim().is_empty()))
}
