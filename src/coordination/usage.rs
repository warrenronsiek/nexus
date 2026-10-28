// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use super::api::{CaptureHealthResponse, ServiceResponse, UsageQuery, UsageResponse};
use super::client_evidence::ClientDerivedUsage;
use super::domain::{
    HookResponse, ScriptOutcome, ScriptUseInput, SkillEvidence, SkillUseInput, ToolCompletion,
    ToolHookInput,
};
use super::service::NexusService;
use super::usage_detection::detect_script_invocations;
use super::workspace::{self, ProjectBinding, ProjectIdentity};
use crate::persistence::{
    CapabilityEvidence, CapabilityKind, CapabilityObservation, CapabilityOutcome, CapabilitySource,
    Store,
};
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_SKILL_MANIFEST_BYTES: usize = 64 * 1024;

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
        client_evidence: Option<ClientDerivedUsage>,
    ) {
        let outcome = tool_outcome(completion);
        let tool = self.tool_observation(&project.id, input);
        let result = store.observe_capability_completion(&tool, outcome);
        let _ = self.record(result);
        let filesystem_evidence = client_evidence.or_else(|| {
            project
                .worktree
                .as_deref()
                .map(|root| derive_filesystem_evidence(input, completion, Path::new(root)))
        });
        if let Some(filesystem_evidence) = filesystem_evidence {
            self.observe_scripts(store, project, input, outcome, &filesystem_evidence.scripts);
            if let Some(skill) = filesystem_evidence.skill {
                self.observe_manifest_skill(store, project, input, skill);
            }
        }
    }

    fn observe_scripts(
        &self,
        store: &mut Store,
        project: &ProjectIdentity,
        input: &ToolHookInput,
        outcome: CapabilityOutcome,
        scripts: &[String],
    ) {
        for (index, script) in scripts.iter().enumerate() {
            let observation = self.script_observation(project, input, index, script.clone());
            let result = store.observe_capability_completion(&observation, outcome);
            let _ = self.record(result);
        }
    }

    fn observe_manifest_skill(
        &self,
        store: &mut Store,
        project: &ProjectIdentity,
        input: &ToolHookInput,
        skill_name: String,
    ) {
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

pub(crate) fn derive_filesystem_evidence(
    input: &ToolHookInput,
    completion: ToolCompletion,
    project_root: &Path,
) -> ClientDerivedUsage {
    let scripts: Vec<String> = input
        .tool_input
        .first_string(&["cmd", "command"])
        .map(|command| {
            detect_script_invocations(command, project_root)
                .into_iter()
                .map(|script| script.path)
                .collect()
        })
        .unwrap_or_default();
    let skill = (completion == ToolCompletion::Succeeded)
        .then(|| infer_skill_manifest(input, project_root))
        .flatten();
    ClientDerivedUsage {
        script_versions: scripts
            .iter()
            .filter_map(|name| {
                crate::analytics::capture::script_version(project_root, name)
                    .map(|version| (name.clone(), version))
            })
            .collect(),
        scripts,
        skill,
        repository_inventory: None,
    }
}

pub(super) enum ExplicitUsageInput {
    Skill(SkillUseInput),
    Script(ScriptUseInput),
}

impl From<SkillUseInput> for ExplicitUsageInput {
    fn from(input: SkillUseInput) -> Self {
        Self::Skill(input)
    }
}

impl From<ScriptUseInput> for ExplicitUsageInput {
    fn from(input: ScriptUseInput) -> Self {
        Self::Script(input)
    }
}

impl ExplicitUsageInput {
    fn context(&self) -> &super::domain::HookContext {
        match self {
            Self::Skill(input) => &input.context,
            Self::Script(input) => &input.context,
        }
    }

    fn into_observation(
        self,
        capture: &UsageCapture,
        project_id: String,
    ) -> (CapabilityObservation, CapabilityOutcome) {
        let (context, invocation_id, kind, name, source, evidence, outcome) = match self {
            Self::Skill(input) => {
                let source = source_for_agent(&input.context.agent);
                (
                    input.context,
                    input.invocation_id,
                    CapabilityKind::Skill,
                    input.skill_name.trim().to_owned(),
                    source,
                    skill_evidence(input.evidence),
                    CapabilityOutcome::Observed,
                )
            }
            Self::Script(input) => {
                let outcome = wrapped_outcome(input.outcome);
                (
                    input.context,
                    input.invocation_id,
                    CapabilityKind::Script,
                    privacy_safe_script_name(&input.script_name),
                    CapabilitySource::NexusExec,
                    CapabilityEvidence::Wrapped,
                    outcome,
                )
            }
        };
        let super::domain::HookContext {
            session_id,
            agent,
            turn_id,
            model,
            ..
        } = context;
        (
            CapabilityObservation {
                project_id,
                agent,
                session_id,
                turn_id: clean_metadata(turn_id.as_deref()),
                invocation_id,
                parent_invocation_id: None,
                kind,
                name,
                source,
                evidence,
                config_hash: capture.config_hash.clone(),
                model_id: clean_metadata(model.as_deref()),
            },
            outcome,
        )
    }
}

impl NexusService {
    pub(super) fn explicit_usage<I>(&self, input: I, binding: &ProjectBinding) -> ServiceResponse
    where
        I: Into<ExplicitUsageInput>,
    {
        let result = self.record_explicit_usage(input.into(), binding);
        ServiceResponse::Hook(result.unwrap_or_else(super::service::lifecycle_failure))
    }

    fn record_explicit_usage(
        &self,
        input: ExplicitUsageInput,
        binding: &ProjectBinding,
    ) -> Result<HookResponse> {
        let context = input.context();
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let mut store = self.lifecycle_store().context("acquire lifecycle store")?;
        self.touch_usage_session(&mut store, &project, context)?;
        let (observation, outcome) = input.into_observation(&self.usage, project.id);
        self.usage
            .record(store.observe_capability_completion(&observation, outcome))?;
        Ok(HookResponse::allow(Vec::new()))
    }

    pub(super) fn usage(&self, mut query: UsageQuery, binding: &ProjectBinding) -> ServiceResponse {
        query.project_id = binding.default_read_project(query.project_id);
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
    let metadata = std::fs::symlink_metadata(manifest).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SKILL_MANIFEST_BYTES as u64 {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(manifest)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SKILL_MANIFEST_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_SKILL_MANIFEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_SKILL_MANIFEST_BYTES {
        return None;
    }
    let contents = String::from_utf8(bytes).ok()?;
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

    enum ManifestRead {
        Direct,
        Shell,
    }

    fn assert_manifest_inference(
        read: ManifestRead,
        directory: &str,
        contents: &str,
        expected: &str,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let manifest = temporary
            .path()
            .join(format!(".codex/skills/{directory}/SKILL.md"));
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, contents).unwrap();
        let (tool_use_id, tool_name, tool_input) = match read {
            ManifestRead::Direct => ("read-1", "read_file", json!({"path": manifest}).into()),
            ManifestRead::Shell => (
                "shell-read-1",
                "exec_command",
                json!({"cmd": format!("sed -n '1,240p' {}", manifest.display())}).into(),
            ),
        };
        let input = ToolHookInput {
            client_path_intents: Vec::new(),
            is_interrupt: None,
            agent_id: None,
            context: super::super::domain::HookContext {
                session_id: "skill-test".to_owned(),
                project_root: None,
                agent: "codex".to_owned(),
                turn_id: None,
                model: None,
            },
            tool_use_id: tool_use_id.to_owned(),
            tool_name: tool_name.to_owned(),
            parent_tool_use_id: None,
            tool_input,
            tool_output: None,
            error: None,
        };

        assert_eq!(
            infer_skill_manifest(&input, temporary.path()).as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn skill_manifest_identity_prefers_frontmatter_name_over_directory_alias() {
        assert_manifest_inference(
            ManifestRead::Direct,
            "alias",
            "---\nname: canonical-skill\ndescription: Test.\n---\n",
            "canonical-skill",
        );
    }

    #[test]
    fn successful_shell_manifest_reads_are_inferred_without_model_ceremony() {
        assert_manifest_inference(ManifestRead::Shell, "tdd", "---\nname: tdd\n---\n", "tdd");
    }

    #[test]
    fn oversized_skill_manifests_are_ignored_without_reading_the_whole_file() {
        let temporary = tempfile::tempdir().unwrap();
        let manifest = temporary.path().join(".codex/skills/large/SKILL.md");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(
            &manifest,
            format!(
                "---\nname: should-not-be-observed\n---\n{}",
                "x".repeat(64 * 1024)
            ),
        )
        .unwrap();

        assert_eq!(skill_name_from_manifest(&manifest), None);
    }
}
