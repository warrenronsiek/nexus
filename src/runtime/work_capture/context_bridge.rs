// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary exact-host-context-bridge: retain bounded identities for one exact project/agent/host-session, never command payloads or checkout/time heuristics.
use super::{
    directory, host_handoff,
    lifecycle::{event, stable_id},
    LoadedConfig, WorkCommand, WorkHandoff,
};
use crate::runtime::host_state::HostStateFile;
use crate::work_history::{WorkAction, WorkEvent, WorkObservationInput, WorkSubject};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    active: WorkHandoff,
    ended: bool,
    #[serde(default)]
    host_parent: Option<String>,
    #[serde(default)]
    last_title: Option<String>,
    #[serde(default)]
    title_revision: u64,
    #[serde(default)]
    request_bindings: VecDeque<(String, String)>,
}

pub fn active_host_context(
    loaded: &LoadedConfig,
    agent: &str,
    session: &str,
) -> Result<WorkHandoff> {
    let path = state_path(loaded, agent, session, None)?;
    let state = read_state(&path)?
        .context("no captured context for this exact project and host session")?;
    anyhow::ensure!(
        !state.ended,
        "host session ended; supply explicit caller context for independent work"
    );
    Ok(state.active)
}

pub(super) fn enrich(loaded: &LoadedConfig, command: &mut WorkCommand) {
    if update(loaded, command).is_err() {
        disclose_gap(command);
    }
}

fn disclose_gap(command: &mut WorkCommand) {
    let WorkAction::Record { observations } = &mut command.command else {
        return;
    };
    if !observations
        .iter()
        .any(|item| item.producer.name == "nexus-companion")
    {
        return;
    }
    let agent = crate::work_history::host_agent_identity(
        &command.context.agent,
        &command.context.session_id,
    );
    let mut context = super::lifecycle::context_from_hook(&command.context);
    context.labels.insert(
        "nexus.capture.gap".into(),
        "context_bridge_unavailable".into(),
    );
    let key = observations
        .first()
        .map(|item| item.id.as_str())
        .unwrap_or("empty");
    observations.push(event(
        WorkSubject::Activity { id: agent },
        &format!("context-gap-{key}"),
        WorkEvent::Milestone {
            name: "Exact host context unavailable".into(),
            context,
        },
    ));
}

fn host_source(item: &WorkObservationInput) -> bool {
    if item.producer.name != "nexus-companion" {
        return false;
    }
    matches!(&item.body,WorkEvent::ActivityDeclared {kind,..} if kind=="host.session")
        || matches!(&item.body,WorkEvent::Milestone {name,..} if name=="Host session title updated")
}

fn update(loaded: &LoadedConfig, command: &mut WorkCommand) -> Result<()> {
    let WorkAction::Record { observations } = &mut command.command else {
        return Ok(());
    };
    if !observations.iter().any(host_source) {
        return Ok(());
    }
    let hook = &command.context;
    let file = BridgeFile::open(loaded, hook)?;
    let previous = file.previous.clone();
    let mut state = previous.unwrap_or_else(|| State::new(hook));
    state.restore_shared_root(hook, observations);
    let missing = observations
        .iter_mut()
        .filter_map(|observation| state.observe(hook, observation))
        .count()
        > 0;
    if missing {
        disclose_evicted_request(hook, observations);
    }
    state.revise_title(observations);
    persist_state(&file.storage, &state, file.original.as_deref())
}

struct BridgeFile {
    storage: HostStateFile<State>,
    previous: Option<State>,
    original: Option<Vec<u8>>,
}
impl BridgeFile {
    fn open(
        loaded: &LoadedConfig,
        hook: &crate::coordination::domain::HookContext,
    ) -> Result<Self> {
        let path = state_path(
            loaded,
            &hook.agent,
            &hook.session_id,
            hook.project_root.as_deref(),
        )?;
        let storage = HostStateFile::open_locked(&path, MAX_BYTES)?;
        let previous = read_state(&path)?;
        let original = previous.as_ref().map(serde_json::to_vec).transpose()?;
        Ok(Self {
            storage,
            previous,
            original,
        })
    }
}

