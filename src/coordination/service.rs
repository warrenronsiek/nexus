// @feature coordination
// @spec docs/features/coordination.md
// @entrypoint NexusService::handle
use super::api::{
    AnalysisResponse, AnalyzeCommand, ClaimQuery, ClaimsResponse, ConfigResponse, ConflictQuery,
    ConflictsResponse, DashboardQuery, DashboardResponse, EventQuery, EventsResponse,
    ProjectsResponse, PromptResponse, ReleaseCommand, ReleasedResponse, ResolveCommand,
    ResolvedResponse, ServiceRequest, ServiceResponse, SessionQuery, SessionsResponse,
    StatusResponse, ToolHookPhase,
};
use super::classifier::{advisory, extract_path_intents, strongest_overlaps};
use super::domain::{
    HookResponse, IgnoredAdvisoryPolicy, SessionStopInput, ToolCompletion, ToolHookInput,
    ToolResultEvent, UserPromptInput,
};
use super::workspace;
use crate::agents::analyst;
use crate::config::LoadedConfig;
use crate::persistence::Store;
use anyhow::{Context, Result};
use std::sync::Mutex;

pub struct NexusService {
    loaded: LoadedConfig,
    store: Mutex<Store>,
}

impl NexusService {
    pub fn new(loaded: LoadedConfig) -> Result<Self> {
        let store = Store::open(&loaded.config.storage.database_path)?;
        Ok(Self {
            loaded,
            store: Mutex::new(store),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_store(loaded: LoadedConfig, store: Store) -> Self {
        Self {
            loaded,
            store: Mutex::new(store),
        }
    }

    pub fn handle(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::UserPrompt(input) => self.user_prompt(input),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::Before,
                input,
            } => self.pre_tool_use(input),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::After(completion),
                input,
            } => self.post_tool_use(input, completion),
            ServiceRequest::SessionStop(input) => self.session_stop(input),
            ServiceRequest::Status => self.status(),
            ServiceRequest::Events(query) => self.events(query),
            ServiceRequest::Sessions(query) => self.sessions(query),
            ServiceRequest::Claims(query) => self.claims(query),
            ServiceRequest::Conflicts(query) => self.conflicts(query),
            ServiceRequest::Resolve(command) => self.resolve(command),
            ServiceRequest::Release(command) => self.release(command),
            ServiceRequest::Analyze(command) => self.analyze(command),
            ServiceRequest::Projects => self.projects(),
            ServiceRequest::Dashboard(query) => self.dashboard(query),
            ServiceRequest::Config => ServiceResponse::Config(Box::new(ConfigResponse {
                ok: true,
                config: self.loaded.config.clone(),
                sources: self.loaded.sources.clone(),
                hash: self.loaded.hash.clone(),
            })),
        }
    }

