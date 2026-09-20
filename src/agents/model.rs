// @feature analyst
// @feature commit-review
// @spec docs/features/analyst.md
// @spec docs/features/commit-review.md
// @boundary dynamic-model-output
use crate::config::{ModelConfig, ModelProvider};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

pub(crate) fn invoke(
    provider: ModelProvider,
    model: &ModelConfig,
    prompt: &str,
    working_directory: Option<&Path>,
    timeout: Duration,
) -> Result<String> {
    let mut command = Command::new(&model.command);
    match provider {
        ModelProvider::Codex => {
            command.args([
                "exec",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--ephemeral",
                "--json",
            ]);
            if let Some(model) = &model.model {
                command.args(["--model", model]);
            }
            if let Some(effort) = &model.reasoning_effort {
                command
                    .arg("-c")
                    .arg(format!("model_reasoning_effort={effort:?}"));
            }
            command.arg("-");
        }
        ModelProvider::Claude => {
            command.args([
                "-p",
                "--output-format",
                "json",
                "--permission-mode",
                "plan",
                "--no-session-persistence",
            ]);
            if let Some(model) = &model.model {
                command.args(["--model", model]);
            }
            if let Some(effort) = &model.reasoning_effort {
                command.args(["--effort", effort]);
            }
        }
    }
    if let Some(directory) = working_directory.filter(|path| path.exists()) {
        command.current_dir(directory);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "start {} model command {}",
                provider_name(provider),
                model.command
            )
        })?;
    child
        .stdin
        .take()
        .context("model stdin unavailable")?
        .write_all(prompt.as_bytes())?;
    let mut stdout = child.stdout.take().context("model stdout unavailable")?;
    let mut stderr = child.stderr.take().context("model stderr unavailable")?;
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = match child.wait_timeout(timeout)? {
        Some(status) => status,
        None => {
            child.kill()?;
            child.wait()?;
            bail!(
                "{} model timed out after {} seconds",
                provider_name(provider),
                timeout.as_secs()
            )
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("model stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("model stderr reader panicked"))??;
    if !status.success() {
        bail!(
            "{} model exited with {}: {}",
            provider_name(provider),
            status,
            failure_detail(provider, &stdout, &stderr)
        )
    }
    String::from_utf8(stdout).context("model output was not UTF-8")
}

fn failure_detail(provider: ModelProvider, stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if stderr.trim().is_empty() {
        extract_text(provider, &String::from_utf8_lossy(stdout))
    } else {
        stderr.trim().to_owned()
    }
}

pub(crate) fn extract_text(provider: ModelProvider, raw: &str) -> String {
    match provider {
        ModelProvider::Claude => serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|value| {
                value
                    .get("result")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| raw.trim().to_owned()),
        ModelProvider::Codex => raw
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|value| {
                value
                    .pointer("/item/text")
                    .or_else(|| value.get("text"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .next_back()
            .unwrap_or_else(|| raw.trim().to_owned()),
    }
}

pub(crate) fn provider_name(provider: ModelProvider) -> &'static str {
    match provider {
        ModelProvider::Codex => "codex",
        ModelProvider::Claude => "claude",
    }
}