impl State {
    fn new(hook: &crate::coordination::domain::HookContext) -> Self {
        Self {
            active: host_handoff(&hook.agent, &hook.session_id, None, hook.model.as_deref()),
            ended: false,
            host_parent: None,
            last_title: None,
            title_revision: 0,
            request_bindings: VecDeque::new(),
        }
    }

    fn restore_shared_root(
        &self,
        hook: &crate::coordination::domain::HookContext,
        observations: &mut Vec<WorkObservationInput>,
    ) {
        let automatic = crate::work_history::host_work_identity(&hook.agent, &hook.session_id);
        if self.active.work_session_id == automatic {
            return;
        }
        let offered_default = observations.iter().any(|item| {
            item.subject
                == WorkSubject::Session {
                    id: automatic.clone(),
                }
                && matches!(item.body, WorkEvent::SessionOpened { .. })
        });
        if !offered_default {
            return;
        }
        observations.retain(|item| {
            item.subject
                != WorkSubject::Session {
                    id: automatic.clone(),
                }
        });
        for observation in observations {
            if let WorkEvent::ActivityDeclared {
                kind,
                work_session_id,
                parent_activity_id,
                ..
            } = &mut observation.body
            {
                if work_session_id.as_deref() == Some(automatic.as_str()) {
                    *work_session_id = Some(self.active.work_session_id.clone());
                }
                if kind == "host.session" {
                    *parent_activity_id = self.host_parent.clone();
                }
            }
        }
    }

    fn observe(
        &mut self,
        hook: &crate::coordination::domain::HookContext,
        observation: &mut WorkObservationInput,
    ) -> Option<()> {
        match &mut observation.body {
            WorkEvent::ActivityDeclared {
                kind,
                work_session_id,
                parent_activity_id,
                ..
            } if kind == "host.session" => {
                self.host_declaration(hook, work_session_id, parent_activity_id)
            }
            WorkEvent::ActivityDeclared { kind, .. } if kind == "request" => {
                self.active.request_activity_id = Some(observation.subject.id().into());
                if self.active.turn_id.is_none() {
                    self.active.parent_activity_id = self.active.request_activity_id.clone();
                }
            }
            WorkEvent::ActivityDeclared {
                kind,
                parent_activity_id,
                ..
            } if kind == "host.turn" => {
                self.turn_declaration(hook, observation.subject.id(), parent_activity_id)
            }
            WorkEvent::Finished { .. } => {
                if !self.restore_request_terminal(hook, &mut observation.subject) {
                    return Some(());
                }
                self.finish(hook, observation.subject.id());
            }
            _ => {}
        }
        None
    }

    fn host_declaration(
        &mut self,
        hook: &crate::coordination::domain::HookContext,
        root: &Option<String>,
        parent: &Option<String>,
    ) {
        if let Some(id) = root {
            self.active.work_session_id = id.clone();
        }
        self.host_parent = parent.clone();
        if self.active.turn_id.is_none() {
            self.active.parent_activity_id =
                self.active.request_activity_id.clone().or_else(|| {
                    Some(crate::work_history::host_agent_identity(
                        &hook.agent,
                        &hook.session_id,
                    ))
                });
        }
    }

    fn turn_declaration(
        &mut self,
        hook: &crate::coordination::domain::HookContext,
        id: &str,
        parent: &mut Option<String>,
    ) {
        let default_request = hook
            .turn_id
            .as_deref()
            .map(|turn| stable_id(&["request", &hook.agent, &hook.session_id, turn]));
        if *parent == default_request && self.active.request_activity_id.is_some() {
            *parent = self.active.request_activity_id.clone();
        }
        if let (Some(turn), Some(request)) = (&hook.turn_id, parent.as_ref()) {
            if !self.request_bindings.iter().any(|(known, _)| known == turn) {
                self.request_bindings
                    .push_back((turn.clone(), request.clone()));
                if self.request_bindings.len() > 16 {
                    self.request_bindings.pop_front();
                }
            }
        }
        self.active.turn_id = hook.turn_id.clone();
        self.active.model = hook.model.clone();
        self.active.parent_activity_id = Some(id.into());
        self.ended = false;
    }

