// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use super::api::{CaptureHealthResponse, ServiceResponse, UsageQuery, UsageResponse};
use super::domain::{
    HookResponse, ScriptOutcome, ScriptUseInput, SkillEvidence, SkillUseInput, ToolCompletion,
    ToolHookInput,
};
use super::service::NexusService;
use super::usage_detection::detect_script_invocations;
use super::workspace::{self, ProjectIdentity};
use crate::persistence::{
    CapabilityEvidence, CapabilityKind, CapabilityObservation, CapabilityOutcome, CapabilitySource,
    Store,
};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct UsageCapture {
    config_hash: String,
    received: AtomicU64,
    recorded: AtomicU64,
    dropped: AtomicU64,
}

impl UsageCapture {
    pub(super) fn new(config_hash: String) -> Self {
        Self {
            config_hash,
            received: AtomicU64::new(0),
            recorded: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    pub(super) fn observe_tool_start(
        &self,
        store: &mut Store,
        project_id: &str,
        input: &ToolHookInput,
    ) {
        let observation = self.tool_observation(project_id, input);
        let result = store.observe_capability_start(&observation);
        let _ = self.record(result);
    }

    pub(super) fn observe_tool_completion(
        &self,
        store: &mut Store,
        project: &ProjectIdentity,
        input: &ToolHookInput,
        completion: ToolCompletion,
    ) {
        let outcome = tool_outcome(completion);
        let tool = self.tool_observation(&project.id, input);
        let result = store.observe_capability_completion(&tool, outcome);
        let _ = self.record(result);
        self.observe_scripts(store, project, input, outcome);
        if completion == ToolCompletion::Succeeded {
            self.observe_manifest_skill(store, project, input);
        }
    }

    fn observe_scripts(
        &self,
        store: &mut Store,
        project: &ProjectIdentity,
        input: &ToolHookInput,
        outcome: CapabilityOutcome,
    ) {
        let Some(root) = project.worktree.as_deref() else {
            return;
        };
        let Some(command) = input.tool_input.first_string(&["cmd", "command"]) else {
            return;
        };
        for (index, script) in detect_script_invocations(command, Path::new(root))
            .into_iter()
            .enumerate()
        {
            let observation = self.script_observation(project, input, index, script.path);
            let result = store.observe_capability_completion(&observation, outcome);
            let _ = self.record(result);
        }
    }

    fn observe_manifest_skill(
        &self,
        store: &mut Store,
        project: &ProjectIdentity,
        input: &ToolHookInput,
    ) {
        let Some(root) = project.worktree.as_deref() else {
            return;
        };
        let Some(skill_name) = infer_skill_manifest(input, Path::new(root)) else {
            return;
        };
        let turn_id = clean_metadata(input.context.turn_id.as_deref());
        let turn_key = turn_id.as_deref().unwrap_or(&input.tool_use_id);
        let invocation_id = format!("skill:{turn_key}:{skill_name}");
        let observation = CapabilityObservation {
            project_id: project.id.clone(),
            agent: input.context.agent.clone(),
            session_id: input.context.session_id.clone(),
            turn_id,
            invocation_id,
            parent_invocation_id: Some(input.tool_use_id.clone()),
            kind: CapabilityKind::Skill,
            name: skill_name,
            source: source_for_agent(&input.context.agent),
            evidence: CapabilityEvidence::InstructionRead,
            config_hash: self.config_hash.clone(),
            model_id: clean_metadata(input.context.model.as_deref()),
        };
        let result = store.observe_capability_completion(&observation, CapabilityOutcome::Observed);
        let _ = self.record(result);
    }

    fn tool_observation(&self, project_id: &str, input: &ToolHookInput) -> CapabilityObservation {
        CapabilityObservation {
            project_id: project_id.to_owned(),
            agent: input.context.agent.clone(),
            session_id: input.context.session_id.clone(),
            turn_id: clean_metadata(input.context.turn_id.as_deref()),
            invocation_id: input.tool_use_id.clone(),
            parent_invocation_id: input.parent_tool_use_id.clone(),
            kind: CapabilityKind::Tool,
            name: canonical_tool_name(&input.tool_name),
            source: source_for_agent(&input.context.agent),
            evidence: CapabilityEvidence::NativeHook,
            config_hash: self.config_hash.clone(),
            model_id: clean_metadata(input.context.model.as_deref()),
        }
    }

    fn script_observation(
        &self,
        project: &ProjectIdentity,
        input: &ToolHookInput,
        index: usize,
        name: String,
    ) -> CapabilityObservation {
        CapabilityObservation {
            project_id: project.id.clone(),
            agent: input.context.agent.clone(),
            session_id: input.context.session_id.clone(),
            turn_id: clean_metadata(input.context.turn_id.as_deref()),
            invocation_id: format!("{}:script:{index}", input.tool_use_id),
            parent_invocation_id: Some(input.tool_use_id.clone()),
            kind: CapabilityKind::Script,
            name,
            source: CapabilitySource::ShellInference,
            evidence: CapabilityEvidence::ShellParsed,
            config_hash: self.config_hash.clone(),
            model_id: clean_metadata(input.context.model.as_deref()),
        }
    }

    fn record(&self, result: Result<()>) -> Result<()> {
        self.received.fetch_add(1, Ordering::Relaxed);
        match result {
            Ok(()) => {
                self.recorded.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(error) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }

    fn health(&self) -> CaptureHealthResponse {
        CaptureHealthResponse {
            label: "observed_by_nexus",
            received: self.received.load(Ordering::Relaxed),
            recorded: self.recorded.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }
}

impl NexusService {
    pub(super) fn skill_use(&self, input: SkillUseInput) -> ServiceResponse {
        let result = self.record_skill_use(input);
        ServiceResponse::Hook(result.unwrap_or_else(HookResponse::fail_open))
    }

    fn record_skill_use(&self, input: SkillUseInput) -> Result<HookResponse> {
        let project = workspace::identify(input.context.project_root.as_deref())?;
        let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
        self.touch_usage_session(&mut store, &project, &input.context)?;
        let observation = explicit_skill_observation(&self.usage, project.id, input);
        self.usage.record(
            store.observe_capability_completion(&observation, CapabilityOutcome::Observed),
        )?;
        Ok(HookResponse::allow(Vec::new()))
    }

    pub(super) fn script_use(&self, input: ScriptUseInput) -> ServiceResponse {
        let result = self.record_script_use(input);
        ServiceResponse::Hook(result.unwrap_or_else(HookResponse::fail_open))
    }

    fn record_script_use(&self, input: ScriptUseInput) -> Result<HookResponse> {
        let project = workspace::identify(input.context.project_root.as_deref())?;
        let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
        self.touch_usage_session(&mut store, &project, &input.context)?;
        let outcome = wrapped_outcome(input.outcome);
        let observation = wrapped_script_observation(&self.usage, project.id, input);
        self.usage
            .record(store.observe_capability_completion(&observation, outcome))?;
        Ok(HookResponse::allow(Vec::new()))
    }

    pub(super) fn usage(&self, query: UsageQuery) -> ServiceResponse {
        let window_ended_at = chrono::Utc::now();
        let window_started_at = window_ended_at - chrono::Duration::days(7);
        let summary = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
            .and_then(|mut store| {
                store.usage_summary(
                    query.project_id.as_deref(),
                    window_started_at,
                    window_ended_at,
                )
            });
        match summary {
            Ok(summary) => ServiceResponse::Usage(UsageResponse {
                ok: true,
                project_id: query.project_id,
                summary,
                capture_health: self.usage.health(),
            }),
            Err(error) => ServiceResponse::error(error),
        }
    }

    fn touch_usage_session(
        &self,
        store: &mut Store,
        project: &ProjectIdentity,
        context: &super::domain::HookContext,
    ) -> Result<()> {
        store.touch_session(
            &project.id,
            &context.session_id,
            &context.agent,
            project.worktree.as_deref(),
            &self.loaded.hash,
        )
    }
}

fn explicit_skill_observation(
    capture: &UsageCapture,
    project_id: String,
    input: SkillUseInput,
) -> CapabilityObservation {
    CapabilityObservation {
        project_id,
        agent: input.context.agent.clone(),
        session_id: input.context.session_id,
        turn_id: clean_metadata(input.context.turn_id.as_deref()),
        invocation_id: input.invocation_id,
        parent_invocation_id: None,
        kind: CapabilityKind::Skill,
        name: input.skill_name.trim().to_owned(),
        source: source_for_agent(&input.context.agent),
        evidence: skill_evidence(input.evidence),
        config_hash: capture.config_hash.clone(),
        model_id: clean_metadata(input.context.model.as_deref()),
    }
}

fn wrapped_script_observation(
    capture: &UsageCapture,
    project_id: String,
    input: ScriptUseInput,
) -> CapabilityObservation {
    CapabilityObservation {
        project_id,
        agent: input.context.agent,
        session_id: input.context.session_id,
        turn_id: clean_metadata(input.context.turn_id.as_deref()),
        invocation_id: input.invocation_id,
        parent_invocation_id: None,
        kind: CapabilityKind::Script,
        name: privacy_safe_script_name(&input.script_name),
        source: CapabilitySource::NexusExec,
        evidence: CapabilityEvidence::Wrapped,
        config_hash: capture.config_hash.clone(),
        model_id: clean_metadata(input.context.model.as_deref()),
    }
}

fn skill_evidence(evidence: SkillEvidence) -> CapabilityEvidence {
    match evidence {
        SkillEvidence::NativeHook => CapabilityEvidence::NativeHook,
        SkillEvidence::ExplicitInvocation => CapabilityEvidence::ExplicitInvocation,
        SkillEvidence::InstructionRead => CapabilityEvidence::InstructionRead,
        SkillEvidence::AssetExecution => CapabilityEvidence::AssetExecution,
    }
}

fn tool_outcome(completion: ToolCompletion) -> CapabilityOutcome {
    match completion {
        ToolCompletion::Succeeded => CapabilityOutcome::Succeeded,
        ToolCompletion::Failed => CapabilityOutcome::Failed,
    }
}

fn wrapped_outcome(outcome: ScriptOutcome) -> CapabilityOutcome {
    match outcome {
        ScriptOutcome::Succeeded => CapabilityOutcome::Succeeded,
        ScriptOutcome::Failed => CapabilityOutcome::Failed,
        ScriptOutcome::Interrupted => CapabilityOutcome::Interrupted,
    }
}

fn source_for_agent(agent: &str) -> CapabilitySource {
    if agent.eq_ignore_ascii_case("pi") {
        CapabilitySource::PiEvent
    } else {
        CapabilitySource::HostHook
    }
}

fn clean_metadata(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.starts_with("${"))
        .map(str::to_owned)
}

fn canonical_tool_name(name: &str) -> String {
    let leaf = name
        .rsplit([':', '.'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    match leaf.as_str() {
        "bash" | "shell" | "exec_command" => "shell".to_owned(),
        "read" | "read_file" => "read".to_owned(),
        "write" | "write_file" => "write".to_owned(),
        "edit" | "edit_file" => "edit".to_owned(),
        _ => name.trim().to_ascii_lowercase(),
    }
}

fn infer_skill_manifest(input: &ToolHookInput, project_root: &Path) -> Option<String> {
    let candidate = match canonical_tool_name(&input.tool_name).as_str() {
        "read" => {
            let path = input.tool_input.first_string(&["path", "file_path"])?;
            resolve_manifest(project_root, path)?
        }
        "shell" => shell_manifest_read(input, project_root)?,
        _ => return None,
    };
    skill_name_from_manifest(&candidate)
}

fn shell_manifest_read(input: &ToolHookInput, project_root: &Path) -> Option<std::path::PathBuf> {
    let command = input.tool_input.first_string(&["cmd", "command"])?;
    let words = shell_words::split(command).ok()?;
    let (program, arguments) = words.split_first()?;
    let program = executable_name(program)?;
    if !matches!(
        program,
        "cat" | "sed" | "head" | "tail" | "less" | "bat" | "grep" | "rg"
    ) {
        return None;
    }
    arguments
        .iter()
        .find_map(|argument| resolve_manifest(project_root, argument))
}

fn executable_name(program: &str) -> Option<&str> {
    Path::new(program).file_name()?.to_str()
}

fn resolve_manifest(project_root: &Path, path: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    };
    let candidate = candidate.canonicalize().ok()?;
    let skill_manifest = candidate.file_name()?.to_str()? == "SKILL.md";
    let within_skill_root = candidate
        .components()
        .any(|part| part.as_os_str() == "skills");
    (skill_manifest && within_skill_root).then_some(candidate)
}

fn skill_name_from_manifest(manifest: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(manifest).ok()?;
    let frontmatter = contents
        .lines()
        .skip(1)
        .take_while(|line| line.trim() != "---");
    frontmatter
        .filter_map(|line| line.trim().strip_prefix("name:"))
        .map(|name| name.trim().trim_matches(['\'', '"']).to_owned())
        .find(|name| !name.is_empty())
        .or_else(|| manifest.parent()?.file_name()?.to_str().map(str::to_owned))
}

fn privacy_safe_script_name(name: &str) -> String {
    let path = Path::new(name);
    let private = path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir));
    if !private {
        return name.trim_start_matches("./").replace('\\', "/");
    }
    let basename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("script");
    let hash = blake3::hash(name.as_bytes()).to_hex();
    format!("{basename}#{}", &hash[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skill_manifest_identity_prefers_frontmatter_name_over_directory_alias() {
        let temporary = tempfile::tempdir().unwrap();
        let manifest = temporary.path().join(".codex/skills/alias/SKILL.md");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(
            &manifest,
            "---\nname: canonical-skill\ndescription: Test.\n---\n",
        )
        .unwrap();
        let input = ToolHookInput {
            context: super::super::domain::HookContext {
                session_id: "skill-test".to_owned(),
                project_root: None,
                agent: "codex".to_owned(),
                turn_id: None,
                model: None,
            },
            tool_use_id: "read-1".to_owned(),
            tool_name: "read_file".to_owned(),
            parent_tool_use_id: None,
            tool_input: json!({"path":manifest}).into(),
            tool_output: None,
            error: None,
        };

        assert_eq!(
            infer_skill_manifest(&input, temporary.path()).as_deref(),
            Some("canonical-skill")
        );
    }

    #[test]
    fn successful_shell_manifest_reads_are_inferred_without_model_ceremony() {
        let temporary = tempfile::tempdir().unwrap();
        let manifest = temporary.path().join(".codex/skills/tdd/SKILL.md");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, "---\nname: tdd\n---\n").unwrap();
        let input = ToolHookInput {
            context: super::super::domain::HookContext {
                session_id: "shell-skill-test".to_owned(),
                project_root: None,
                agent: "codex".to_owned(),
                turn_id: None,
                model: None,
            },
            tool_use_id: "shell-read-1".to_owned(),
            tool_name: "exec_command".to_owned(),
            parent_tool_use_id: None,
            tool_input: json!({"cmd":format!("sed -n '1,240p' {}", manifest.display())}).into(),
            tool_output: None,
            error: None,
        };

        assert_eq!(
            infer_skill_manifest(&input, temporary.path()).as_deref(),
            Some("tdd")
        );
    }
}
