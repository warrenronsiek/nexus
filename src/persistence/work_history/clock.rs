// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::work_history::{Elapsed, WorkObservation};

pub(super) enum IntervalGap {
    ClockDomainMismatch,
    SourceClockAnomaly,
    WallClockAnomaly,
    WallClockRange,
}
impl IntervalGap {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ClockDomainMismatch => "clock_domain_mismatch",
            Self::SourceClockAnomaly => "source_clock_anomaly",
            Self::WallClockAnomaly => "wall_clock_anomaly",
            Self::WallClockRange => "wall_clock_range",
        }
    }
    pub fn conflict(&self) -> Option<&'static str> {
        match self {
            Self::SourceClockAnomaly => Some("invalid_source_clock_interval"),
            Self::WallClockAnomaly => Some("invalid_wall_clock_interval"),
            _ => None,
        }
    }
}
pub(super) fn elapsed(
    start: &WorkObservation,
    finish: &WorkObservation,
) -> Result<Elapsed, IntervalGap> {
    if let (Some(start), Some(finish)) = (&start.input.source_clock, &finish.input.source_clock) {
        monotonic(start, finish)
    } else {
        wall_elapsed(start, finish)
    }
}
fn monotonic(
    start: &crate::work_history::WorkSourceClock,
    finish: &crate::work_history::WorkSourceClock,
) -> Result<Elapsed, IntervalGap> {
    if start.clock_id != finish.clock_id {
        return Err(IntervalGap::ClockDomainMismatch);
    }
    let start_ns = start
        .nanoseconds
        .parse::<u128>()
        .expect("validated source clock");
    let finish_ns = finish
        .nanoseconds
        .parse::<u128>()
        .expect("validated source clock");
    let nanoseconds = finish_ns
        .checked_sub(start_ns)
        .ok_or(IntervalGap::SourceClockAnomaly)?;
    Ok(Elapsed {
        nanoseconds: nanoseconds.to_string(),
        method: "os_monotonic_callback_interval".into(),
        clock_id: Some(start.clock_id.clone()),
    })
}
fn wall_elapsed(start: &WorkObservation, finish: &WorkObservation) -> Result<Elapsed, IntervalGap> {
    if start.input.producer != finish.input.producer {
        return Err(IntervalGap::ClockDomainMismatch);
    }
    let nanoseconds = finish
        .input
        .occurred_at
        .signed_duration_since(start.input.occurred_at)
        .num_nanoseconds()
        .ok_or(IntervalGap::WallClockRange)?;
    if nanoseconds < 0 {
        return Err(IntervalGap::WallClockAnomaly);
    }
    Ok(Elapsed {
        nanoseconds: nanoseconds.to_string(),
        method: "wall_clock".into(),
        clock_id: Some(start.input.producer.instance_id.clone()),
    })
}
pub(super) fn same_boundary(left: &WorkObservation, right: &WorkObservation) -> bool {
    left.input.occurred_at == right.input.occurred_at
        && left.input.source_clock == right.input.source_clock
}
pub(super) fn competing_boundaries(events: &[&WorkObservation]) -> bool {
    events
        .windows(2)
        .any(|pair| !same_boundary(pair[0], pair[1]))
}
