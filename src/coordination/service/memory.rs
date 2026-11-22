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
    MemoryEntriesResponse, MemoryInspection, MemoryInvalidationResponse, MemoryNodesResponse,
    MemoryReadScopeArg, MemoryScopeArg, MemorySearchQuery, MemoryStatusQuery, MemoryStatusResponse,
    MemorySummaryQuery, MemoryWriteResponse, ServiceResponse,
};
use crate::coordination::domain::ToolHookInput;
use crate::coordination::workspace::ProjectBinding;
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

    pub(super) fn memory_add(
        &self,
        command: MemoryAddCommand,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        let result = (|| -> Result<_> {
            let scope = match command.scope {
                MemoryScopeArg::Global => MemoryScope::Global,
                MemoryScopeArg::Project => MemoryScope::project(
                    self.resolve_project(binding, command.context.project_root.as_deref())?
                        .id,
                ),
            };
            let provenance = MemoryProvenance {
                agent: command.context.agent,
                session_id: command.context.session_id,
                model_id: command.context.model,
                config_hash: self.loaded.hash.clone(),
            };
            self.write_store()?
                .add_memory(&scope, &command.content, &provenance)
        })();
        match result {
            Ok(result) => ServiceResponse::MemoryWrite(MemoryWriteResponse { ok: true, result }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_search(
        &self,
        query: MemorySearchQuery,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        self.read_memory(
            binding,
            query.scope,
            query.project_root.as_deref(),
            |store, scope| store.search_memories(scope, &query.regex, query.bounded_limit()),
        )
        .map(|memories| {
            ServiceResponse::MemoryEntries(MemoryEntriesResponse { ok: true, memories })
        })
        .unwrap_or_else(ServiceResponse::error)
    }

    pub(super) fn memory_context(
        &self,
        query: MemoryContextQuery,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        self.read_memory(
            binding,
            query.scope,
            query.project_root.as_deref(),
            |store, scope| store.memory_context(scope, self.memory_context_limits()),
        )
        .map(|context| ServiceResponse::MemoryContext(MemoryContextResponse { ok: true, context }))
        .unwrap_or_else(ServiceResponse::error)
    }

    fn read_memory<T>(
        &self,
        binding: &ProjectBinding,
        scope: MemoryReadScopeArg,
        root: Option<&str>,
        read: impl FnOnce(&mut Store, &MemoryReadScope) -> Result<T>,
    ) -> Result<T> {
        let scope = memory_read_scope(self, binding, scope, root)?;
        let mut store = self.read_store()?;
        read(&mut store, &scope)
    }

    pub(super) fn inspect_memory(&self, query: MemoryInspection) -> ServiceResponse {
        match query {
            MemoryInspection::Status { project_id } => {
                match inspection_binding(Some(project_id), MemoryReadScopeArg::Project) {
                    Ok(binding) => self.memory_status(MemoryStatusQuery::default(), &binding),
                    Err(error) => ServiceResponse::error(error),
                }
            }
            MemoryInspection::Search {
                project_id,
                scope,
                regex,
                limit,
            } => {
                if regex.len() > crate::coordination::api::MAX_MEMORY_SEARCH_REGEX_BYTES {
                    return ServiceResponse::error("memory search regex exceeds its limit");
                }
                let binding = match inspection_binding(project_id, scope) {
                    Ok(binding) => binding,
                    Err(error) => return ServiceResponse::error(error),
                };
                self.memory_search(
                    MemorySearchQuery {
                        scope,
                        regex,
                        limit,
                        project_root: None,
                    },
                    &binding,
                )
            }
            MemoryInspection::Context { project_id, scope } => {
                match inspection_binding(project_id, scope) {
                    Ok(binding) => self.memory_context(
                        MemoryContextQuery {
                            scope,
                            project_root: None,
                        },
                        &binding,
                    ),
                    Err(error) => ServiceResponse::error(error),
                }
            }
        }
    }

    pub(super) fn memory_expand(&self, query: MemorySummaryQuery) -> ServiceResponse {
        match self
            .read_store()
            .and_then(|mut store| store.expand_memory_summary(&query.summary_id))
        {
            Ok(nodes) => ServiceResponse::MemoryNodes(MemoryNodesResponse { ok: true, nodes }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(super) fn memory_invalidate(&self, command: MemorySummaryQuery) -> ServiceResponse {
        match self
            .write_store()
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

    pub(super) fn memory_status(
        &self,
        query: MemoryStatusQuery,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        let result = (|| -> Result<_> {
            let project_id = self
                .resolve_project(binding, query.project_root.as_deref())?
                .id;
            self.read_store()?
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
        if !self.try_claim_memory_consolidation()? {
            return Ok(MemoryConsolidationReport {
                ok: true,
                in_progress: true,
                ..MemoryConsolidationReport::default()
            });
        }
        self.run_claimed_memory_consolidation()
    }

    pub(crate) fn try_claim_memory_consolidation(&self) -> Result<bool> {
        if !self.loaded.config.memory.enabled {
            bail!("memory is disabled");
        }
        Ok(self
            .memory_consolidation_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok())
    }

    pub(crate) fn run_claimed_memory_consolidation(&self) -> Result<MemoryConsolidationReport> {
        debug_assert!(
            self.memory_consolidation_in_flight.load(Ordering::Acquire),
            "memory consolidation must be claimed before it runs"
        );
        let _lease = MemoryConsolidationLease(&self.memory_consolidation_in_flight);
        MemoryConsolidator { service: self }.run()
    }

    pub(crate) fn memory_schedule_seconds(&self) -> Option<u64> {
        self.loaded
            .config
            .memory
            .enabled
            .then_some(self.loaded.config.memory.consolidation_interval_seconds)
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

enum MemoryProviderAttempt {
    Primary(ModelProvider),
    Fallback(ModelProvider),
}

struct RecordedMemoryAttempt<'a> {
    job: &'a MemoryCompactionJob,
    outcome: MemoryCompactionOutcome,
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

    fn consolidate_jobs(
        &self,
        primary: ModelProvider,
        fallback: ModelProvider,
        jobs: &[MemoryCompactionJob],
    ) -> Result<MemoryConsolidationReport> {
        let primary_jobs = self.provider_ready_jobs(jobs, primary)?;
        let primary_attempts =
            self.attempt_provider(MemoryProviderAttempt::Primary(primary), &primary_jobs)?;
        let fallback_jobs = self
            .provider_ready_jobs(jobs, fallback)?
            .into_iter()
            .filter(|job| {
                !primary_attempts.iter().any(|attempt| {
                    attempt.job.job_id == job.job_id && !attempt.outcome.is_failure()
                })
            })
            .collect::<Vec<_>>();
        let fallback_attempts =
            self.attempt_provider(MemoryProviderAttempt::Fallback(fallback), &fallback_jobs)?;
        let report = MemoryConsolidationReport {
            ok: true,
            fallback_uses: fallback_attempts.len(),
            ..MemoryConsolidationReport::default()
        };
        Ok(primary_attempts
            .into_iter()
            .chain(fallback_attempts)
            .fold(report, |report, attempt| {
                let report = MemoryConsolidationReport {
                    attempted: report.attempted + 1,
                    ..report
                };
                match attempt.outcome {
                    MemoryCompactionOutcome::Succeeded => MemoryConsolidationReport {
                        succeeded: report.succeeded + 1,
                        ..report
                    },
                    MemoryCompactionOutcome::Stale => MemoryConsolidationReport {
                        stale: report.stale + 1,
                        ..report
                    },
                    MemoryCompactionOutcome::Failed
                    | MemoryCompactionOutcome::TimedOut
                    | MemoryCompactionOutcome::InvalidOutput => MemoryConsolidationReport {
                        failed: report.failed + 1,
                        ..report
                    },
                }
            }))
    }

    fn repair_stale_frontiers(&self) -> Result<()> {
        self.service
            .maintenance_store()?
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
        self.service.maintenance_store()?.ready_memory_compactions(
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

    fn provider_ready_jobs(
        &self,
        jobs: &[MemoryCompactionJob],
        provider: ModelProvider,
    ) -> Result<Vec<MemoryCompactionJob>> {
        let provider = model::provider_name(provider);
        let now = Utc::now();
        let mut store = self.service.maintenance_store()?;
        jobs.iter()
            .filter_map(|job| {
                store
                    .memory_provider_can_attempt(job, provider, now)
                    .map(|ready| ready.then(|| job.clone()))
                    .transpose()
            })
            .collect()
    }

    fn attempt_provider<'a>(
        &self,
        attempt: MemoryProviderAttempt,
        jobs: &'a [MemoryCompactionJob],
    ) -> Result<Vec<RecordedMemoryAttempt<'a>>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let provider = match &attempt {
            MemoryProviderAttempt::Primary(provider)
            | MemoryProviderAttempt::Fallback(provider) => *provider,
        };
        let analyst = &self.service.loaded.config.analyst;
        let result = model::invoke_memory_consolidation(analyst, provider, jobs, None);
        let mut store = self.service.maintenance_store()?;
        let recorded = jobs
            .iter()
            .map(|job| {
                let outcome = self.record_provider_result(&mut store, job, &result, &attempt)?;
                Ok(RecordedMemoryAttempt { job, outcome })
            })
            .collect::<Result<Vec<_>>>()?;
        let affected_scopes = recorded
            .iter()
            .filter(|attempt| attempt.outcome == MemoryCompactionOutcome::Succeeded)
            .map(|attempt| &attempt.job.scope)
            .collect::<HashSet<_>>();
        affected_scopes
            .into_iter()
            .try_for_each(|scope| store.repair_memory_frontier(scope).map(|_| ()))?;
        Ok(recorded)
    }

    fn record_provider_result(
        &self,
        store: &mut Store,
        job: &MemoryCompactionJob,
        result: &std::result::Result<
            model::MemoryConsolidationResult,
            model::MemoryConsolidationFailure,
        >,
        attempt: &MemoryProviderAttempt,
    ) -> Result<MemoryCompactionOutcome> {
        let (provider, was_fallback) = match attempt {
            MemoryProviderAttempt::Primary(provider) => (*provider, false),
            MemoryProviderAttempt::Fallback(provider) => (*provider, true),
        };
        let (provider, model_id) = match result {
            Ok(result) => (
                model::provider_name(result.provider),
                result.model_id.as_deref(),
            ),
            Err(_) => (
                model::provider_name(provider),
                self.service
                    .loaded
                    .config
                    .analyst
                    .model(provider)
                    .model
                    .as_deref(),
            ),
        };
        let (outcome, diagnostic) = match result {
            Ok(result) => {
                // The model adapter validates exactly one summary for every job.
                let summary = result
                    .summaries
                    .iter()
                    .find(|summary| summary.job_id == job.job_id)
                    .expect("validated memory summary job");
                let inserted =
                    store.insert_memory_summary(job, &summary.summary, provider, model_id)?;
                let outcome = match inserted {
                    MemorySummaryInsert::Inserted(_) | MemorySummaryInsert::AlreadyPresent(_) => {
                        MemoryCompactionOutcome::Succeeded
                    }
                    MemorySummaryInsert::Stale => MemoryCompactionOutcome::Stale,
                };
                (outcome, None)
            }
            Err(failure) => (
                match failure.kind {
                    MemoryConsolidationFailureKind::Invocation => MemoryCompactionOutcome::Failed,
                    MemoryConsolidationFailureKind::Timeout => MemoryCompactionOutcome::TimedOut,
                    MemoryConsolidationFailureKind::InvalidOutput => {
                        MemoryCompactionOutcome::InvalidOutput
                    }
                },
                Some(failure.diagnostic().into()),
            ),
        };
        store.record_memory_compaction_attempt(
            job,
            &MemoryCompactionAttempt {
                provider: provider.into(),
                model_id: model_id.map(str::to_owned),
                outcome,
                diagnostic,
                was_fallback,
                attempted_at: Utc::now(),
            },
        )?;
        Ok(outcome)
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

fn memory_read_scope(
    service: &NexusService,
    binding: &ProjectBinding,
    scope: MemoryReadScopeArg,
    project_root: Option<&str>,
) -> Result<MemoryReadScope> {
    Ok(match scope {
        MemoryReadScopeArg::Global => MemoryReadScope::Global,
        MemoryReadScopeArg::Project => {
            MemoryReadScope::project(service.resolve_project(binding, project_root)?.id)
        }
        MemoryReadScopeArg::Layered => {
            MemoryReadScope::layered(service.resolve_project(binding, project_root)?.id)
        }
    })
}

#[cfg(test)]
mod tests;

fn inspection_binding(
    project_id: Option<String>,
    scope: MemoryReadScopeArg,
) -> Result<ProjectBinding> {
    let project_id = match (project_id, scope) {
        (Some(project), _) => crate::config::canonical_project_id(&project)
            .map_err(|_| anyhow::anyhow!("invalid memory inspection project"))?
            .to_owned(),
        (None, MemoryReadScopeArg::Global) => "browser-global-memory".into(),
        (None, _) => bail!("select a project for memory inspection"),
    };
    Ok(ProjectBinding::Inspect {
        project_id: Some(project_id),
    })
}
