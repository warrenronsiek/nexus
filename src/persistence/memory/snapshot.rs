// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(super) enum SnapshotVersion {
    Absent(MemoryScope),
    Present { space_id: String, revision: i64 },
}

#[derive(Clone)]
pub(super) struct LoadedSnapshot {
    pub(super) nodes: Vec<MemoryNode>,
    pub(super) omitted: Vec<crate::memory::MemoryOmission>,
    pub(super) version: SnapshotVersion,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SnapshotNodeRef {
    Raw { id: String },
    Summary { id: String },
}

struct EncodedSnapshot {
    node_refs_json: String,
    omissions_json: String,
    item_count: i32,
    omission_count: i32,
    byte_count: i32,
}

pub(super) fn load_current_snapshot(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
) -> Result<Option<LoadedSnapshot>> {
    let Some(space) = find_space(connection, scope)? else {
        return Ok(Some(LoadedSnapshot {
            nodes: Vec::new(),
            omitted: Vec::new(),
            version: SnapshotVersion::Absent(scope.clone()),
        }));
    };
    load_snapshot_for_space(connection, &space, scope)
}

pub(super) fn space_snapshot_is_current(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
) -> Result<bool> {
    Ok(memory_frontier_snapshots::table
        .filter(memory_frontier_snapshots::space_id.eq(&space.id))
        .filter(memory_frontier_snapshots::revision.eq(space.revision))
        .select(memory_frontier_snapshots::space_id)
        .first::<String>(connection)
        .optional()?
        .is_some())
}

pub(super) fn advance_snapshot_for_append(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    entry: &MemoryEntry,
    now: &str,
) -> Result<bool> {
    let scope = compaction::scope_from_space(space)?;
    let base = snapshot_before_append(connection, space, &scope)?;
    let Some(mut base) = base else {
        return Ok(false);
    };
    advance_loaded_snapshot(connection, space, entry, now, &mut base)
}

fn advance_loaded_snapshot(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    entry: &MemoryEntry,
    now: &str,
    base: &mut LoadedSnapshot,
) -> Result<bool> {
    let scope = compaction::scope_from_space(space)?;
    base.nodes.push(MemoryNode::Raw(entry.clone()));
    let frontier = continue_with_stored_parents(
        connection,
        &scope,
        std::mem::take(&mut base.nodes),
        std::mem::take(&mut base.omitted),
        MemoryContextLimits::default(),
    )?;
    let revision = next_revision(space)?;
    persist_snapshot(connection, &space.id, revision, &frontier, now)?;
    Ok(true)
}

fn snapshot_before_append(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    scope: &MemoryScope,
) -> Result<Option<LoadedSnapshot>> {
    if space.next_ordinal != 0 {
        return load_snapshot_for_space(connection, space, scope);
    }
    Ok(Some(LoadedSnapshot {
        nodes: Vec::new(),
        omitted: Vec::new(),
        version: SnapshotVersion::Present {
            space_id: space.id.clone(),
            revision: space.revision,
        },
    }))
}

fn next_revision(space: &MemorySpaceRow) -> Result<i64> {
    space
        .revision
        .checked_add(1)
        .context("memory space revision overflow")
}

pub(super) fn continue_snapshot(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
    snapshot: LoadedSnapshot,
    limits: MemoryContextLimits,
) -> Result<(crate::memory::tree::MemoryFrontier, SnapshotVersion)> {
    if matches!(&snapshot.version, SnapshotVersion::Absent(_)) {
        return Ok((
            crate::memory::tree::MemoryFrontier {
                nodes: snapshot.nodes,
                omitted: snapshot.omitted,
            },
            snapshot.version,
        ));
    }
    let state =
        continue_with_stored_parents(connection, scope, snapshot.nodes, snapshot.omitted, limits)?;
    Ok((
        crate::memory::tree::finish_frontier(scope, state),
        snapshot.version,
    ))
}

pub(super) fn rebuild_frontier_snapshot(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
) -> Result<bool> {
    let Some((space, frontier)) = build_snapshot(connection, scope)? else {
        return Ok(true);
    };
    publish_snapshot(connection, &space, &frontier)
}

fn build_snapshot(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
) -> Result<Option<(MemorySpaceRow, crate::memory::tree::MemoryFrontier)>> {
    let Some(space) = find_space(connection, scope)? else {
        return Ok(None);
    };
    let raw = load_raw_nodes(connection, &space.id, scope)?;
    let summaries = load_summaries(connection, &space.id, scope)?;
    let frontier = crate::memory::tree::build_frontier_state(
        scope,
        raw,
        summaries,
        MemoryContextLimits::default(),
    );
    Ok(Some((space, frontier)))
}

fn publish_snapshot(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    frontier: &crate::memory::tree::MemoryFrontier,
) -> Result<bool> {
    let built_at = Utc::now().to_rfc3339();
    connection.immediate_transaction::<_, anyhow::Error, _>(|connection| {
        let revision = memory_spaces::table
            .find(&space.id)
            .select(memory_spaces::revision)
            .first::<i64>(connection)?;
        if revision != space.revision {
            return Ok(false);
        }
        persist_snapshot(connection, &space.id, revision, frontier, &built_at)?;
        Ok(true)
    })
}

pub(super) fn stale_snapshot_scopes(
    connection: &mut SqliteConnection,
    limit: usize,
) -> Result<Vec<MemoryScope>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let order = (memory_spaces::updated_at.asc(), memory_spaces::id.asc());
    let mut spaces = memory_spaces::table
        .left_join(
            memory_frontier_snapshots::table
                .on(memory_frontier_snapshots::space_id.eq(memory_spaces::id)),
        )
        .filter(memory_frontier_snapshots::space_id.is_null())
        .order(order)
        .limit(i64::try_from(limit)?)
        .select(MemorySpaceRow::as_select())
        .load::<MemorySpaceRow>(connection)?;
    let remaining = limit.saturating_sub(spaces.len());
    if remaining > 0 {
        spaces.extend(
            memory_spaces::table
                .inner_join(
                    memory_frontier_snapshots::table
                        .on(memory_frontier_snapshots::space_id.eq(memory_spaces::id)),
                )
                .filter(memory_frontier_snapshots::revision.ne(memory_spaces::revision))
                .order(order)
                .limit(i64::try_from(remaining)?)
                .select(MemorySpaceRow::as_select())
                .load::<MemorySpaceRow>(connection)?,
        );
    }
    spaces.iter().map(compaction::scope_from_space).collect()
}

