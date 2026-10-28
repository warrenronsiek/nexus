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
mod concerns;
mod dispatch;
mod evals;
mod feature_ops;
mod history_ops;
mod memory;
mod papercuts;
mod prompt;
mod records;
mod sos;
mod store_lock;
mod work;

pub(crate) use papercuts::{PapercutDispatch, PapercutReviewJob};
use prompt::synopsis;
pub(crate) use sos::{SosDispatch, SosTriageJob};
use store_lock::{TrackedStoreGuard, TrackedStoreMutex};

#[cfg(test)]
use super::api::ServiceRequest;
use super::api::{
    AnalysisResponse, AnalyzeCommand, DashboardQuery, DashboardResponse, EventQuery,
    EventsResponse, ProjectsResponse, PromptResponse, ReleaseCommand, ReleasedResponse,
    ResolveCommand, ResolvedResponse, ServiceResponse, StatusResponse,
};
use super::classifier::{advisory, extract_path_intents, strongest_overlaps};
use super::client_evidence::{
    capture_changed_paths_page, ClientDerivedUsage, ClientFilesystemEvidence,
};
use super::domain::{
    Advisory, ConflictRecord, HookResponse, IgnoredAdvisoryPolicy, PathIntent, SessionStopInput,
    ToolCompletion, ToolHookInput, ToolResultEvent, UserPromptInput,
};
use super::usage::UsageCapture;
use super::workspace;
use super::workspace::ProjectBinding;
use crate::agents::analyst::{self, AnalysisResult};
use crate::agents::model;
use crate::config::LoadedConfig;
use crate::memory::MemoryReadScope;
#[cfg(feature = "postgres")]
use crate::persistence::DatabaseReadinessProbe;
use crate::persistence::Store;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
#[cfg(feature = "postgres")]
use std::sync::Mutex;
use std::sync::TryLockError;
use std::time::Duration;

const LIFECYCLE_STORE_WAIT: Duration = Duration::from_millis(100);
const LIFECYCLE_STORE_RETRY: Duration = Duration::from_millis(1);
const POSTGRES_STORE_STALL_THRESHOLD: Duration = Duration::from_secs(30);

pub struct NexusService {
    pub(super) loaded: LoadedConfig,
    pub(super) store: TrackedStoreMutex,
    pub(super) usage: UsageCapture,
    pub(super) memory_consolidation_in_flight: AtomicBool,
    readiness: ServiceReadiness,
}

enum ServiceReadiness {
    SharedStore,
    #[cfg(feature = "postgres")]
    Dedicated(Mutex<DatabaseReadinessProbe>),
}

impl NexusService {
    pub fn new(loaded: LoadedConfig) -> Result<Self> {
        let store = Store::open(&loaded.config.storage.database_path)?;
        Ok(Self::from_parts(loaded, store))
    }

    pub fn connect(loaded: LoadedConfig, database_url: &str) -> Result<Self> {
        let store = Store::connect(database_url)?;
        #[cfg(feature = "postgres")]
        let readiness =
            ServiceReadiness::Dedicated(Mutex::new(DatabaseReadinessProbe::connect(database_url)?));
        #[cfg(feature = "sqlite")]
        let readiness = ServiceReadiness::SharedStore;
        Ok(Self::from_parts_with_readiness(loaded, store, readiness))
    }

    fn from_parts(loaded: LoadedConfig, store: Store) -> Self {
        Self::from_parts_with_readiness(loaded, store, ServiceReadiness::SharedStore)
    }

