// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;
use crate::persistence::transaction::{transaction, TransactionMode};

impl Store {
    pub fn memory_context(
        &mut self,
        scope: &MemoryReadScope,
        limits: MemoryContextLimits,
    ) -> Result<MemoryContextStatus> {
        let limits = hard_bounded_limits(limits);
        self.repair_context_snapshots(scope)?;
        self.prepare_memory_context(scope, limits)?
            .map(|prepared| prepared.status)
            .context("memory frontier changed while it was being rebuilt")
    }

    pub fn repair_memory_frontier(&mut self, scope: &MemoryScope) -> Result<bool> {
        let Some(space) = find_space(&mut self.connection, scope)? else {
            return Ok(false);
        };
        if snapshot::space_snapshot_is_current(&mut self.connection, &space)? {
            return Ok(false);
        }
        snapshot::rebuild_frontier_snapshot(&mut self.connection, scope)
    }

    pub fn repair_stale_memory_frontiers(&mut self, limit: usize) -> Result<usize> {
        let scopes = snapshot::stale_snapshot_scopes(&mut self.connection, limit)?;
        let mut repaired = 0;
        for scope in scopes {
            repaired += usize::from(snapshot::rebuild_frontier_snapshot(
                &mut self.connection,
                &scope,
            )?);
        }
        Ok(repaired)
    }

    pub fn activate_memory_context(
        &mut self,
        scope: &MemoryReadScope,
        agent: &str,
        session_id: &str,
        limits: MemoryContextLimits,
    ) -> Result<Option<MemoryContextStatus>> {
        if agent.trim().is_empty() || session_id.trim().is_empty() {
            bail!("memory activation requires non-empty agent and session ids");
        }
        if activation_exists(&mut self.connection, agent, session_id)? {
            return Ok(None);
        }
        let limits = hard_bounded_limits(limits);
        let Some(prepared) = self.prepare_memory_context(scope, limits)? else {
            return Ok(None);
        };
        let inserted =
            insert_activation(&mut self.connection, scope, agent, session_id, &prepared)?;
        Ok((inserted == 1).then_some(prepared.status))
    }

    fn prepare_memory_context(
        &mut self,
        scope: &MemoryReadScope,
        limits: MemoryContextLimits,
    ) -> Result<Option<PreparedMemoryContext>> {
        match scope {
            MemoryReadScope::Global => {
                self.single_scope_context(scope.clone(), &MemoryScope::Global, limits)
            }
            MemoryReadScope::Project { project_id } => self.single_scope_context(
                scope.clone(),
                &MemoryScope::project(project_id.clone()),
                limits,
            ),
            MemoryReadScope::Layered { project_id } => {
                self.layered_memory_context(scope.clone(), project_id, limits)
            }
        }
    }

    fn single_scope_context(
        &mut self,
        read_scope: MemoryReadScope,
        scope: &MemoryScope,
        limits: MemoryContextLimits,
    ) -> Result<Option<PreparedMemoryContext>> {
        let Some(set) = snapshot::load_current_snapshot(&mut self.connection, scope)? else {
            return Ok(None);
        };
        let version = set.version.clone();
        let status = context_from_set(&mut self.connection, read_scope, scope, set, limits)?;
        Ok(Some(PreparedMemoryContext {
            status,
            versions: vec![version],
        }))
    }

    fn layered_memory_context(
        &mut self,
        read_scope: MemoryReadScope,
        project_id: &str,
        limits: MemoryContextLimits,
    ) -> Result<Option<PreparedMemoryContext>> {
        let global_scope = MemoryScope::Global;
        let project_scope = MemoryScope::project(project_id.to_owned());
        let Some(global) = snapshot::load_current_snapshot(&mut self.connection, &global_scope)?
        else {
            return Ok(None);
        };
        let Some(project) = snapshot::load_current_snapshot(&mut self.connection, &project_scope)?
        else {
            return Ok(None);
        };
        let versions = vec![global.version.clone(), project.version.clone()];
        let (global_context, project_context) =
            allocate_layered_contexts(&mut self.connection, project_id, global, project, limits)?;
        Ok(Some(PreparedMemoryContext {
            status: merge_layered_context(read_scope, global_context, project_context),
            versions,
        }))
    }

