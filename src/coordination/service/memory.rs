// @feature agent-memory
// @feature coordination
// @spec docs/features/agent-memory.md
// @spec docs/features/coordination.md
// @boundary dynamic-json
use super::NexusService;
use crate::agents::model::{self, MemoryConsolidationFailureKind};
use crate::config::ModelProvider;
use crate::coordination::api::{
    MemoryAddCommand, MemoryConsolidationReport, MemoryContextQuery, MemoryContextResponse,
    MemoryEntriesResponse, MemoryInvalidationResponse, MemoryNodesResponse, MemoryReadScopeArg,
    MemoryScopeArg, MemorySearchQuery, MemoryStatusQuery, MemoryStatusResponse, MemorySummaryQuery,
    MemoryWriteResponse, ServiceResponse,
};
use crate::coordination::domain::ToolHookInput;
use crate::coordination::workspace;
use crate::memory::{
    MemoryCompactionAttempt, MemoryCompactionJob, MemoryCompactionOutcome, MemoryContextLimits,
    MemoryProvenance, MemoryReadScope, MemoryScope, MemorySummaryInsert,
};
use crate::persistence::Store;
use anyhow::{bail, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::sync::atomic::Ordering;

struct MemoryConsolidationLease<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for MemoryConsolidationLease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl NexusService {
    pub(super) fn capture_explicit_memory_tool(
        &self,
        store: &mut Store,
        project_id: &str,
        input: &ToolHookInput,
    ) -> Result<()> {
        if !is_memory_add_tool(&input.tool_name) {
            return Ok(());
        }
        let payload = input.tool_input.as_json();
        let content = payload
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("memory add tool requires content"))?;
        let scope = match payload.get("scope").and_then(serde_json::Value::as_str) {
            Some("global") => MemoryScope::Global,
            Some("project") => MemoryScope::project(project_id),
            _ => bail!("memory add tool requires an explicit global or project scope"),
        };
        store.add_memory_from_lifecycle(
            &scope,
            content,
            &MemoryProvenance {
                agent: input.context.agent.clone(),
                session_id: input.context.session_id.clone(),
                model_id: input.context.model.clone(),
                config_hash: self.loaded.hash.clone(),
            },
        )?;
        Ok(())
    }

    pub(super) fn memory_add(&self, command: MemoryAddCommand) -> ServiceResponse {
        let result = (|| -> Result<_> {
            let scope = match command.scope {
                MemoryScopeArg::Global => MemoryScope::Global,
                MemoryScopeArg::Project => MemoryScope::project(
                    workspace::identify(command.context.project_root.as_deref())?.id,
                ),
            };
            let provenance = MemoryProvenance {
                agent: command.context.agent,
                session_id: command.context.session_id,
                model_id: command.context.model,
                config_hash: self.loaded.hash.clone(),
            };
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .add_memory(&scope, &command.content, &provenance)
        })();
        match result {
            Ok(result) => ServiceResponse::MemoryWrite(MemoryWriteResponse { ok: true, result }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_search(&self, query: MemorySearchQuery) -> ServiceResponse {
        let result = (|| -> Result<_> {
            let scope = memory_read_scope(query.scope, query.project_root.as_deref())?;
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .search_memories(&scope, &query.regex, query.bounded_limit())
        })();
        match result {
            Ok(memories) => {
                ServiceResponse::MemoryEntries(MemoryEntriesResponse { ok: true, memories })
            }
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_context(&self, query: MemoryContextQuery) -> ServiceResponse {
        let result = (|| -> Result<_> {
            let scope = memory_read_scope(query.scope, query.project_root.as_deref())?;
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .memory_context(&scope, self.memory_context_limits())
        })();
        match result {
            Ok(context) => {
                ServiceResponse::MemoryContext(MemoryContextResponse { ok: true, context })
            }
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_expand(&self, query: MemorySummaryQuery) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.expand_memory_summary(&query.summary_id))
        {
            Ok(nodes) => ServiceResponse::MemoryNodes(MemoryNodesResponse { ok: true, nodes }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_invalidate(&self, command: MemorySummaryQuery) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.invalidate_memory_summary(&command.summary_id))
        {
            Ok(invalidated_summaries) => {
                ServiceResponse::MemoryInvalidation(MemoryInvalidationResponse {
                    ok: true,
                    invalidated_summaries,
                })
            }
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_status(&self, query: MemoryStatusQuery) -> ServiceResponse {
        let result = (|| -> Result<_> {
            let project_id = workspace::identify(query.project_root.as_deref())?.id;
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .memory_health(Some(&project_id), chrono::Utc::now())
        })();
        match result {
            Ok(health) => ServiceResponse::MemoryStatus(MemoryStatusResponse { ok: true, health }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_consolidate(&self) -> ServiceResponse {
        match self.consolidate_memories() {
            Ok(report) => ServiceResponse::MemoryConsolidation(report),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(crate) fn consolidate_memories(&self) -> Result<MemoryConsolidationReport> {
        if !self.loaded.config.memory.enabled {
            bail!("memory is disabled");
        }
        if self
            .memory_consolidation_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(MemoryConsolidationReport {
                ok: true,
                in_progress: true,
                ..MemoryConsolidationReport::default()
            });
        }
        let _lease = MemoryConsolidationLease(&self.memory_consolidation_in_flight);
        MemoryConsolidator { service: self }.run()
    }

    pub(super) fn memory_context_limits(&self) -> MemoryContextLimits {
        MemoryContextLimits {
            max_items: self
                .loaded
                .config
                .memory
                .context_max_items
                .min(crate::memory::DEFAULT_MEMORY_MAX_ITEMS),
            max_bytes: self
                .loaded
                .config
                .memory
                .context_max_bytes
                .min(crate::memory::DEFAULT_MEMORY_MAX_BYTES),
        }
    }
}

struct MemoryConsolidator<'a> {
    service: &'a NexusService,
}

impl MemoryConsolidator<'_> {
    fn run(&self) -> Result<MemoryConsolidationReport> {
        self.repair_stale_frontiers()?;
        let result = self.run_ready_jobs();
        self.repair_stale_frontiers()?;
        result
    }

    fn run_ready_jobs(&self) -> Result<MemoryConsolidationReport> {
        let primary = self.service.loaded.config.analyst.provider;
        let fallback = other_provider(primary);
        let jobs = self.ready_jobs(primary, fallback)?;
        if jobs.is_empty() {
            return Ok(MemoryConsolidationReport {
                ok: true,
                ..MemoryConsolidationReport::default()
            });
        }
        self.consolidate_jobs(primary, fallback, &jobs)
    }

    fn repair_stale_frontiers(&self) -> Result<()> {
        self.service
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .repair_stale_memory_frontiers(8)?;
        Ok(())
    }

    fn ready_jobs(
        &self,
        primary: ModelProvider,
        fallback: ModelProvider,
    ) -> Result<Vec<MemoryCompactionJob>> {
        let provider_names = [
            model::provider_name(primary),
            model::provider_name(fallback),
        ];
        self.service
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .ready_memory_compactions(
                &provider_names,
                Utc::now(),
                self.service
                    .loaded
                    .config
                    .memory
                    .consolidation_batch_size
                    .min(8),
            )
    }

    fn consolidate_jobs(
        &self,
        primary: ModelProvider,
        fallback: ModelProvider,
        jobs: &[MemoryCompactionJob],
    ) -> Result<MemoryConsolidationReport> {
        let mut report = MemoryConsolidationReport {
            ok: true,
            ..MemoryConsolidationReport::default()
        };
        let primary_jobs = self.provider_ready_jobs(jobs, primary)?;
        let completed = self.attempt_provider(primary, &primary_jobs, false, &mut report)?;
        let fallback_jobs = self
            .provider_ready_jobs(jobs, fallback)?
            .into_iter()
            .filter(|job| !completed.contains(&job.job_id))
            .collect::<Vec<_>>();
        let _ = self.attempt_provider(fallback, &fallback_jobs, true, &mut report)?;
        Ok(report)
    }

    fn provider_ready_jobs(
        &self,
        jobs: &[MemoryCompactionJob],
        provider: ModelProvider,
    ) -> Result<Vec<MemoryCompactionJob>> {
        let provider = model::provider_name(provider);
        let now = Utc::now();
        let mut store = self
            .service
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
        let mut ready = Vec::new();
        for job in jobs {
            if store.memory_provider_can_attempt(job, provider, now)? {
                ready.push(job.clone());
            }
        }
        Ok(ready)
    }

    fn attempt_provider(
        &self,
        provider: ModelProvider,
        jobs: &[MemoryCompactionJob],
        was_fallback: bool,
        report: &mut MemoryConsolidationReport,
    ) -> Result<HashSet<String>> {
        if jobs.is_empty() {
            return Ok(HashSet::new());
        }
        let attempt = model::invoke_memory_consolidation(
            &self.service.loaded.config.analyst,
            provider,
            jobs,
            None,
        );
        match attempt {
            Ok(result) => self.record_summaries(jobs, result, was_fallback, report),
            Err(failure) => self.record_failure(jobs, provider, failure, was_fallback, report),
        }
    }

    fn record_failure(
        &self,
        jobs: &[MemoryCompactionJob],
        provider: ModelProvider,
        failure: model::MemoryConsolidationFailure,
        was_fallback: bool,
        report: &mut MemoryConsolidationReport,
    ) -> Result<HashSet<String>> {
        let outcome = match failure.kind {
            MemoryConsolidationFailureKind::Invocation => MemoryCompactionOutcome::Failed,
            MemoryConsolidationFailureKind::Timeout => MemoryCompactionOutcome::TimedOut,
            MemoryConsolidationFailureKind::InvalidOutput => MemoryCompactionOutcome::InvalidOutput,
        };
        let model_id = selected_model_id(&self.service.loaded.config.analyst, provider);
        let provider_name = model::provider_name(provider);
        let mut store = self
            .service
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
        for job in jobs {
            store.record_memory_compaction_attempt(
                job,
                &MemoryCompactionAttempt {
                    provider: provider_name.into(),
                    model_id: model_id.clone(),
                    outcome,
                    diagnostic: Some(failure.diagnostic().into()),
                    was_fallback,
                    attempted_at: Utc::now(),
                },
            )?;
        }
        report.attempted += jobs.len();
        report.failed += jobs.len();
        if was_fallback {
            report.fallback_uses += jobs.len();
        }
        Ok(HashSet::new())
    }

    fn record_summaries(
        &self,
        jobs: &[MemoryCompactionJob],
        result: model::MemoryConsolidationResult,
        was_fallback: bool,
        report: &mut MemoryConsolidationReport,
    ) -> Result<HashSet<String>> {
        let provider = model::provider_name(result.provider);
        let mut completed = HashSet::new();
        let mut affected_scopes = HashSet::new();
        let mut store = self
            .service
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
        for (job, summary) in jobs.iter().zip(result.summaries) {
            let inserted = store.insert_memory_summary(
                job,
                &summary.summary,
                provider,
                result.model_id.as_deref(),
            )?;
            let outcome = match inserted {
                MemorySummaryInsert::Inserted(_) | MemorySummaryInsert::AlreadyPresent(_) => {
                    report.succeeded += 1;
                    affected_scopes.insert(job.scope.clone());
                    MemoryCompactionOutcome::Succeeded
                }
                MemorySummaryInsert::Stale => {
                    report.stale += 1;
                    MemoryCompactionOutcome::Stale
                }
            };
            store.record_memory_compaction_attempt(
                job,
                &MemoryCompactionAttempt {
                    provider: provider.into(),
                    model_id: result.model_id.clone(),
                    outcome,
                    diagnostic: None,
                    was_fallback,
                    attempted_at: Utc::now(),
                },
            )?;
            report.attempted += 1;
            if was_fallback {
                report.fallback_uses += 1;
            }
            completed.insert(job.job_id.clone());
        }
        for scope in affected_scopes {
            store.repair_memory_frontier(&scope)?;
        }
        Ok(completed)
    }
}

fn is_memory_add_tool(tool_name: &str) -> bool {
    tool_name == "nexus_memory_add" || tool_name.ends_with("__nexus_memory_add")
}

fn other_provider(provider: ModelProvider) -> ModelProvider {
    match provider {
        ModelProvider::Codex => ModelProvider::Claude,
        ModelProvider::Claude => ModelProvider::Codex,
    }
}

fn selected_model_id(
    config: &crate::config::AnalystConfig,
    provider: ModelProvider,
) -> Option<String> {
    match provider {
        ModelProvider::Codex => config.codex.model.clone(),
        ModelProvider::Claude => config.claude.model.clone(),
    }
}

fn memory_read_scope(
    scope: MemoryReadScopeArg,
    project_root: Option<&str>,
) -> Result<MemoryReadScope> {
    Ok(match scope {
        MemoryReadScopeArg::Global => MemoryReadScope::Global,
        MemoryReadScopeArg::Project => {
            MemoryReadScope::project(workspace::identify(project_root)?.id)
        }
        MemoryReadScopeArg::Layered => {
            MemoryReadScope::layered(workspace::identify(project_root)?.id)
        }
    })
}

#[cfg(test)]
mod tests;