    fn from_parts_with_readiness(
        loaded: LoadedConfig,
        store: Store,
        readiness: ServiceReadiness,
    ) -> Self {
        let usage = UsageCapture::new(loaded.hash.clone());
        Self {
            loaded,
            store: TrackedStoreMutex::new(store),
            usage,
            memory_consolidation_in_flight: AtomicBool::new(false),
            readiness,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_store(loaded: LoadedConfig, store: Store) -> Self {
        Self::from_parts(loaded, store)
    }

    fn user_prompt(&self, input: UserPromptInput, binding: &ProjectBinding) -> ServiceResponse {
        let result = (|| -> Result<PromptResponse> {
            let project = workspace::identify(binding, input.context.project_root.as_deref())?;
            let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
            store.touch_session(
                &project.id,
                &input.context.session_id,
                &input.context.agent,
                project.worktree.as_deref(),
                &self.loaded.hash,
            )?;
            let hash = blake3::hash(input.prompt.as_bytes()).to_hex().to_string();
            let synopsis = self.loaded.config.privacy.store_prompt_synopsis.then(|| {
                crate::coordination::domain::truncate_utf8(
                    synopsis(
                        &input.prompt,
                        self.loaded
                            .config
                            .privacy
                            .prompt_synopsis_max_chars
                            .min(crate::coordination::api::MAX_SESSION_SUMMARY_BYTES),
                    ),
                    crate::coordination::api::MAX_SESSION_SUMMARY_BYTES,
                )
            });
            let full = self
                .loaded
                .config
                .privacy
                .store_full_prompts
                .then_some(input.prompt.as_str());
            store.record_prompt(
                &project.id,
                &input.context.session_id,
                synopsis.as_deref(),
                &hash,
                full,
            )?;
            let memory_context = if self.loaded.config.memory.enabled {
                store.activate_memory_context(
                    &MemoryReadScope::layered(project.id.clone()),
                    &input.context.agent,
                    &input.context.session_id,
                    self.memory_context_limits(),
                )?
            } else {
                None
            };
            Ok(PromptResponse {
                permitted: true,
                recorded: true,
                project_id: project.id,
                prompt_hash: hash,
                memory_context,
            })
        })();
        match result {
            Ok(response) => ServiceResponse::Prompt(response),
            Err(error) => ServiceResponse::Hook(HookResponse::fail_open(error)),
        }
    }

    fn pre_tool_use(
        &self,
        input: ToolHookInput,
        binding: &ProjectBinding,
        evidence: Option<ClientDerivedUsage>,
    ) -> ServiceResponse {
        ServiceResponse::Hook(
            pre_tool_use_result(self, input, binding, evidence.as_ref())
                .unwrap_or_else(HookResponse::fail_open),
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
            let project = workspace::identify(binding, input.context.project_root.as_deref())?;
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
        ServiceResponse::Hook(result.unwrap_or_else(HookResponse::fail_open))
    }

    fn session_stop(&self, input: SessionStopInput, binding: &ProjectBinding) -> ServiceResponse {
        let result = (|| -> Result<HookResponse> {
            let project = workspace::identify(binding, input.context.project_root.as_deref())?;
            let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
            store.stop_session(&project.id, &input.context.session_id)?;
            Ok(HookResponse::allow(Vec::new()))
        })();
        ServiceResponse::Hook(result.unwrap_or_else(HookResponse::fail_open))
    }

    fn status(&self) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
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
        #[cfg(not(feature = "postgres"))]
        let _ = store_stall_threshold;
        match &self.readiness {
            #[cfg(feature = "postgres")]
            ServiceReadiness::Dedicated(probe) => {
                anyhow::ensure!(!self.store.is_poisoned(), "store lock poisoned");
                if self.store.held_for_at_least(store_stall_threshold) {
                    anyhow::bail!(
                        "application store stalled beyond the {}ms readiness threshold",
                        store_stall_threshold.as_millis()
                    );
                }
                let mut probe = probe
                    .try_lock()
                    .map_err(|error| anyhow::anyhow!("readiness probe unavailable: {error}"))?;
                probe.check()?;
                Ok("ready")
            }
            ServiceReadiness::SharedStore => match self.store.try_lock() {
                Ok(mut store) => {
                    store.counts(self.loaded.config.coordination.claim_ttl_seconds)?;
                    Ok("ready")
                }
                Err(TryLockError::WouldBlock) => Ok("busy"),
                Err(TryLockError::Poisoned(_)) => anyhow::bail!("store lock poisoned"),
            },
        }
    }

    fn events(&self, mut query: EventQuery, binding: &ProjectBinding) -> ServiceResponse {
        query.project_id = binding.default_read_project(query.project_id);
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| {
                store.list_events(query.project_id.as_deref(), query.bounded_limit())
            }) {
            Ok(events) => ServiceResponse::Events(EventsResponse { ok: true, events }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn resolve(&self, command: ResolveCommand) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
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
            let project = workspace::identify(binding, None)?;
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .release_claims(&project.id, &command.session_id, &release)
        })();
        match result {
            Ok(released) => ServiceResponse::Released(ReleasedResponse { ok: true, released }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn analyze(&self, command: AnalyzeCommand, binding: &ProjectBinding) -> ServiceResponse {
        match analyze_result(self, &command, binding) {
            Ok(response) => ServiceResponse::Analysis(response),
            Err(error) if matches!(binding, ProjectBinding::Remote { .. }) => {
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
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
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
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| {
                store.dashboard(
                    query.project_id.as_deref(),
                    self.loaded.config.ui.recent_event_limit,
                    self.loaded.config.ui.recent_record_limit,
                    self.loaded.config.coordination.claim_ttl_seconds,
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

    pub(super) fn lifecycle_store(&self) -> Result<TrackedStoreGuard<'_>> {
        match self
            .store
            .try_lock_for(LIFECYCLE_STORE_WAIT, LIFECYCLE_STORE_RETRY)
        {
            Ok(store) => Ok(store),
            Err(TryLockError::Poisoned(error)) => {
                Err(anyhow::anyhow!("lifecycle store unavailable: {error}"))
            }
            Err(TryLockError::WouldBlock) => Err(anyhow::anyhow!(
                "lifecycle store remained busy for {}ms",
                LIFECYCLE_STORE_WAIT.as_millis()
            )),
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
                self.lifecycle_store()
                    .context("acquire reconciliation store")?
                    .record_reconciliation(&project_id, &session_id, &intents, ttl_seconds)?;
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
        let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
        if ensure_session {
            store.ensure_session(
                project_id,
                &evidence.session_id,
                &evidence.agent,
                &self.loaded.hash,
            )?;
        }
        store.record_reconciliation(
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
    let project = workspace::identify(binding, input.context.project_root.as_deref())?;
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
    let intents = extract_path_intents(
        &input.tool_name,
        &input.tool_input,
        project.claim_root.as_deref(),
    );
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
            let item = advisory(classification, &intent.path, &existing.session_id);
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
    let trusted_worktree = match binding {
        ProjectBinding::Local => service
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .conflict_worktree(&conflict)?,
        ProjectBinding::Remote { .. } => None,
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
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
        .conflict_by_id(conflict_id)?
        .context("conflict not found")
}

fn record_analysis(
    service: &NexusService,
    conflict: &ConflictRecord,
    conflict_id: &str,
    analysis: &AnalysisResult,
) -> Result<()> {
    service
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
        .record_analysis(
            &conflict.project_id,
            conflict_id,
            analysis,
            &service.loaded.hash,
        )
}

#[cfg(test)]
mod project_isolation_tests;
#[cfg(test)]
mod store_contention_tests;
#[cfg(test)]
mod tests;