pub(super) fn versions_are_current(
    connection: &mut SqliteConnection,
    versions: &[SnapshotVersion],
) -> Result<bool> {
    for version in versions {
        if !version_is_current(connection, version)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn version_is_current(
    connection: &mut SqliteConnection,
    version: &SnapshotVersion,
) -> Result<bool> {
    match version {
        SnapshotVersion::Absent(scope) => Ok(find_space(connection, scope)?.is_none()),
        SnapshotVersion::Present { space_id, revision } => {
            let current = memory_spaces::table
                .find(space_id)
                .select(memory_spaces::revision)
                .first::<i64>(connection)
                .optional()?;
            let snapshot = memory_frontier_snapshots::table
                .find(space_id)
                .select(memory_frontier_snapshots::revision)
                .first::<i64>(connection)
                .optional()?;
            Ok(current == Some(*revision) && snapshot == Some(*revision))
        }
    }
}

fn load_snapshot_for_space(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    scope: &MemoryScope,
) -> Result<Option<LoadedSnapshot>> {
    let row = memory_frontier_snapshots::table
        .filter(memory_frontier_snapshots::space_id.eq(&space.id))
        .filter(memory_frontier_snapshots::revision.eq(space.revision))
        .select(MemoryFrontierSnapshotRow::as_select())
        .first::<MemoryFrontierSnapshotRow>(connection)
        .optional()?;
    row.map(|row| decode_snapshot(connection, scope, row))
        .transpose()
}

fn decode_snapshot(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
    row: MemoryFrontierSnapshotRow,
) -> Result<LoadedSnapshot> {
    DateTime::parse_from_rfc3339(&row.built_at)
        .context("memory frontier snapshot has an invalid build time")?;
    let (nodes, omitted) = decode_snapshot_contents(connection, scope, &row)?;
    Ok(LoadedSnapshot {
        nodes,
        omitted,
        version: SnapshotVersion::Present {
            space_id: row.space_id,
            revision: row.revision,
        },
    })
}

fn decode_snapshot_contents(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
    row: &MemoryFrontierSnapshotRow,
) -> Result<(Vec<MemoryNode>, Vec<crate::memory::MemoryOmission>)> {
    let references = decode_references(row)?;
    let nodes = load_referenced_nodes(connection, scope, &references)?;
    validate_snapshot_bytes(&nodes, row.byte_count)?;
    let omitted = decode_omissions(row)?;
    validate_snapshot_ranges(scope, &nodes, &omitted)?;
    Ok((nodes, omitted))
}

fn decode_references(row: &MemoryFrontierSnapshotRow) -> Result<Vec<SnapshotNodeRef>> {
    let references = serde_json::from_str::<Vec<SnapshotNodeRef>>(&row.node_refs_json)
        .context("decode memory frontier node references")?;
    if references.len() > crate::memory::DEFAULT_MEMORY_MAX_ITEMS
        || i32::try_from(references.len())? != row.item_count
    {
        bail!("memory frontier snapshot has an invalid item count");
    }
    Ok(references)
}

fn validate_snapshot_bytes(nodes: &[MemoryNode], expected: i32) -> Result<()> {
    if i32::try_from(node_bytes(nodes))? != expected {
        bail!("memory frontier snapshot has an invalid byte count");
    }
    Ok(())
}

fn decode_omissions(row: &MemoryFrontierSnapshotRow) -> Result<Vec<crate::memory::MemoryOmission>> {
    let omitted = serde_json::from_str::<Vec<crate::memory::MemoryOmission>>(&row.omissions_json)
        .context("decode memory frontier omissions")?;
    if omitted.len() > crate::memory::MAX_MEMORY_FRONTIER_OMISSIONS
        || i32::try_from(omitted.len())? != row.omission_count
    {
        bail!("memory frontier snapshot has an invalid omission count");
    }
    Ok(omitted)
}

fn load_referenced_nodes(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
    references: &[SnapshotNodeRef],
) -> Result<Vec<MemoryNode>> {
    let raw_ids = references
        .iter()
        .filter_map(|reference| match reference {
            SnapshotNodeRef::Raw { id } => Some(id),
            SnapshotNodeRef::Summary { .. } => None,
        })
        .collect::<Vec<_>>();
    let summary_ids = references
        .iter()
        .filter_map(|reference| match reference {
            SnapshotNodeRef::Raw { .. } => None,
            SnapshotNodeRef::Summary { id } => Some(id),
        })
        .collect::<Vec<_>>();
    let raw = index_nodes(
        memory_entries::table
            .filter(memory_entries::id.eq_any(raw_ids))
            .select(MemoryEntryRow::as_select())
            .load::<MemoryEntryRow>(connection)?
            .into_iter()
            .map(|row| entry_from_row(row, scope.clone()).map(MemoryNode::Raw)),
    )?;
    let summaries = index_nodes(
        memory_summaries::table
            .filter(memory_summaries::id.eq_any(summary_ids))
            .select(MemorySummaryRow::as_select())
            .load::<MemorySummaryRow>(connection)?
            .into_iter()
            .map(|row| compaction::summary_from_row(row, scope.clone()).map(MemoryNode::Summary)),
    )?;
    references
        .iter()
        .map(|reference| referenced_node(reference, &raw, &summaries))
        .collect()
}

fn index_nodes(
    nodes: impl IntoIterator<Item = Result<MemoryNode>>,
) -> Result<HashMap<String, MemoryNode>> {
    nodes
        .into_iter()
        .map(|node| {
            let node = node?;
            Ok((node.id().to_owned(), node))
        })
        .collect()
}

fn referenced_node(
    reference: &SnapshotNodeRef,
    raw: &HashMap<String, MemoryNode>,
    summaries: &HashMap<String, MemoryNode>,
) -> Result<MemoryNode> {
    match reference {
        SnapshotNodeRef::Raw { id } => raw.get(id),
        SnapshotNodeRef::Summary { id } => summaries.get(id),
    }
    .cloned()
    .context("memory frontier references an unavailable node")
}

fn validate_snapshot_ranges(
    scope: &MemoryScope,
    nodes: &[MemoryNode],
    omitted: &[crate::memory::MemoryOmission],
) -> Result<()> {
    if nodes.iter().any(|node| node.scope() != scope)
        || nodes
            .windows(2)
            .any(|pair| pair[0].end_ordinal() > pair[1].start_ordinal())
        || omitted
            .iter()
            .any(|range| range.scope != *scope || range.start_ordinal >= range.end_ordinal)
        || omitted
            .windows(2)
            .any(|pair| pair[0].end_ordinal >= pair[1].start_ordinal)
        || omitted.iter().any(|range| {
            nodes.iter().any(|node| {
                range.start_ordinal < node.end_ordinal() && node.start_ordinal() < range.end_ordinal
            })
        })
    {
        bail!("memory frontier snapshot contains invalid ranges");
    }
    Ok(())
}

fn continue_with_stored_parents(
    connection: &mut SqliteConnection,
    scope: &MemoryScope,
    nodes: Vec<MemoryNode>,
    omitted: Vec<crate::memory::MemoryOmission>,
    limits: MemoryContextLimits,
) -> Result<crate::memory::tree::MemoryFrontier> {
    let space_id = find_space(connection, scope)?
        .map(|space| space.id)
        .context("memory frontier space disappeared")?;
    crate::memory::tree::continue_frontier(scope, nodes, omitted, limits, |start, end| {
        compaction::find_summary_row(connection, &space_id, start, end)?
            .map(|row| compaction::summary_from_row(row, scope.clone()).map(MemoryNode::Summary))
            .transpose()
    })
}

fn persist_snapshot(
    connection: &mut SqliteConnection,
    space_id: &str,
    revision: i64,
    frontier: &crate::memory::tree::MemoryFrontier,
    built_at: &str,
) -> Result<()> {
    let encoded = encode_snapshot(frontier)?;
    let row = NewMemoryFrontierSnapshot {
        space_id,
        revision,
        node_refs_json: &encoded.node_refs_json,
        omissions_json: &encoded.omissions_json,
        item_count: encoded.item_count,
        omission_count: encoded.omission_count,
        byte_count: encoded.byte_count,
        built_at,
    };
    upsert_snapshot(connection, &row)
}

fn encode_snapshot(frontier: &crate::memory::tree::MemoryFrontier) -> Result<EncodedSnapshot> {
    let references = snapshot_references(&frontier.nodes);
    if references.len() > crate::memory::DEFAULT_MEMORY_MAX_ITEMS {
        bail!("memory frontier snapshot exceeds the hard item limit");
    }
    let byte_count = node_bytes(&frontier.nodes);
    if byte_count > crate::memory::DEFAULT_MEMORY_MAX_BYTES {
        bail!("memory frontier snapshot exceeds the hard byte limit");
    }
    if frontier.omitted.len() > crate::memory::MAX_MEMORY_FRONTIER_OMISSIONS {
        bail!("memory frontier snapshot exceeds the omission limit");
    }
    Ok(EncodedSnapshot {
        node_refs_json: serde_json::to_string(&references)?,
        omissions_json: serde_json::to_string(&frontier.omitted)?,
        item_count: i32::try_from(references.len())?,
        omission_count: i32::try_from(frontier.omitted.len())?,
        byte_count: i32::try_from(byte_count)?,
    })
}

fn snapshot_references(nodes: &[MemoryNode]) -> Vec<SnapshotNodeRef> {
    nodes
        .iter()
        .map(|node| match node {
            MemoryNode::Raw(entry) => SnapshotNodeRef::Raw {
                id: entry.id.clone(),
            },
            MemoryNode::Summary(summary) => SnapshotNodeRef::Summary {
                id: summary.id.clone(),
            },
        })
        .collect()
}

fn upsert_snapshot(
    connection: &mut SqliteConnection,
    row: &NewMemoryFrontierSnapshot<'_>,
) -> Result<()> {
    diesel::insert_into(memory_frontier_snapshots::table)
        .values(row)
        .on_conflict(memory_frontier_snapshots::space_id)
        .do_update()
        .set((
            memory_frontier_snapshots::revision.eq(row.revision),
            memory_frontier_snapshots::node_refs_json.eq(row.node_refs_json),
            memory_frontier_snapshots::omissions_json.eq(row.omissions_json),
            memory_frontier_snapshots::item_count.eq(row.item_count),
            memory_frontier_snapshots::omission_count.eq(row.omission_count),
            memory_frontier_snapshots::byte_count.eq(row.byte_count),
            memory_frontier_snapshots::built_at.eq(row.built_at),
        ))
        .execute(connection)?;
    Ok(())
}
