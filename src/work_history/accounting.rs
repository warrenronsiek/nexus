// @feature work-history
// @spec docs/features/work-history.md
use super::{Elapsed, WorkOutcome, WorkSummary};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkAccountingRole {
    Container,
    Execution,
    Wait,
    Unknown,
}
impl WorkAccountingRole {
    fn of(summary: &WorkSummary) -> Self {
        match summary
            .context
            .labels
            .get("nexus.accounting.role")
            .map(String::as_str)
        {
            Some("container") => Self::Container,
            Some("execution") => Self::Execution,
            Some("wait") => Self::Wait,
            _ => Self::Unknown,
        }
    }
    fn attributed(self) -> bool {
        matches!(self, Self::Execution | Self::Wait)
    }
}

/// Compact source evidence for the vertically pannable session waterfall.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkIntervalNode {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub parent_activity_id: Option<String>,
    pub queued_at: Option<DateTime<Utc>>,
    pub queue_elapsed: Option<Elapsed>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub state: String,
    pub outcome: Option<WorkOutcome>,
    pub elapsed: Option<Elapsed>,
    pub executor: Option<String>,
    pub accounting_role: WorkAccountingRole,
    pub conflicts: Vec<String>,
    pub gaps: Vec<String>,
}
impl From<&WorkSummary> for WorkIntervalNode {
    fn from(summary: &WorkSummary) -> Self {
        Self {
            id: summary.subject.id().into(),
            title: summary.title.clone(),
            kind: summary.kind.clone(),
            parent_activity_id: summary.parent_activity_id.clone(),
            queued_at: summary.queued_at,
            queue_elapsed: summary.queue_elapsed.clone(),
            started_at: summary.started_at,
            finished_at: summary.finished_at,
            state: summary.state.clone(),
            outcome: summary.outcome,
            elapsed: summary.elapsed.clone(),
            executor: summary.executor.clone(),
            accounting_role: WorkAccountingRole::of(summary),
            conflicts: summary.conflicts.clone(),
            gaps: summary.gaps.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountingDuration {
    pub nanoseconds: String,
    /// Approximate display conversion; nanoseconds retains the exact integer.
    pub seconds: f64,
}
impl AccountingDuration {
    fn exact(nanoseconds: String) -> Self {
        Self {
            seconds: nanoseconds
                .parse::<f64>()
                .expect("validated exact nanoseconds")
                / 1_000_000_000.0,
            nanoseconds,
        }
    }
    fn sum(nanoseconds: String) -> Self {
        Self::exact(if nanoseconds.is_empty() {
            "0".into()
        } else {
            nanoseconds
        })
    }
    fn nanos(nanoseconds: u128) -> Self {
        Self::exact(nanoseconds.to_string())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountingInterval {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkAccountingCoverage {
    pub activity_count: usize,
    pub interval_count: usize,
    pub queue_count: usize,
    pub known_queue_count: usize,
    pub missing_start_count: usize,
    pub missing_finish_count: usize,
    pub clock_conflict_count: usize,
    pub conflicted_count: usize,
    pub unlinked_count: usize,
    pub unknown_executor_count: usize,
    pub unknown_role_count: usize,
    pub outside_session_count: usize,
    pub source_partial: bool,
    pub display_partial: bool,
    pub capability_inventory: String,
    pub observed_adapters: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkSessionAccounting {
    pub session_wall: Option<AccountingDuration>,
    pub occupied_wall: Option<AccountingDuration>,
    pub inclusive_execution: AccountingDuration,
    pub queue_wait: AccountingDuration,
    pub worker_occupancy: Option<AccountingDuration>,
    pub unobserved: Option<AccountingDuration>,
    pub unobserved_intervals: Vec<AccountingInterval>,
    pub coverage: WorkAccountingCoverage,
    pub definitions: Vec<String>,
}
type Interval = (DateTime<Utc>, DateTime<Utc>);

/// One identity contributes once; containers and descendants can overlap.
pub(crate) fn session_accounting(
    root: &WorkSummary,
    activities: &[WorkSummary],
    partial: bool,
) -> WorkSessionAccounting {
    let mut coverage = coverage(root, activities, partial);
    let bounds = source_interval(root);
    let mut observed = AccountingIntervals::default();
    for activity in activities {
        observed.include(activity, bounds);
    }
    coverage.interval_count = observed.interval_count;
    coverage.outside_session_count = observed.outside_session_count;
    coverage.source_partial |= observed.outside_session_count > 0;
    let occupied = merged(observed.occupied);
    let gaps = bounds
        .map(|bounds| uncovered(bounds, &occupied))
        .unwrap_or_default();
    let worker_occupancy = (!observed.workers.is_empty()).then(|| {
        AccountingDuration::nanos(
            observed
                .workers
                .into_values()
                .map(|intervals| total(&merged(intervals)))
                .sum(),
        )
    });
    WorkSessionAccounting {
        session_wall: bounds.map(|interval| AccountingDuration::nanos(total(&[interval]))),
        occupied_wall: bounds.map(|_| AccountingDuration::nanos(total(&occupied))),
        inclusive_execution: AccountingDuration::sum(observed.inclusive),
        queue_wait: AccountingDuration::sum(observed.queue_wait), worker_occupancy,
        unobserved: bounds.map(|_| AccountingDuration::nanos(total(&gaps))),
        unobserved_intervals: gaps.into_iter().map(|(started_at, finished_at)| AccountingInterval { started_at, finished_at }).collect(),
        coverage,
        definitions: vec![
            "session_wall: source UTC end minus start, qualified by matched same-producer or reported timing; session lifetime is not active work".into(),
            "occupied_wall: union of valid UTC intervals explicitly labeled nexus.accounting.role execution or wait plus qualified queue waits, clipped to session boundaries; other container and unknown intervals are excluded".into(),
            "queue_wait: exact inclusive sum of qualified queued-to-start intervals; separate from execution and worker occupancy".into(),
            "inclusive_execution: exact sum of all non-conflicted reported/qualified activity durations, including nested containers; not effort".into(),
            "worker_occupancy: union of qualified activity UTC start-to-finish intervals per explicitly qualified executor, including containers, waits, and unknown roles; summed across executors, with queued intervals excluded; not CPU time or inferred agent identity".into(),
            "unobserved: session intervals not attributed to explicitly classified execution/wait intervals; not inferred idle, model-response, reasoning, or CPU time".into(),
            "coverage: observed history only; missing source inventory, unsupported callbacks, partial reads, and capture loss prevent a completeness claim".into(),
        ],
    }
}
#[derive(Default)]
struct AccountingIntervals<'a> {
    occupied: Vec<Interval>,
    workers: BTreeMap<&'a str, Vec<Interval>>,
    inclusive: String,
    queue_wait: String,
    interval_count: usize,
    outside_session_count: usize,
}
impl<'a> AccountingIntervals<'a> {
    fn include(&mut self, activity: &'a WorkSummary, bounds: Option<Interval>) {
        if let Some(elapsed) = activity
            .elapsed
            .as_ref()
            .filter(|_| activity.conflicts.is_empty())
        {
            self.inclusive = decimal_sum(&self.inclusive, &elapsed.nanoseconds);
        }
        let execution = source_interval(activity);
        let queue = queue_interval(activity);
        self.outside_session_count += usize::from(bounds.is_some_and(|bounds| {
            execution
                .into_iter()
                .chain(queue)
                .any(|interval| interval.0 < bounds.0 || interval.1 > bounds.1)
        }));
        self.include_queue(activity, queue, bounds);
        let Some(interval) = execution else {
            return;
        };
        self.interval_count += 1;
        if let Some(executor) = activity.executor.as_deref() {
            self.workers.entry(executor).or_default().push(interval);
        }
        if WorkAccountingRole::of(activity).attributed() {
            if let Some(interval) = bounds.and_then(|bounds| clipped(interval, bounds)) {
                self.occupied.push(interval);
            }
        }
    }
    fn include_queue(
        &mut self,
        activity: &WorkSummary,
        interval: Option<Interval>,
        bounds: Option<Interval>,
    ) {
        if let Some(elapsed) = activity
            .queue_elapsed
            .as_ref()
            .filter(|_| activity.conflicts.is_empty())
        {
            self.queue_wait = decimal_sum(&self.queue_wait, &elapsed.nanoseconds);
        }
        if let Some(interval) =
            interval.and_then(|interval| bounds.and_then(|bounds| clipped(interval, bounds)))
        {
            self.occupied.push(interval);
        }
    }
}
fn source_interval(summary: &WorkSummary) -> Option<Interval> {
    if !summary.conflicts.is_empty()
        || summary.elapsed.is_none()
        || summary.gaps.iter().any(|gap| {
            matches!(
                gap.as_str(),
                "clock_domain_mismatch" | "wall_clock_anomaly" | "projection_limit"
            )
        })
    {
        return None;
    }
    let (Some(start), Some(end)) = (summary.started_at, summary.finished_at) else {
        return None;
    };
    (end >= start).then_some((start, end))
}
fn queue_interval(summary: &WorkSummary) -> Option<Interval> {
    if !summary.conflicts.is_empty()
        || summary.queue_elapsed.is_none()
        || summary
            .gaps
            .iter()
            .any(|gap| gap == "queue_wall_clock_anomaly")
    {
        return None;
    }
    let (Some(start), Some(end)) = (summary.queued_at, summary.started_at) else {
        return None;
    };
    (end >= start).then_some((start, end))
}
fn clipped((start, end): Interval, (left, right): Interval) -> Option<Interval> {
    let interval = (start.max(left), end.min(right));
    (interval.0 <= interval.1).then_some(interval)
}
fn merged(mut intervals: Vec<Interval>) -> Vec<Interval> {
    intervals.sort_unstable();
    let mut result: Vec<Interval> = Vec::new();
    for (start, end) in intervals {
        if let Some(previous) = result.last_mut().filter(|previous| start <= previous.1) {
            previous.1 = previous.1.max(end);
        } else {
            result.push((start, end));
        }
    }
    result
}
fn total(intervals: &[Interval]) -> u128 {
    intervals
        .iter()
        .map(|(start, end)| {
            let duration = end.signed_duration_since(*start);
            duration.num_seconds() as u128 * 1_000_000_000 + duration.subsec_nanos() as u128
        })
        .sum()
}
fn uncovered((start, end): Interval, occupied: &[Interval]) -> Vec<Interval> {
    let mut cursor = start;
    let mut gaps = vec![];
    for &(left, right) in occupied {
        if left > cursor {
            gaps.push((cursor, left));
        }
        cursor = cursor.max(right);
    }
    if cursor < end {
        gaps.push((cursor, end));
    }
    gaps
}
fn decimal_sum(left: &str, right: &str) -> String {
    let mut digits = left.bytes().rev();
    let mut other = right.bytes().rev();
    let mut result = Vec::new();
    let mut carry = 0;
    loop {
        let (a, b) = (digits.next(), other.next());
        if a.is_none() && b.is_none() && carry == 0 {
            break;
        }
        let sum = a.map_or(0, |n| n - b'0') + b.map_or(0, |n| n - b'0') + carry;
        result.push(b'0' + sum % 10);
        carry = sum / 10;
    }
    let text = String::from_utf8(result.into_iter().rev().collect()).expect("decimal digits");
    let trimmed = text.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.into()
    }
}
fn coverage(
    root: &WorkSummary,
    activities: &[WorkSummary],
    partial: bool,
) -> WorkAccountingCoverage {
    let ids = activities
        .iter()
        .map(|activity| activity.subject.id())
        .collect::<BTreeSet<_>>();
    let mut result = WorkAccountingCoverage {
        activity_count: activities.len(),
        source_partial: partial || incomplete_source(root),
        capability_inventory: "unavailable".into(),
        ..Default::default()
    };
    let mut adapters = BTreeSet::new();
    if let Some(adapter) = root.context.labels.get("nexus.capture.adapter") {
        adapters.insert(adapter.clone());
    }
    for activity in activities {
        result.observe(activity, &ids);
        if let Some(adapter) = activity.context.labels.get("nexus.capture.adapter") {
            adapters.insert(adapter.clone());
        }
    }
    result.observed_adapters = adapters.into_iter().take(32).collect();
    result
}

impl WorkAccountingCoverage {
    fn observe(&mut self, activity: &WorkSummary, ids: &BTreeSet<&str>) {
        self.queue_count += usize::from(
            activity.queued_at.is_some()
                || activity
                    .conflicts
                    .iter()
                    .any(|conflict| conflict == "conflicting_queues"),
        );
        self.known_queue_count +=
            usize::from(activity.queue_elapsed.is_some() && activity.conflicts.is_empty());
        self.missing_start_count += usize::from(activity.started_at.is_none());
        self.missing_finish_count += usize::from(activity.finished_at.is_none());
        self.clock_conflict_count += usize::from(activity.gaps.iter().any(|gap| {
            matches!(
                gap.as_str(),
                "clock_domain_mismatch"
                    | "wall_clock_anomaly"
                    | "source_clock_anomaly"
                    | "wall_clock_range"
                    | "queue_clock_domain_mismatch"
                    | "queue_wall_clock_anomaly"
                    | "queue_source_clock_anomaly"
                    | "queue_wall_clock_range"
            )
        }));
        self.conflicted_count += usize::from(!activity.conflicts.is_empty());
        self.unlinked_count += usize::from(
            activity
                .parent_activity_id
                .as_deref()
                .is_some_and(|parent| !ids.contains(parent)),
        );
        self.unknown_executor_count += usize::from(activity.executor.is_none());
        self.unknown_role_count +=
            usize::from(WorkAccountingRole::of(activity) == WorkAccountingRole::Unknown);
        self.source_partial |= incomplete_source(activity);
    }
}

fn incomplete_source(summary: &WorkSummary) -> bool {
    summary.gaps.iter().any(|gap| {
        matches!(
            gap.as_str(),
            "projection_limit"
                | "lineage_limit"
                | "membership_limit"
                | "missing_request_identity"
                | "context_bridge_unavailable"
                | "host_resume_epoch_unavailable"
                | "request_context_evicted"
        )
    })
}
