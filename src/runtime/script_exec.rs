// @feature runtime
// @feature usage-analytics
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @entrypoint run
use super::daemon;
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::coordination::domain::{HookContext, ScriptOutcome, ScriptUseInput};
use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

pub async fn run(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    working_directory: &Path,
    command: Vec<OsString>,
) -> Result<i32> {
    let (program, arguments) = command
        .split_first()
        .context("nexus exec requires a program after `--`")?;
    let status = Command::new(program)
        .args(arguments)
        .current_dir(working_directory)
        .status()
        .with_context(|| format!("execute {}", program.to_string_lossy()))?;
    let outcome = if status.success() {
        ScriptOutcome::Succeeded
    } else if status.code().is_some() {
        ScriptOutcome::Failed
    } else {
        ScriptOutcome::Interrupted
    };
    let request = ServiceRequest::ScriptUse(ScriptUseInput {
        context: HookContext {
            session_id: format!("nexus-exec:{}", std::process::id()),
            project_root: Some(working_directory.to_string_lossy().into_owned()),
            agent: "nexus-exec".to_owned(),
            turn_id: None,
            model: None,
        },
        invocation_id: Uuid::new_v4().to_string(),
        script_name: script_identity(program, arguments, working_directory),
        outcome,
    });
    let _ = daemon::lifecycle_request(loaded, explicit_config, &request).await;
    Ok(exit_code(status))
}

fn script_identity(program: &OsStr, arguments: &[OsString], working_directory: &Path) -> String {
    let target = interpreter_target(program, arguments).unwrap_or(program);
    let target_path = Path::new(target);
    let resolved = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        working_directory.join(target_path)
    };
    let project_root = working_directory
        .canonicalize()
        .unwrap_or_else(|_| working_directory.to_path_buf());
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    if let Ok(relative) = resolved.strip_prefix(&project_root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    private_external_identity(&resolved)
}

fn interpreter_target<'a>(program: &OsStr, arguments: &'a [OsString]) -> Option<&'a OsStr> {
    let program = Path::new(program).file_name()?.to_str()?;
    let interpreter = matches!(
        program,
        "bash" | "sh" | "zsh" | "fish" | "node" | "ruby" | "perl"
    ) || program == "python"
        || program.strip_prefix("python").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
        });
    if !interpreter {
        return None;
    }
    arguments
        .iter()
        .find(|argument| !argument.to_string_lossy().starts_with('-'))
        .map(OsString::as_os_str)
}

fn private_external_identity(path: &Path) -> String {
    let basename = path.file_name().and_then(OsStr::to_str).unwrap_or("script");
    let text = path.to_string_lossy();
    let hash = blake3::hash(text.as_bytes()).to_hex();
    format!("{basename}#{}", &hash[..8])
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map_or(1, |signal| 128 + signal)
    }
    #[cfg(not(unix))]
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreter_identity_uses_the_script_operand_without_arguments() {
        let temporary = tempfile::tempdir().unwrap();
        let script = temporary.path().join("task.py");
        std::fs::write(&script, "print('ok')\n").unwrap();
        assert_eq!(
            script_identity(
                OsStr::new("python3"),
                &[script.as_os_str().to_owned(), OsString::from("--secret")],
                temporary.path(),
            ),
            "task.py"
        );
    }
}