    fn repair_context_snapshots(&mut self, scope: &MemoryReadScope) -> Result<()> {
        for concrete_scope in read_scopes(scope) {
            let current = snapshot::load_current_snapshot(&mut self.connection, &concrete_scope)?;
            if current.is_none() {
                snapshot::rebuild_frontier_snapshot(&mut self.connection, &concrete_scope)?;
            }
        }
        Ok(())
    }
}

struct PreparedMemoryContext {
    status: MemoryContextStatus,
    versions: Vec<snapshot::SnapshotVersion>,
}

fn allocate_layered_contexts(
    connection: &mut SqliteConnection,
    project_id: &str,
    global: snapshot::LoadedSnapshot,
    project: snapshot::LoadedSnapshot,
    limits: MemoryContextLimits,
) -> Result<(MemoryContextStatus, MemoryContextStatus)> {
    let global_scope = MemoryScope::Global;
    let project_scope = MemoryScope::project(project_id.to_owned());
    let (global_limits, project_limits) = base_layered_limits(limits);
    let global_context = context_from_set(
        connection,
        MemoryReadScope::Global,
        &global_scope,
        global.clone(),
        global_limits,
    )?;
    let project_context = context_from_set(
        connection,
        MemoryReadScope::project(project_id.to_owned()),
        &project_scope,
        project.clone(),
        project_limits,
    )?;
    let expanded_global_limits =
        receive_unused_capacity(global_limits, project_limits, &project_context);
    let expanded_project_limits =
        receive_unused_capacity(project_limits, global_limits, &global_context);
    let candidates = [
        LayeredAllocation::new(global_context.clone(), project_context.clone(), 0),
        LayeredAllocation::new(
            context_from_set(
                connection,
                MemoryReadScope::Global,
                &global_scope,
                global,
                expanded_global_limits,
            )?,
            project_context.clone(),
            1,
        ),
        LayeredAllocation::new(
            global_context,
            context_from_set(
                connection,
                MemoryReadScope::project(project_id.to_owned()),
                &project_scope,
                project,
                expanded_project_limits,
            )?,
            2,
        ),
    ];
    Ok(candidates
        .into_iter()
        .filter(|candidate| candidate.within(limits))
        .max_by_key(LayeredAllocation::score)
        .expect("the base layered allocation is within the total limits")
        .into_contexts())
}

struct LayeredAllocation {
    global: MemoryContextStatus,
    project: MemoryContextStatus,
    project_preference: u8,
}

impl LayeredAllocation {
    fn new(
        global: MemoryContextStatus,
        project: MemoryContextStatus,
        project_preference: u8,
    ) -> Self {
        Self {
            global,
            project,
            project_preference,
        }
    }

    fn within(&self, limits: MemoryContextLimits) -> bool {
        self.global.item_count + self.project.item_count <= limits.max_items
            && self.global.byte_count + self.project.byte_count <= limits.max_bytes
    }

    fn score(&self) -> (u64, usize, u8) {
        let coverage = self
            .global
            .nodes
            .iter()
            .chain(&self.project.nodes)
            .map(|node| node.end_ordinal() - node.start_ordinal())
            .sum();
        (
            coverage,
            self.global.item_count + self.project.item_count,
            self.project_preference,
        )
    }

    fn into_contexts(self) -> (MemoryContextStatus, MemoryContextStatus) {
        (self.global, self.project)
    }
}

fn receive_unused_capacity(
    recipient: MemoryContextLimits,
    donor: MemoryContextLimits,
    donor_context: &MemoryContextStatus,
) -> MemoryContextLimits {
    MemoryContextLimits {
        max_items: recipient.max_items + donor.max_items.saturating_sub(donor_context.item_count),
        max_bytes: recipient.max_bytes + donor.max_bytes.saturating_sub(donor_context.byte_count),
    }
}