    fn restore_request_terminal(
        &self,
        hook: &crate::coordination::domain::HookContext,
        subject: &mut WorkSubject,
    ) -> bool {
        let Some(turn) = hook.turn_id.as_deref() else {
            return true;
        };
        let fallback = stable_id(&["request", &hook.agent, &hook.session_id, turn]);
        if subject.id() == fallback {
            let known = self
                .request_bindings
                .iter()
                .find(|(known, _)| known == turn)
                .map(|(_, id)| id);
            let Some(id) = known else {
                return false;
            };
            *subject = WorkSubject::Activity { id: id.clone() };
        }
        true
    }

    fn finish(&mut self, hook: &crate::coordination::domain::HookContext, id: &str) {
        let host_id = crate::work_history::host_agent_identity(&hook.agent, &hook.session_id);
        if id == host_id {
            self.ended = true;
        }
        if self.active.request_activity_id.as_deref() == Some(id) {
            self.active.request_activity_id = None;
        }
        if self.active.parent_activity_id.as_deref() == Some(id) {
            self.active.turn_id = None;
            self.active.parent_activity_id =
                self.active.request_activity_id.clone().or(Some(host_id));
        }
    }

    fn revise_title(&mut self, observations: &mut Vec<WorkObservationInput>) {
        let incoming = observations
            .iter()
            .find_map(title_metadata)
            .map(|(title, revision)| (title.to_owned(), revision.map(str::to_owned)));
        let Some((title, source_revision)) = incoming else {
            return;
        };
        if source_revision.is_some() {
            self.last_title = Some(title);
            return;
        }
        if self.last_title.as_deref() == Some(&title) {
            observations.retain(|item| title_metadata(item).is_none());
            return;
        }
        self.title_revision = self.title_revision.saturating_add(1);
        self.last_title = Some(title);
        for observation in observations
            .iter_mut()
            .filter(|item| title_metadata(item).is_some())
        {
            observation.id = stable_id(&[
                observation.subject.id(),
                "host-title-revision",
                &self.title_revision.to_string(),
            ]);
        }
    }
}

fn disclose_evicted_request(
    hook: &crate::coordination::domain::HookContext,
    observations: &mut Vec<WorkObservationInput>,
) {
    let mut context = super::lifecycle::context_from_hook(hook);
    context
        .labels
        .insert("nexus.capture.gap".into(), "request_context_evicted".into());
    let key = hook.turn_id.as_deref().unwrap_or("missing");
    observations.push(event(
        WorkSubject::Activity {
            id: crate::work_history::host_agent_identity(&hook.agent, &hook.session_id),
        },
        &format!("request-context-missing-{key}"),
        WorkEvent::Milestone {
            name: "Host lifecycle coverage incomplete".into(),
            context,
        },
    ));
}

fn title_metadata(observation: &WorkObservationInput) -> Option<(&str, Option<&str>)> {
    let WorkEvent::Milestone { name, context } = &observation.body else {
        return None;
    };
    if name != "Host session title updated" {
        return None;
    }
    Some((
        context.labels.get("nexus.session_title")?.as_str(),
        context
            .labels
            .get("nexus.title.source_revision")
            .map(String::as_str),
    ))
}

fn persist_state(
    storage: &HostStateFile<State>,
    state: &State,
    original: Option<&[u8]>,
) -> Result<()> {
    let bytes = serde_json::to_vec(state)?;
    if original == Some(bytes.as_slice()) {
        return Ok(());
    }
    storage.publish(state)
}

fn state_path(
    loaded: &LoadedConfig,
    agent: &str,
    session: &str,
    root: Option<&str>,
) -> Result<PathBuf> {
    for value in [agent, session] {
        anyhow::ensure!(
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control),
            "invalid host context identity"
        );
    }
    let project = super::project_id(loaded, root)?;
    Ok(directory(loaded)?
        .join("contexts")
        .join(format!("{}.json", stable_id(&[&project, agent, session]))))
}

fn read_state(path: &Path) -> Result<Option<State>> {
    HostStateFile::<State>::read(path, MAX_BYTES)?
        .map(|state| {
            WorkHandoff::decode(&serde_json::to_vec(&state.active)?)?;
            Ok(state)
        })
        .transpose()
}
