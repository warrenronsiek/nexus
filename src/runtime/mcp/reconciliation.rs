// @feature runtime
// @feature coordination
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @boundary configured-service-invoker
use super::ServiceInvoker;
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::runtime::client::NexusClient;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(super) const MAX_ACTIVE_CLIENT_SESSIONS: usize = 1_024;

pub(super) struct ConfiguredInvoker {
    pub(super) client: NexusClient,
    pub(super) active_sessions: Mutex<HashMap<String, ActiveClientSession>>,
    pub(super) claim_ttl: Duration,
}

#[derive(Clone)]
pub(super) struct ActiveClientSession {
    session_id: String,
    agent: String,
    project_root: String,
    pub(super) last_seen: Instant,
}

impl ConfiguredInvoker {
    pub(super) fn from_loaded(loaded: Arc<LoadedConfig>, explicit_config: Option<PathBuf>) -> Self {
        Self {
            client: NexusClient::from_shared(loaded.clone(), explicit_config),
            active_sessions: Mutex::new(HashMap::new()),
            claim_ttl: Duration::from_secs(loaded.config.coordination.claim_ttl_seconds as u64),
        }
    }

    pub(super) fn observe_session(&self, request: &ServiceRequest) {
        if let ServiceRequest::SessionStop(input) = request {
            if let Ok(mut sessions) = self.active_sessions.lock() {
                sessions.remove(&input.context.session_id);
            }
            return;
        }
        let context = match request {
            ServiceRequest::UserPrompt(input) => Some(&input.context),
            ServiceRequest::ToolHook { input, .. } => Some(&input.context),
            ServiceRequest::SkillUse(input) => Some(&input.context),
            ServiceRequest::ScriptUse(input) => Some(&input.context),
            _ => None,
        };
        let Some((context, project_root)) =
            context.and_then(|context| context.project_root.as_ref().map(|root| (context, root)))
        else {
            return;
        };
        if let Ok(mut sessions) = self.active_sessions.lock() {
            prune_active_client_sessions(&mut sessions, self.claim_ttl);
            if !sessions.contains_key(&context.session_id)
                && sessions.len() >= MAX_ACTIVE_CLIENT_SESSIONS
            {
                evict_oldest_active_client_session(&mut sessions);
            }
            sessions.insert(
                context.session_id.clone(),
                ActiveClientSession {
                    session_id: context.session_id.clone(),
                    agent: context.agent.clone(),
                    project_root: project_root.clone(),
                    last_seen: Instant::now(),
                },
            );
        }
    }

    pub(super) fn sessions_by_root(&self) -> HashMap<String, Vec<ActiveClientSession>> {
        let Ok(mut sessions) = self.active_sessions.lock() else {
            return HashMap::new();
        };
        prune_active_client_sessions(&mut sessions, self.claim_ttl);
        let mut by_root: HashMap<String, Vec<ActiveClientSession>> = HashMap::new();
        for session in sessions.values() {
            by_root
                .entry(session.project_root.clone())
                .or_default()
                .push(session.clone());
        }
        by_root
    }
}

#[async_trait]
impl ServiceInvoker for ConfiguredInvoker {
    async fn invoke(&self, request: ServiceRequest, lifecycle: bool) -> Result<Value> {
        if lifecycle {
            self.observe_session(&request);
            Ok(self.client.lifecycle_request(&request).await)
        } else {
            self.client.ensure_and_request(&request).await
        }
    }
}

fn prune_active_client_sessions(
    sessions: &mut HashMap<String, ActiveClientSession>,
    claim_ttl: Duration,
) {
    let now = Instant::now();
    sessions.retain(|_, session| now.saturating_duration_since(session.last_seen) <= claim_ttl);
}

fn evict_oldest_active_client_session(sessions: &mut HashMap<String, ActiveClientSession>) {
    if let Some(oldest) = sessions
        .iter()
        .min_by_key(|(_, session)| session.last_seen)
        .map(|(session_id, _)| session_id.clone())
    {
        sessions.remove(&oldest);
    }
}

pub(super) fn spawn_remote_reconciler(
    seconds: u64,
    invoker: Arc<ConfiguredInvoker>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !invoker.client.is_remote() {
        return None;
    }
    Some(tokio::spawn(async move {
        let period = Duration::from_secs(seconds);
        let start = tokio::time::Instant::now() + period;
        let mut interval = tokio::time::interval_at(start, period);
        let mut path_offsets = HashMap::<String, usize>::new();
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let sessions_by_root = invoker.sessions_by_root();
            path_offsets.retain(|root, _| sessions_by_root.contains_key(root));
            for (project_root, sessions) in sessions_by_root {
                let offset = path_offsets.get(&project_root).copied().unwrap_or_default();
                let root = project_root.clone();
                let page = tokio::task::spawn_blocking(move || {
                    crate::coordination::client_evidence::capture_changed_paths_page(&root, offset)
                })
                .await
                .unwrap_or_default();
                if page.truncated {
                    path_offsets.insert(project_root, page.next_offset);
                } else {
                    path_offsets.remove(&project_root);
                }
                for session in sessions {
                    let evidence = crate::coordination::client_evidence::ClientFilesystemEvidence::reconciliation(
                        session.session_id,
                        session.agent,
                        page.paths.clone(),
                        page.truncated,
                    );
                    let _ = tokio::time::timeout(
                        crate::runtime::client::LIFECYCLE_RESPONSE_BUDGET,
                        invoker.client.submit_reconciliation(evidence),
                    )
                    .await;
                }
            }
        }
    }))
}
