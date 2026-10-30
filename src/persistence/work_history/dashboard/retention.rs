// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
use crate::work_history::WorkIntervalNode;
use std::collections::{BTreeMap, HashSet};

/// Place recent operations and their recorded ancestors before older display rows.
pub(super) fn recent_with_ancestors(mut nodes: Vec<WorkIntervalNode>) -> Vec<WorkIntervalNode> {
    nodes.sort_by(|left, right| {
        right
            .finished_at
            .max(right.started_at)
            .max(right.queued_at)
            .cmp(&left.finished_at.max(left.started_at).max(left.queued_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    let by_id: BTreeMap<_, _> = nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut emitted = HashSet::new();
    let mut ordered = Vec::with_capacity(nodes.len());
    for candidate in &nodes {
        let mut chain = Vec::new();
        let mut visiting = HashSet::new();
        let mut current = Some(candidate);
        while let Some(node) = current {
            if emitted.contains(node.id.as_str()) || !visiting.insert(node.id.as_str()) {
                break;
            }
            chain.push(node);
            current = node
                .parent_activity_id
                .as_deref()
                .and_then(|id| by_id.get(id).copied());
        }
        for node in chain.into_iter().rev() {
            emitted.insert(node.id.as_str());
            ordered.push(node.clone());
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_history::WorkAccountingRole;

    fn cyclic_node(id: &str, parent: &str) -> WorkIntervalNode {
        WorkIntervalNode {
            id: id.into(),
            title: id.into(),
            kind: "tool".into(),
            parent_activity_id: Some(parent.into()),
            queued_at: None,
            queue_elapsed: None,
            started_at: None,
            finished_at: None,
            state: "declared".into(),
            outcome: None,
            elapsed: None,
            executor: None,
            accounting_role: WorkAccountingRole::Unknown,
            conflicts: vec!["parent_cycle".into()],
            gaps: vec![],
        }
    }

    #[test]
    fn cyclic_parent_evidence_remains_once_with_its_conflict() {
        let nodes = recent_with_ancestors(vec![cyclic_node("a", "b"), cyclic_node("b", "a")]);
        let mut ids: Vec<_> = nodes.iter().map(|node| node.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["a", "b"]);
        assert!(nodes.iter().all(|node| node.conflicts == ["parent_cycle"]));
        assert_eq!(
            nodes
                .iter()
                .find(|node| node.id == "a")
                .unwrap()
                .parent_activity_id
                .as_deref(),
            Some("b")
        );
    }
}
