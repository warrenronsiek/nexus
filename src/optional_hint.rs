// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @boundary optional-host-hints: blank model, child-agent and executor hints mean unavailable; required record and parent identities remain strict.
use serde::{Deserialize, Deserializer};

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.filter(|value| !value.trim().is_empty()))
}
