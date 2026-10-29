// @feature persistence
// @spec docs/features/persistence.md
use anyhow::Result;
use serde::Serialize;

pub(super) struct GroupBounds {
    pub rows: usize,
    pub bytes: usize,
    pub items: usize,
}

/// Ordered bounded reads may reuse complete groups; an uncertain tail must use a normal read.
pub(super) fn complete_groups<K: Eq + Serialize, V: Serialize>(
    rows: Vec<(K, V)>,
    requested: impl IntoIterator<Item = K>,
    bounds: GroupBounds,
) -> Result<Vec<(K, Vec<V>)>> {
    let cut = rows.len() == bounds.rows;
    let mut groups: Vec<(K, Vec<V>)> = vec![];
    for (key, value) in rows {
        match groups.last_mut() {
            Some((last, values)) if *last == key => values.push(value),
            _ => groups.push((key, vec![value])),
        }
    }
    if cut {
        groups.pop();
    } else {
        for key in requested {
            if !groups.iter().any(|(existing, _)| *existing == key) {
                groups.push((key, vec![]));
            }
        }
    }
    bound_groups(groups, bounds)
}

fn bound_groups<K: Serialize, V: Serialize>(
    groups: Vec<(K, Vec<V>)>,
    bounds: GroupBounds,
) -> Result<Vec<(K, Vec<V>)>> {
    let mut retained = vec![];
    let mut bytes = 0;
    for (key, mut values) in groups {
        values.truncate(bounds.items);
        bytes += serde_json::to_vec(&(&key, &values))?.len();
        if bytes > bounds.bytes {
            break;
        }
        retained.push((key, values));
    }
    Ok(retained)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds(rows: usize) -> GroupBounds {
        GroupBounds {
            rows,
            bytes: 1024,
            items: 129,
        }
    }
    #[test]
    fn complete_reads_cache_missing_keys_but_cut_reads_exclude_the_uncertain_tail() {
        let rows = vec![("a", 1), ("a", 2), ("b", 3)];
        assert_eq!(
            complete_groups(rows.clone(), ["a", "b", "c"], bounds(4)).unwrap(),
            vec![("a", vec![1, 2]), ("b", vec![3]), ("c", vec![])]
        );
        assert_eq!(
            complete_groups(rows, ["a", "b", "c"], bounds(3)).unwrap(),
            vec![("a", vec![1, 2])]
        );
    }
    #[test]
    fn byte_capacity_stops_at_a_group_boundary_and_item_capacity_keeps_the_sentinel() {
        let mut limit = bounds(4);
        limit.bytes = serde_json::to_vec(&("a", vec![1])).unwrap().len();
        assert_eq!(
            complete_groups(vec![("a", 1), ("b", 2)], ["a", "b"], limit).unwrap(),
            vec![("a", vec![1])]
        );
        let rows: Vec<_> = (0..130).map(|value| ("a", value)).collect();
        let result = complete_groups(rows, ["a"], bounds(1025)).unwrap();
        assert_eq!(result[0].1.len(), 129);
        assert_eq!(result[0].1.last(), Some(&128));
    }
}
