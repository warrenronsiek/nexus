// @feature analyst
// @spec docs/features/analyst.md
// @entrypoint analyze
// @boundary dynamic-json
use super::model;
use crate::config::{AnalystConfig, ModelProvider};
use crate::coordination::domain::ConflictRecord;
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub provider: ModelProvider,
    pub model: Option<String>,
    pub conflict_id: String,
    pub text: String,
    pub raw_output: String,
    pub generated_at: DateTime<Utc>,
}

pub fn analyze(
    config: &AnalystConfig,
    conflict: &ConflictRecord,
    working_directory: Option<&Path>,
) -> Result<AnalysisResult> {
    if !config.enabled {
        bail!("analyst is disabled; set analyst.enabled = true")
    }
    let (provider, model) = match config.provider {
        ModelProvider::Codex => (ModelProvider::Codex, &config.codex),
        ModelProvider::Claude => (ModelProvider::Claude, &config.claude),
    };
    let prompt = analyst_prompt(conflict);
    let raw_output = model::invoke(
        provider,
        model,
        &prompt,
        working_directory,
        Duration::from_secs(config.timeout_seconds),
    )?;
    let text = model::extract_text(provider, &raw_output);
    Ok(AnalysisResult {
        provider,
        model: model.model.clone(),
        conflict_id: conflict.id.clone(),
        text,
        raw_output,
        generated_at: Utc::now(),
    })
}

fn analyst_prompt(conflict: &ConflictRecord) -> String {
    format!(
        "You are a read-only coordination analyst. Do not edit files, run mutating commands, or decide whether an agent may proceed. Summarize this detected overlap and suggest up to three optional resolutions (split scope, sequence, handoff, accept overlap, request review, or release). Clearly state uncertainty. Return concise JSON with keys summary, rationale, and suggested_resolutions.\n\nConflict:\n{}",
        serde_json::to_string_pretty(conflict).unwrap_or_else(|_| format!("{conflict:?}"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::coordination::domain::{ConflictStatus, Severity};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn configured_provider_is_invoked_and_provenance_is_returned() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-claude");
        std::fs::write(
            &script,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"result\":\"analysis text\"}'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        let mut config = Config::default().analyst;
        config.enabled = true;
        config.provider = ModelProvider::Claude;
        config.claude.command = script.to_string_lossy().into_owned();
        config.claude.model = Some("chosen-model".into());
        let now = Utc::now();
        let conflict = ConflictRecord {
            id: "conflict-1".into(),
            project_id: "project-1".into(),
            left_claim_id: "left".into(),
            right_claim_id: "right".into(),
            path: "a.rs".into(),
            severity: Severity::Warning,
            kind: "path_overlap".into(),
            status: ConflictStatus::Open,
            message: "overlap".into(),
            created_at: now,
            updated_at: now,
        };
        let result = analyze(&config, &conflict, Some(temp.path())).unwrap();
        assert_eq!(result.provider, ModelProvider::Claude);
        assert_eq!(result.model.as_deref(), Some("chosen-model"));
        assert_eq!(result.text, "analysis text");
    }
}
