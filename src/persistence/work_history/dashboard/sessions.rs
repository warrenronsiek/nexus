// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// Selectable dashboard roots and their human-request activity.
use crate::work_history::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Visible roots in scan order plus the qualified request time observed for each.
#[derive(Default)]
pub(super) struct SessionRows {
    rows: Vec<(WorkSummary, String)>,
    ids: HashSet<String>,
    requests: HashMap<String, RequestTiming>,
}

#[derive(Default)]
struct RequestTiming {
    count: usize,
    timed: usize,
    seconds: f64,
}

impl SessionRows {
    pub(super) fn push(&mut self, summary: WorkSummary, date: String) {
        self.ids.insert(summary.subject.id().to_owned());
        self.rows.push((summary, date));
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    pub(super) fn root_ids(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(summary, _)| summary.subject.id().to_owned())
            .collect()
    }

    /// Requests count once per visible root they currently belong to.
    pub(super) fn observe_request(&mut self, request: &WorkSummary) {
        let seconds = known_seconds(request);
        for root in request
            .work_session_ids
            .iter()
            .filter(|id| self.ids.contains(*id))
        {
            let timing = self.requests.entry(root.clone()).or_default();
            timing.count += 1;
            if let Some(seconds) = seconds {
                timing.timed += 1;
                timing.seconds += seconds;
            }
        }
    }

    /// Rows plus active statistics overall and by session UTC date.
    pub(super) fn finish(
        mut self,
    ) -> (
        Vec<DashboardSession>,
        DurationStatistics,
        BTreeMap<String, DurationStatistics>,
    ) {
        let mut overall = DurationStatistics::default();
        let mut daily: BTreeMap<String, DurationStatistics> = BTreeMap::new();
        let rows = self
            .rows
            .into_iter()
            .map(|(summary, date)| {
                let timing = self
                    .requests
                    .remove(summary.subject.id())
                    .unwrap_or_default();
                let active_seconds =
                    (timing.count > 0 && timing.timed == timing.count).then_some(timing.seconds);
                overall.observe_seconds(active_seconds);
                daily
                    .entry(date)
                    .or_default()
                    .observe_seconds(active_seconds);
                DashboardSession {
                    summary,
                    request_count: timing.count,
                    timed_request_count: timing.timed,
                    active_seconds,
                }
            })
            .collect();
        (rows, overall, daily)
    }
}

/// A qualified non-negative duration, or unknown when absent or conflicted.
pub(super) fn known_seconds(summary: &WorkSummary) -> Option<f64> {
    summary
        .elapsed
        .as_ref()
        .filter(|_| summary.conflicts.is_empty())
        .and_then(|elapsed| elapsed.nanoseconds.parse::<f64>().ok())
        .map(|ns| ns / 1_000_000_000.0)
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

impl DurationStatistics {
    pub(super) fn observe(&mut self, summary: &WorkSummary) {
        self.observe_seconds(known_seconds(summary));
    }

    pub(super) fn observe_seconds(&mut self, seconds: Option<f64>) {
        self.count += 1;
        if let Some(seconds) = seconds {
            self.known_count += 1;
            self.total_seconds += seconds;
            self.average_seconds = Some(self.total_seconds / self.known_count as f64);
        }
    }
}
