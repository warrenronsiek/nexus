// @feature audit-registry
// @spec docs/features/audit-registry.md
// @feature code-concerns
// @spec docs/features/code-concerns.md
// @feature evals
// @spec docs/features/evals.md
// @feature coordination
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
mod analytics;
mod audit;
mod concerns;
mod conflict_closure;
mod database;
mod database_operations;
mod delivery;
mod delivery_watch;
mod dispatch;
mod evals;
mod feature_ops;
mod history_ops;
mod memory;
mod papercuts;
mod project_catalog;
mod prompt;
mod records;
mod session_history;
mod sos;
mod store_lock;
mod work;

use database::Database;
pub(crate) use database::DatabaseOperation;
use database_operations::StorageOperation;
pub(crate) use papercuts::{PapercutDispatch, PapercutReviewJob};
pub(crate) use sos::{SosDispatch, SosTriageJob};
use store_lock::LifecycleStoreBusy;

use super::api::{
    AnalysisResponse, AnalyzeCommand, CoordinationSummaryResponse, DashboardQuery,
    DashboardResponse, EventQuery, EventsResponse, ProjectsResponse, PromptResponse,
    ReleaseCommand, ReleasedResponse, ResolveCommand, ResolvedResponse, ServiceRequest,
    ServiceResponse, StatusResponse, UiPreferencesResponse,
};
use super::classifier::{
    advisory, bounded_reported_intents, extract_path_intents, strongest_overlaps,
};
use super::client_evidence::{
    capture_changed_paths_page, ClientDerivedUsage, ClientFilesystemEvidence,
};
use super::domain::{
    Advisory, ConflictRecord, HookResponse, IgnoredAdvisoryPolicy, PathIntent, SessionCounterpart,
    SessionStopInput, ToolCompletion, ToolHookInput, ToolResultEvent, UserPromptInput,
};
use super::usage::UsageCapture;
use super::workspace;
use super::workspace::ProjectBinding;
use crate::agents::analyst::{self, AnalysisResult};
use crate::agents::model;
use crate::config::LoadedConfig;
use crate::persistence::Store;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const LIFECYCLE_STORE_RETRY: Duration = Duration::from_millis(1);
const RECONCILIATION_BATCH_PATHS: usize = 32;
const POSTGRES_STORE_STALL_THRESHOLD: Duration = Duration::from_secs(30);

pub struct NexusService {
    pub(super) loaded: LoadedConfig,
    store: Database,
    pub(super) usage: UsageCapture,
    pub(super) memory_consolidation_in_flight: AtomicBool,
    project_catalog: project_catalog::ProjectCatalog,
}

impl NexusService {
    pub fn new(loaded: LoadedConfig) -> Result<Self> {
        let store = Store::open(&loaded.config.storage.database_path)?;
        Ok(Self::with_database(loaded, Database::single(store)))
    }

    pub fn connect(loaded: LoadedConfig, database_url: &str) -> Result<Self> {
        Ok(Self::with_database(
            loaded,
            Database::connect(database_url)?,
        ))
    }