    fn user_prompt(&self, input: UserPromptInput) -> ServiceResponse {
        let result = (|| -> Result<PromptResponse> {
            let project = workspace::identify(input.context.project_root.as_deref())?;
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
                synopsis(
                    &input.prompt,
                    self.loaded.config.privacy.prompt_synopsis_max_chars,
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
            Ok(PromptResponse {
                permitted: true,
                recorded: true,
                project_id: project.id,
                prompt_hash: hash,
            })
        })();
        match result {
            Ok(response) => ServiceResponse::Prompt(response),
            Err(error) => ServiceResponse::Hook(HookResponse::fail_open(error)),
        }
    }

    fn pre_tool_use(&self, input: ToolHookInput) -> ServiceResponse {
        let result = (|| -> Result<HookResponse> {
            let project = workspace::identify(input.context.project_root.as_deref())?;
            let intents = extract_path_intents(
                &input.tool_name,
                &input.tool_input,
                project.worktree.as_deref(),
            );
            let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
            store.touch_session(
                &project.id,
                &input.context.session_id,
                &input.context.agent,
                project.worktree.as_deref(),
                &self.loaded.hash,
            )?;
            store.record_tool_inspection(
                &project.id,
                &input.context.session_id,
                &input.tool_use_id,
                &input.tool_name,
                &intents,
            )?;

            let mut advisories = Vec::new();
            for intent in intents {
                let active = store.active_claims_for_path(
                    &project.id,
                    &intent.path,
                    &input.context.session_id,
                )?;
                let claim = store.insert_claim(
                    &project.id,
                    &input.context.session_id,
                    &input.tool_use_id,
                    &intent,
                    self.loaded.config.coordination.claim_ttl_seconds,
                )?;
                for (existing, classification) in strongest_overlaps(&intent, active) {
                    let item = advisory(classification, &intent.path, &existing.session_id);
                    advisories.push(store.record_conflict(
                        &project.id,
                        &claim,
                        &existing,
                        item,
                        &input.tool_use_id,
                    )?);
                }
            }
            Ok(HookResponse::allow(advisories))
        })();
        ServiceResponse::Hook(result.unwrap_or_else(HookResponse::fail_open))
    }

    fn post_tool_use(&self, input: ToolHookInput, completion: ToolCompletion) -> ServiceResponse {
        let result = (|| -> Result<HookResponse> {
            let project = workspace::identify(input.context.project_root.as_deref())?;
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

    fn session_stop(&self, input: SessionStopInput) -> ServiceResponse {
        let result = (|| -> Result<HookResponse> {
            let project = workspace::identify(input.context.project_root.as_deref())?;
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
            .and_then(|mut store| store.counts())
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

    fn events(&self, query: EventQuery) -> ServiceResponse {
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

    fn claims(&self, query: ClaimQuery) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.list_claims(query.project_id.as_deref(), query.scope()))
        {
            Ok(claims) => ServiceResponse::Claims(ClaimsResponse { ok: true, claims }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn sessions(&self, query: SessionQuery) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.list_sessions(query.scope()))
        {
            Ok(sessions) => ServiceResponse::Sessions(SessionsResponse { ok: true, sessions }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn conflicts(&self, query: ConflictQuery) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.list_conflicts(query.project_id.as_deref(), query.scope()))
        {
            Ok(conflicts) => ServiceResponse::Conflicts(ConflictsResponse {
                ok: true,
                conflicts,
            }),
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

    fn release(&self, command: ReleaseCommand) -> ServiceResponse {
        let release = command.release();
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.release_claims(&command.session_id, &release))
        {
            Ok(released) => ServiceResponse::Released(ReleasedResponse { ok: true, released }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn analyze(&self, command: AnalyzeCommand) -> ServiceResponse {
        let result = (|| -> Result<AnalysisResponse> {
            let conflict = self
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .conflict_by_id(&command.conflict_id)?
                .context("conflict not found")?;
            let working_directory = std::path::Path::new(&conflict.path).parent();
            let analysis =
                analyst::analyze(&self.loaded.config.analyst, &conflict, working_directory)?;
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .record_analysis(
                    &conflict.project_id,
                    &command.conflict_id,
                    &analysis,
                    &self.loaded.hash,
                )?;
            Ok(AnalysisResponse { ok: true, analysis })
        })();
        match result {
            Ok(response) => ServiceResponse::Analysis(response),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn projects(&self) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| store.list_projects())
        {
            Ok(projects) => ServiceResponse::Projects(ProjectsResponse { ok: true, projects }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn dashboard(&self, query: DashboardQuery) -> ServiceResponse {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| {
                store.dashboard(
                    query.project_id.as_deref(),
                    self.loaded.config.ui.recent_event_limit,
                    self.loaded.config.ui.recent_record_limit,
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

    fn lifecycle_store(&self) -> Result<std::sync::MutexGuard<'_, Store>> {
        self.store
            .try_lock()
            .map_err(|error| anyhow::anyhow!("lifecycle store unavailable: {error}"))
    }

    pub(crate) fn reconcile_all(&self) -> Result<usize> {
        let ttl_seconds = self.loaded.config.coordination.claim_ttl_seconds;
        let sessions = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .reconcilable_sessions(ttl_seconds)?;
        let mut observed = 0;
        for session in sessions {
            let Some(worktree) = session.worktree.as_deref() else {
                continue;
            };
            let intents = workspace::changed_paths(worktree);
            observed += intents.len();
            self.store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .record_reconciliation(&session, &intents, ttl_seconds)?;
        }
        Ok(observed)
    }
}

fn synopsis(prompt: &str, max_chars: usize) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = compact
        .split_once(['.', '\n'])
        .map(|(head, _)| head)
        .unwrap_or(&compact);
    first.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;

    fn service() -> NexusService {
        service_with(Config::default())
    }

    fn service_with(config: Config) -> NexusService {
        let encoded = toml::to_string(&config).unwrap();
        NexusService::from_store(
            LoadedConfig {
                config,
                sources: vec![],
                hash: blake3::hash(encoded.as_bytes()).to_hex().to_string(),
            },
            Store::open_memory().unwrap(),
        )
    }

    fn edit(
        session_id: &str,
        tool_use_id: &str,
        path: &str,
        line_start: u32,
        line_end: u32,
    ) -> ServiceRequest {
        ServiceRequest::decode(
            "pre_tool_use",
            json!({
                "session_id": session_id,
                "agent": "test",
                "project_root": "/tmp",
                "tool_use_id": tool_use_id,
                "tool_name": "Edit",
                "tool_input": {
                    "file_path": path,
                    "line_start": line_start,
                    "line_end": line_end
                }
            }),
        )
        .unwrap()
    }

    #[test]
    fn second_session_gets_overlap_advisory() {
        let service = service();
        let one = json!({"session_id":"one","agent":"codex","project_root":"/tmp","tool_use_id":"t1","tool_name":"Edit","tool_input":{"file_path":"a.rs","line_start":10,"line_end":20}});
        let two = json!({"session_id":"two","agent":"claude","project_root":"/tmp","tool_use_id":"t2","tool_name":"Edit","tool_input":{"file_path":"a.rs","line_start":15,"line_end":18}});
        let one = ServiceRequest::decode("pre_tool_use", one).unwrap();
        let two = ServiceRequest::decode("pre_tool_use", two).unwrap();
        assert_eq!(
            service.handle(one).to_json()["advisories"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let response = service.handle(two).to_json();
        assert_eq!(response["permitted"], true, "{response}");
        assert_eq!(
            response["advisories"][0]["kind"], "hunk_overlap",
            "{response}"
        );
    }

    #[test]
    fn lifecycle_hooks_fail_open_without_waiting_for_the_store() {
        let service = std::sync::Arc::new(service());
        let store_guard = service.store.lock().unwrap();
        let request = edit("one", "t1", "a.rs", 1, 5);
        let worker_service = service.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(worker_service.handle(request).to_json())
                .unwrap();
        });

        let response = receiver.recv_timeout(std::time::Duration::from_millis(100));
        drop(store_guard);
        worker.join().unwrap();

        let response = response.expect("lifecycle hook waited for the busy store");
        assert_eq!(response["permitted"], true);
        assert_eq!(response["recorded"], false);
        assert!(response["diagnostic"].as_str().unwrap().contains("store"));
    }

    #[test]
    fn weaker_observations_do_not_downgrade_or_reopen_a_conflict() {
        let service = service();
        for (tool_use_id, line_start, line_end) in [("t1", 1, 5), ("t2", 10, 20), ("t3", 15, 25)] {
            service.handle(edit("one", tool_use_id, "a.rs", line_start, line_end));
        }

        let critical = service.handle(edit("two", "t4", "a.rs", 16, 18)).to_json();
        assert_eq!(critical["advisories"].as_array().unwrap().len(), 1);
        assert_eq!(critical["advisories"][0]["kind"], "hunk_overlap");
        let weaker = service
            .handle(edit("two", "t5", "a.rs", 100, 110))
            .to_json();
        assert_eq!(weaker["advisories"].as_array().unwrap().len(), 1);
        assert_eq!(weaker["advisories"][0]["severity"], "info");

        let current = service
            .handle(ServiceRequest::Conflicts(ConflictQuery::default()))
            .to_json();
        assert_eq!(current["conflicts"].as_array().unwrap().len(), 1);
        assert_eq!(current["conflicts"][0]["severity"], "critical");
        assert_eq!(current["conflicts"][0]["kind"], "hunk_overlap");
        let conflict_id = current["conflicts"][0]["id"].as_str().unwrap().to_owned();
        service.handle(ServiceRequest::Resolve(ResolveCommand {
            conflict_id,
            resolution: "coordinated".into(),
        }));
        service.handle(edit("two", "t6", "a.rs", 120, 130));
        let open = service
            .handle(ServiceRequest::Conflicts(ConflictQuery::default()))
            .to_json();
        assert!(open["conflicts"].as_array().unwrap().is_empty());
    }

    #[test]
    fn stronger_observation_reopens_a_resolved_conflict() {
        let service = service();
        service.handle(edit("one", "t7", "b.rs", 1, 5));
        service.handle(edit("two", "t8", "b.rs", 100, 110));
        let all = service
            .handle(ServiceRequest::Conflicts(ConflictQuery {
                project_id: None,
                scope: crate::coordination::domain::ConflictScope::All,
            }))
            .to_json();
        let minor_id = all["conflicts"][0]["id"].as_str().unwrap().to_owned();
        service.handle(ServiceRequest::Resolve(ResolveCommand {
            conflict_id: minor_id,
            resolution: "separate hunks".into(),
        }));
        service.handle(edit("two", "t9", "b.rs", 2, 4));
        let reopened = service
            .handle(ServiceRequest::Conflicts(ConflictQuery::default()))
            .to_json();
        assert_eq!(reopened["conflicts"].as_array().unwrap().len(), 1);
        assert_eq!(reopened["conflicts"][0]["severity"], "critical");
    }

    #[test]
    fn dashboard_is_scoped_and_bounded_while_projects_remain_global() {
        let temporary = tempfile::tempdir().unwrap();
        let first_root = temporary.path().join("first");
        let second_root = temporary.path().join("second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        let mut config = Config::default();
        config.ui.recent_event_limit = 2;
        config.ui.recent_record_limit = 1;
        let service = service_with(config);

        let prompt = |session_id: &str, root: &std::path::Path| {
            ServiceRequest::decode(
                "user_prompt",
                json!({
                    "session_id":session_id,
                    "agent":"test",
                    "project_root":root,
                    "prompt":"work on the dashboard"
                }),
            )
            .unwrap()
        };
        let first_project = service.handle(prompt("one", &first_root)).to_json()["project_id"]
            .as_str()
            .unwrap()
            .to_owned();
        service.handle(prompt("two", &first_root));
        service.handle(prompt("three", &second_root));

        let projects = service.handle(ServiceRequest::Projects).to_json();
        assert_eq!(projects["projects"].as_array().unwrap().len(), 2);
        assert_eq!(projects["projects"][0]["agents"][0], "test");

        let dashboard = service
            .handle(ServiceRequest::Dashboard(DashboardQuery {
                project_id: Some(first_project.clone()),
            }))
            .to_json();
        assert_eq!(dashboard["project_id"], first_project);
        assert_eq!(dashboard["counts"]["active_sessions"], 2);
        assert_eq!(dashboard["events"]["items"].as_array().unwrap().len(), 2);
        assert_eq!(dashboard["events"]["truncated"], true);
        assert_eq!(dashboard["sessions"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(dashboard["sessions"]["truncated"], true);
        assert_eq!(dashboard["claims"]["truncated"], false);
        assert_eq!(dashboard["conflicts"]["truncated"], false);
        assert_eq!(dashboard["refresh_interval_ms"], 2_000);
    }
}
