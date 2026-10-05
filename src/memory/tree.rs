// @feature agent-memory
// @spec docs/features/agent-memory.md
use super::{
    MemoryContextLimits, MemoryNode, MemoryOmission, MemoryOmissionReason, MemoryScope,
    MemorySummary,
};
use std::collections::BTreeMap;
use std::ops::Range;

pub(crate) struct MemoryFrontier {
    pub(crate) nodes: Vec<MemoryNode>,
    pub(crate) omitted: Vec<MemoryOmission>,
}

pub(crate) fn build_frontier_state(
    scope: &MemoryScope,
    mut raw_nodes: Vec<MemoryNode>,
    summaries: Vec<MemorySummary>,
    limits: MemoryContextLimits,
) -> MemoryFrontier {
    raw_nodes.sort_by_key(MemoryNode::start_ordinal);
    let summaries = summaries
        .into_iter()
        .map(|summary| {
            (
                (summary.start_ordinal, summary.end_ordinal),
                MemoryNode::Summary(summary),
            )
        })
        .collect::<BTreeMap<_, _>>();
    match continue_frontier(scope, raw_nodes, Vec::new(), limits, |start, end| {
        Ok::<_, std::convert::Infallible>(summaries.get(&(start, end)).cloned())
    }) {
        Ok(state) => state,
        Err(error) => match error {},
    }
}

pub(crate) fn continue_frontier<E>(
    scope: &MemoryScope,
    mut nodes: Vec<MemoryNode>,
    mut omitted: Vec<MemoryOmission>,
    limits: MemoryContextLimits,
    mut parent_summary: impl FnMut(u64, u64) -> Result<Option<MemoryNode>, E>,
) -> Result<MemoryFrontier, E> {
    while exceeds_limits(&nodes, limits) {
        let Some(pair_index) = oldest_sibling_pair(&nodes) else {
            omit_node(
                scope,
                &mut nodes,
                0,
                MemoryOmissionReason::Budget,
                &mut omitted,
            );
            continue;
        };
        let start = nodes[pair_index].start_ordinal();
        let end = nodes[pair_index + 1].end_ordinal();
        if let Some(parent) = parent_summary(start, end)? {
            nodes.splice(pair_index..=pair_index + 1, [parent]);
        } else {
            omit_range(
                scope,
                &mut nodes,
                pair_index..pair_index + 2,
                MemoryOmissionReason::MissingSummary,
                &mut omitted,
            );
        }
    }

    normalize_omissions(&mut omitted);
    Ok(MemoryFrontier { nodes, omitted })
}

pub(crate) fn finish_frontier(scope: &MemoryScope, mut state: MemoryFrontier) -> MemoryFrontier {
    // Age cleanup is intentionally deferred until delivery. The bounded reducer
    // state remains continuation-safe for tighter budgets and future appends.
    while let Some(index) = state
        .nodes
        .windows(2)
        .position(|pair| pair[0].level() < pair[1].level())
    {
        omit_node(
            scope,
            &mut state.nodes,
            index,
            MemoryOmissionReason::MissingSummary,
            &mut state.omitted,
        );
    }
    normalize_omissions(&mut state.omitted);
    state
}

fn normalize_omissions(omitted: &mut Vec<MemoryOmission>) {
    omitted.sort_by_key(|range| range.start_ordinal);
    let mut merged: Vec<MemoryOmission> = Vec::with_capacity(omitted.len());
    for range in omitted.drain(..) {
        if let Some(previous) = merged.last_mut() {
            if previous.scope == range.scope && previous.end_ordinal >= range.start_ordinal {
                previous.end_ordinal = previous.end_ordinal.max(range.end_ordinal);
                if previous.reason != range.reason {
                    previous.reason = MemoryOmissionReason::Mixed;
                }
                continue;
            }
        }
        merged.push(range);
    }
    *omitted = merged;
}

fn exceeds_limits(nodes: &[MemoryNode], limits: MemoryContextLimits) -> bool {
    nodes.len() > limits.max_items
        || nodes.iter().map(|node| node.content().len()).sum::<usize>() > limits.max_bytes
}

fn oldest_sibling_pair(nodes: &[MemoryNode]) -> Option<usize> {
    nodes.windows(2).position(|pair| {
        let left = &pair[0];
        let right = &pair[1];
        if left.level() != right.level() || left.end_ordinal() != right.start_ordinal() {
            return false;
        }
        let Some(width) = 1_u64.checked_shl(left.level()) else {
            return false;
        };
        let Some(parent_width) = width.checked_mul(2) else {
            return false;
        };
        left.end_ordinal() - left.start_ordinal() == width
            && right.end_ordinal() - right.start_ordinal() == width
            && left.start_ordinal() % parent_width == 0
    })
}

fn omit_node(
    scope: &MemoryScope,
    nodes: &mut Vec<MemoryNode>,
    index: usize,
    reason: MemoryOmissionReason,
    omitted: &mut Vec<MemoryOmission>,
) {
    omit_range(scope, nodes, index..index + 1, reason, omitted);
}

fn omit_range(
    scope: &MemoryScope,
    nodes: &mut Vec<MemoryNode>,
    indexes: Range<usize>,
    reason: MemoryOmissionReason,
    omitted: &mut Vec<MemoryOmission>,
) {
    let start_ordinal = nodes[indexes.start].start_ordinal();
    let end_ordinal = nodes[indexes.end - 1].end_ordinal();
    nodes.drain(indexes);
    if let Some(previous) = omitted.last_mut() {
        if previous.scope == *scope
            && previous.reason == reason
            && previous.end_ordinal == start_ordinal
        {
            previous.end_ordinal = end_ordinal;
            return;
        }
    }
    omitted.push(MemoryOmission {
        scope: scope.clone(),
        start_ordinal,
        end_ordinal,
        reason,
    });
}