    fn with_database(loaded: LoadedConfig, store: Database) -> Self {
        let usage = UsageCapture::new(loaded.hash.clone());
        Self {
            loaded,
            store,
            usage,
            memory_consolidation_in_flight: AtomicBool::new(false),
            project_catalog: Default::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_store(loaded: LoadedConfig, store: Store) -> Self {
        Self::with_database(loaded, Database::single(store))
    }

    fn pre_tool_use(
        &self,
        input: ToolHookInput,
        binding: &ProjectBinding,
        evidence: Option<ClientDerivedUsage>,
    ) -> ServiceResponse {
        ServiceResponse::Hook(
            pre_tool_use_result(self, input, binding, evidence.as_ref())
                .unwrap_or_else(lifecycle_failure),
        )
    }

    fn post_tool_use(
        &self,
        input: ToolHookInput,
        completion: ToolCompletion,
        binding: &ProjectBinding,
        client_evidence: Option<ClientDerivedUsage>,
    ) -> ServiceResponse {
        let result = (|| -> Result<HookResponse> {
            let project = self.resolve_project(binding, input.context.project_root.as_deref())?;
            let client_evidence =
                analytics::local_evidence(client_evidence, &project, &input, completion);
            let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
            store.touch_session(
                &project.id,
                &input.context.session_id,
                &input.context.agent,
                project.worktree.as_deref(),
                &self.loaded.hash,
            )?;
            let ignored_advisory_policy =
                if self.loaded.config.coordination.record_ignored_advisories {
                    IgnoredAdvisoryPolicy::Record
                } else {
                    IgnoredAdvisoryPolicy::Omit
                };
            let _ = self.observe_activity(
                &mut store,
                &project,
                &input,
                completion,
                client_evidence.as_ref(),
            );
            self.usage.observe_tool_completion(
                &mut store,
                &project,
                &input,
                completion,
                client_evidence,
            );
            store.record_hook_result(
                &project.id,
                &input.context.session_id,
                &input.tool_use_id,
                completion,
                ToolResultEvent {
                    tool_name: input.tool_name,
                    output: input.tool_output,
                    error: input.error,
                },
                ignored_advisory_policy,
            )?;
            Ok(HookResponse::allow(Vec::new()))
        })();
        ServiceResponse::Hook(result.unwrap_or_else(lifecycle_failure))
    }

    fn session_stop(&self, input: SessionStopInput, binding: &ProjectBinding) -> ServiceResponse {
        let result = (|| -> Result<HookResponse> {
            let project = self.resolve_project(binding, input.context.project_root.as_deref())?;
            let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
            store.stop_session(&project.id, &input.context.session_id)?;
            // Projection maintenance; the periodic closure pass retries a failure.
            let _ = store.end_stale_conflicts(Some(&project.id));
            Ok(HookResponse::allow(Vec::new()))
        })();
        ServiceResponse::Hook(result.unwrap_or_else(lifecycle_failure))
    }

    pub(super) fn read_metadata(
        &self,
        request: ServiceRequest,
        project: &ProjectBinding,
    ) -> ServiceResponse {
        match request {
            ServiceRequest::Status => self.status(),
            ServiceRequest::CoordinationSummary(query) => {
                let project = project.default_read_project(query.project_id);
                match self.read_store().and_then(|mut store| {
                    store.coordination_summary(
                        project.as_deref(),
                        self.loaded.config.coordination.claim_ttl_seconds,
                        query.since,
                    )
                }) {
                    Ok(counts) => {
                        ServiceResponse::CoordinationSummary(CoordinationSummaryResponse {
                            ok: true,
                            counts,
                        })
                    }
                    Err(error) => ServiceResponse::error(error),
                }
            }
            ServiceRequest::UiPreferences => {
                ServiceResponse::UiPreferences(UiPreferencesResponse {
                    ok: true,
                    refresh_interval_ms: self.loaded.config.ui.refresh_interval_ms,
                })
            }
            _ => unreachable!("non-read request routed as read"),
        }
    }

    fn status(&self) -> ServiceResponse {
        match self
            .read_store()
            .and_then(|mut store| store.counts(self.loaded.config.coordination.claim_ttl_seconds))
        {
            Ok(counts) => ServiceResponse::Status(StatusResponse {
                ok: true,
                status: "running",
                counts,
                config_hash: self.loaded.hash.clone(),
            }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(crate) fn readiness_status(&self) -> Result<&'static str> {
        self.readiness_status_with_store_stall_threshold(POSTGRES_STORE_STALL_THRESHOLD)
    }

    fn readiness_status_with_store_stall_threshold(
        &self,
        store_stall_threshold: Duration,
    ) -> Result<&'static str> {
        self.store.readiness(
            self.loaded.config.coordination.claim_ttl_seconds,
            store_stall_threshold,
        )
    }

    fn events(&self, mut query: EventQuery, binding: &ProjectBinding) -> ServiceResponse {
        query.project_id = binding.default_read_project(query.project_id);
        match self.read_store().and_then(|mut store| {
            store.list_events(query.project_id.as_deref(), query.bounded_limit())
        }) {
            Ok(events) => ServiceResponse::Events(EventsResponse { ok: true, events }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn resolve(&self, command: ResolveCommand) -> ServiceResponse {
        match self
            .write_store()
            .and_then(|mut store| store.resolve_conflict(&command.conflict_id, &command.resolution))
        {
            Ok(true) => ServiceResponse::Resolved(ResolvedResponse {
                ok: true,
                conflict_id: command.conflict_id,
                status: "resolved",
            }),
            Ok(false) => ServiceResponse::error("conflict not found"),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn release(&self, command: ReleaseCommand, binding: &ProjectBinding) -> ServiceResponse {
        let release = command.release();
        let result = (|| -> Result<usize> {
            let project = self.resolve_project(binding, None)?;
            let mut store = self.write_store()?;
            let released = store.release_claims(&project.id, &command.session_id, &release)?;
            // Projection maintenance; the periodic closure pass retries a failure.
            let _ = store.end_stale_conflicts(Some(&project.id));
            Ok(released)
        })();
        match result {
            Ok(released) => ServiceResponse::Released(ReleasedResponse { ok: true, released }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn analyze(&self, command: AnalyzeCommand, binding: &ProjectBinding) -> ServiceResponse {
        match analyze_result(self, &command, binding) {
            Ok(response) => ServiceResponse::Analysis(response),
            Err(error) if binding.is_remote() => {
                let diagnostic = if model::is_timeout(&error) {
                    "analysis timed out"
                } else {
                    "analysis failed"
                };
                ServiceResponse::error(diagnostic)
            }
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn projects(&self) -> ServiceResponse {
        match self
            .read_store()
            .and_then(|mut store| store.list_projects_window())
        {
            Ok(window) => ServiceResponse::Projects(ProjectsResponse {
                ok: true,
                projects: window.items,
                truncated: window.truncated,
            }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn dashboard(&self, mut query: DashboardQuery, binding: &ProjectBinding) -> ServiceResponse {
        query.project_id = binding.default_read_project(query.project_id);
        match self.read_store().and_then(|mut store| {
            store.dashboard_window(
                query.project_id.as_deref(),
                self.loaded.config.ui.recent_event_limit,
                self.loaded.config.ui.recent_record_limit,
                self.loaded.config.coordination.claim_ttl_seconds,
                query.since,
            )
        }) {
            Ok(records) => ServiceResponse::Dashboard(DashboardResponse {
                ok: true,
                project_id: query.project_id,
                generated_at: chrono::Utc::now(),
                refresh_interval_ms: self.loaded.config.ui.refresh_interval_ms,
                records,
            }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    pub(crate) fn reconcile_all(&self) -> Result<usize> {
        self.reconcile_all_with(|worktree| capture_changed_paths_page(worktree, 0).paths)
    }

    fn reconcile_all_with(
        &self,
        mut observe: impl FnMut(&str) -> Vec<PathIntent>,
    ) -> Result<usize> {
        let ttl_seconds = self.loaded.config.coordination.claim_ttl_seconds;
        let sessions = self
            .lifecycle_store()
            .context("acquire reconciliation store")?
            .reconcilable_sessions(ttl_seconds)?;
        let mut sessions_by_project_worktree = BTreeMap::<(String, String), Vec<String>>::new();
        for session in sessions {
            let Some(worktree) = session.worktree else {
                continue;
            };
            sessions_by_project_worktree
                .entry((session.project_id, worktree))
                .or_default()
                .push(session.session_id);
        }
        let mut observed = 0_usize;
        for ((project_id, worktree), session_ids) in sessions_by_project_worktree {
            let intents = observe(&worktree);
            observed = observed.saturating_add(intents.len().saturating_mul(session_ids.len()));
            for session_id in session_ids {
                reconcile_chunks(self, &project_id, &session_id, &intents, ttl_seconds)?;
            }
        }
        Ok(observed)
    }

    pub(crate) fn reconcile_remote_evidence(
        &self,
        project_id: &str,
        evidence: &ClientFilesystemEvidence,
        ensure_session: bool,
    ) -> Result<usize> {
        if ensure_session {
            self.lifecycle_store()
                .context("acquire lifecycle store")?
                .ensure_session(
                    project_id,
                    &evidence.session_id,
                    &evidence.agent,
                    &self.loaded.hash,
                )?;
        }
        reconcile_chunks(
            self,
            project_id,
            &evidence.session_id,
            &evidence.changed_paths,
            self.loaded.config.coordination.claim_ttl_seconds,
        )?;
        Ok(evidence.changed_paths.len())
    }
}

fn pre_tool_use_result(
    service: &NexusService,
    input: ToolHookInput,
    binding: &ProjectBinding,
    evidence: Option<&ClientDerivedUsage>,
) -> Result<HookResponse> {
    let project = service.resolve_project(binding, input.context.project_root.as_deref())?;
    let inventory = evidence
        .and_then(|evidence| evidence.repository_inventory.clone())
        .or_else(|| {
            project.worktree.as_deref().and_then(|root| {
                crate::analytics::inventory::capture_once(
                    std::path::Path::new(root),
                    &input.context,
                )
            })
        });
    let root = project.claim_root.as_deref();
    let intents = if input.client_path_intents.is_empty() {
        extract_path_intents(&input.tool_name, &input.tool_input, root)
    } else {
        bounded_reported_intents(&input.client_path_intents, root)
    };
    let mut store = service
        .lifecycle_store()
        .context("acquire lifecycle store")?;
    if let Some(snapshot) = inventory {
        let _ = store.record_inventory(
            &project.id,
            &input.context,
            "repository",
            &snapshot.resources,
            snapshot.complete,
        );
    }
    let _ = store.record_activity_call(&project.id, &input, &[], "started");
    record_tool_start(service, &mut store, &project, &input, &intents)?;

    let mut advisories = Vec::new();
    for intent in intents {
        let remaining =
            crate::coordination::api::MAX_LIFECYCLE_ADVISORIES.saturating_sub(advisories.len());
        advisories.extend(record_intent_advisories(
            service,
            &mut store,
            &project.id,
            &input,
            intent,
            remaining,
        )?);
    }
    Ok(HookResponse::allow(advisories))
}

fn record_tool_start(
    service: &NexusService,
    store: &mut Store,
    project: &workspace::ProjectIdentity,
    input: &ToolHookInput,
    intents: &[PathIntent],
) -> Result<()> {
    store.touch_session(
        &project.id,
        &input.context.session_id,
        &input.context.agent,
        project.worktree.as_deref(),
        &service.loaded.hash,
    )?;
    service.capture_explicit_memory_tool(store, &project.id, input)?;
    service.usage.observe_tool_start(store, &project.id, input);
    store.record_tool_inspection(
        &project.id,
        &input.context.session_id,
        &input.tool_use_id,
        &input.tool_name,
        intents,
    )?;
    Ok(())
}

fn record_intent_advisories(
    service: &NexusService,
    store: &mut Store,
    project_id: &str,
    input: &ToolHookInput,
    intent: PathIntent,
    advisory_limit: usize,
) -> Result<Vec<Advisory>> {
    let active = if advisory_limit == 0 {
        Vec::new()
    } else {
        store.active_claims_for_path(
            project_id,
            &intent,
            &input.context.session_id,
            advisory_limit,
        )?
    };
    let claim = store.insert_claim(
        project_id,
        &input.context.session_id,
        &input.tool_use_id,
        &intent,
        service.loaded.config.coordination.claim_ttl_seconds,
    )?;
    strongest_overlaps(&intent, active)
        .into_iter()
        .map(|(existing, classification)| {
            // Naming is presentation only; an unavailable label must not cost the advisory.
            let counterpart = store
                .session_counterpart(project_id, &existing.session_id)
                .unwrap_or_else(|_| SessionCounterpart::unnamed(&existing.session_id));
            let item = advisory(classification, &intent.path, &counterpart);
            store.record_conflict(project_id, &claim, &existing, item, &input.tool_use_id)
        })
        .collect()
}

fn analyze_result(
    service: &NexusService,
    command: &AnalyzeCommand,
    binding: &ProjectBinding,
) -> Result<AnalysisResponse> {
    let conflict = conflict(service, &command.conflict_id)?;
    let trusted_worktree = if binding.is_remote() {
        None
    } else {
        service.read_store()?.conflict_worktree(&conflict)?
    };
    let analysis = analyst::analyze(
        &service.loaded.config.analyst,
        &conflict,
        trusted_worktree.as_deref().map(std::path::Path::new),
    )?;
    record_analysis(service, &conflict, &command.conflict_id, &analysis)?;
    Ok(AnalysisResponse { ok: true, analysis })
}

fn conflict(service: &NexusService, conflict_id: &str) -> Result<ConflictRecord> {
    service
        .read_store()?
        .conflict_by_id(conflict_id)?
        .context("conflict not found")
}

fn record_analysis(
    service: &NexusService,
    conflict: &ConflictRecord,
    conflict_id: &str,
    analysis: &AnalysisResult,
) -> Result<()> {
    service.write_store()?.record_analysis(
        &conflict.project_id,
        conflict_id,
        analysis,
        &service.loaded.hash,
    )
}

#[cfg(test)]
mod project_catalog_tests;
#[cfg(test)]
mod project_isolation_tests;
#[cfg(test)]
mod session_history_tests;
#[cfg(test)]
mod store_contention_tests;
#[cfg(test)]
mod tests;

pub(super) fn lifecycle_failure(error: anyhow::Error) -> HookResponse {
    let retryable = error.is::<LifecycleStoreBusy>();
    let mut response = HookResponse::fail_open(error);
    response.retryable = retryable;
    response
}

fn reconcile_chunks(
    service: &NexusService,
    project_id: &str,
    session_id: &str,
    intents: &[PathIntent],
    ttl_seconds: i64,
) -> Result<()> {
    for (index, chunk) in intents.chunks(RECONCILIATION_BATCH_PATHS).enumerate() {
        if index != 0 {
            // Give pending foreground hooks a full store-acquisition retry interval.
            std::thread::sleep(LIFECYCLE_STORE_RETRY);
        }
        service
            .lifecycle_store()
            .context("acquire reconciliation store")?
            .record_reconciliation(project_id, session_id, chunk, ttl_seconds)?;
    }
    Ok(())
}
