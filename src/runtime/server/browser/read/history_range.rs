// @feature observability-ui
// @spec docs/features/observability-ui.md
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

#[derive(Clone, Copy, Default, Deserialize)]
pub(super) enum HistoryRange {
    #[serde(rename = "1d")]
    Day,
    #[default]
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "14d")]
    Fortnight,
    #[serde(rename = "30d")]
    Month,
}

impl HistoryRange {
    pub(super) fn cutoff(self) -> DateTime<Utc> {
        let days = match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Fortnight => 14,
            Self::Month => 30,
        };
        Utc::now() - Duration::days(days)
    }
}