fn merge_layered_context(
    read_scope: MemoryReadScope,
    global_context: MemoryContextStatus,
    project_context: MemoryContextStatus,
) -> MemoryContextStatus {
    let mut nodes = global_context.nodes;
    nodes.extend(project_context.nodes);
    let mut omitted = global_context.omitted;
    omitted.extend(project_context.omitted);
    MemoryContextStatus {
        scope: read_scope,
        item_count: nodes.len(),
        byte_count: node_bytes(&nodes),
        nodes,
        omitted,
    }
}

fn activation_exists(
    connection: &mut SqliteConnection,
    agent: &str,
    session_id: &str,
) -> Result<bool> {
    Ok(memory_activations::table
        .filter(memory_activations::agent.eq(agent))
        .filter(memory_activations::session_id.eq(session_id))
        .select(memory_activations::id)
        .first::<String>(connection)
        .optional()?
        .is_some())
}

fn insert_activation(
    connection: &mut SqliteConnection,
    scope: &MemoryReadScope,
    agent: &str,
    session_id: &str,
    prepared: &PreparedMemoryContext,
) -> Result<usize> {
    let activation_id = Uuid::new_v4().to_string();
    let (scope_text, project_id) = read_scope_parts(scope);
    let activated_at = Utc::now().to_rfc3339();
    transaction(connection, TransactionMode::Immediate, |connection| {
        if !snapshot::versions_are_current(connection, &prepared.versions)? {
            return Ok(0);
        }
        diesel::insert_into(memory_activations::table)
            .values(NewMemoryActivation {
                id: &activation_id,
                agent,
                session_id,
                read_scope: scope_text,
                project_id,
                delivered_items: i32::try_from(prepared.status.item_count)?,
                omitted_ranges: i32::try_from(prepared.status.omitted.len())?,
                activated_at: &activated_at,
            })
            .on_conflict((memory_activations::agent, memory_activations::session_id))
            .do_nothing()
            .execute(connection)
            .map_err(Into::into)
    })
}

fn context_from_set(
    connection: &mut SqliteConnection,
    read_scope: MemoryReadScope,
    scope: &MemoryScope,
    set: snapshot::LoadedSnapshot,
    limits: MemoryContextLimits,
) -> Result<MemoryContextStatus> {
    let (frontier, _) = snapshot::continue_snapshot(connection, scope, set, limits)?;
    Ok(MemoryContextStatus {
        scope: read_scope,
        item_count: frontier.nodes.len(),
        byte_count: node_bytes(&frontier.nodes),
        nodes: frontier.nodes,
        omitted: frontier.omitted,
    })
}

fn base_layered_limits(total: MemoryContextLimits) -> (MemoryContextLimits, MemoryContextLimits) {
    let global_items = total.max_items / 3;
    let global_bytes = total.max_bytes / 3;
    (
        MemoryContextLimits {
            max_items: global_items,
            max_bytes: global_bytes,
        },
        MemoryContextLimits {
            max_items: total.max_items - global_items,
            max_bytes: total.max_bytes - global_bytes,
        },
    )
}

fn hard_bounded_limits(limits: MemoryContextLimits) -> MemoryContextLimits {
    MemoryContextLimits {
        max_items: limits
            .max_items
            .min(crate::memory::DEFAULT_MEMORY_MAX_ITEMS),
        max_bytes: limits
            .max_bytes
            .min(crate::memory::DEFAULT_MEMORY_MAX_BYTES),
    }
}

fn read_scope_parts(scope: &MemoryReadScope) -> (&'static str, Option<&str>) {
    match scope {
        MemoryReadScope::Global => ("global", None),
        MemoryReadScope::Project { project_id } => ("project", Some(project_id)),
        MemoryReadScope::Layered { project_id } => ("layered", Some(project_id)),
    }
}
