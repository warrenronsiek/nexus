// @feature runtime
// @feature usage-analytics
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/work-history.md
// @entrypoint run
use super::client::NexusClient;
use super::work_capture::{self, ActivityCapture, CaptureContext};
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::coordination::domain::{ScriptOutcome, ScriptUseInput};
use crate::work_history::WorkOutcome;
use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct ExecOptions {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub work_session_id: Option<String>,
    pub parent_activity_id: Option<String>,
}

pub async fn run(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    working_directory: &Path,
    command: Vec<OsString>,
) -> Result<i32> {
    run_with_options(
        loaded,
        explicit_config,
        working_directory,
        command,
        ExecOptions::default(),
    )
    .await
}

pub async fn run_with_options(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    working_directory: &Path,
    command: Vec<OsString>,
    options: ExecOptions,
) -> Result<i32> {
    let (program, arguments) = command
        .split_first()
        .context("nexus exec requires a program after `--`")?;
    let mut invocation =
        ObservedCommand::prepare(loaded, working_directory, program, arguments, options);
    if let Some(path) = explicit_config {
        invocation.process.env(
            "NEXUS_CONFIG",
            path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
        );
    }
    let execution = invocation.execute();
    let result = match execution {
        Ok((status, request)) => {
            let _ = NexusClient::new(loaded, explicit_config)
                .lifecycle_request(&request)
                .await;
            Ok(exit_code(status))
        }
        Err(error) => Err(error),
    };
    let _ = work_capture::flush(loaded, explicit_config).await;
    result
}

struct ObservedCommand {
    process: Command,
    context: crate::coordination::domain::HookContext,
    identity: String,
    capture: ActivityCapture,
}

impl ObservedCommand {
    fn prepare(
        loaded: &LoadedConfig,
        root: &Path,
        program: &OsStr,
        arguments: &[OsString],
        options: ExecOptions,
    ) -> Self {
        let mut context = CaptureContext::from_environment(root);
        context.work_session_id = options.work_session_id.or(context.work_session_id);
        context.parent_activity_id = options.parent_activity_id.or(context.parent_activity_id);
        let identity = script_identity(program, arguments, root);
        let commit_attempt = options.kind.as_deref() == Some("commit.attempt")
            || (Path::new(program)
                .file_name()
                .is_some_and(|name| name == "git")
                && arguments
                    .first()
                    .is_some_and(|argument| argument == "commit"));
        let capture = ActivityCapture::begin(
            loaded,
            context.clone(),
            options.kind.as_deref().unwrap_or(if commit_attempt {
                "commit.attempt"
            } else {
                "command"
            }),
            options.name.as_deref().unwrap_or(&identity),
        );
        let mut process = Command::new(program);
        process.args(arguments).current_dir(root);
        capture.child_context().apply(&mut process);
        if commit_attempt {
            process.env("NEXUS_COMMIT_OPERATION_ID", &capture.id);
        }
        Self {
            process,
            context: context.hook,
            identity,
            capture,
        }
    }

    fn execute(mut self) -> Result<(std::process::ExitStatus, ServiceRequest)> {
        let status = match self.process.status() {
            Ok(status) => status,
            Err(error) => {
                self.capture
                    .finish(WorkOutcome::InfrastructureError, None, None, None);
                return Err(error).context("execute wrapped command");
            }
        };
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(not(unix))]
        let signal = None;
        let (outcome, legacy) = match (status.success(), signal) {
            (true, _) => (WorkOutcome::Succeeded, ScriptOutcome::Succeeded),
            (_, Some(_)) => (WorkOutcome::Canceled, ScriptOutcome::Interrupted),
            _ => (WorkOutcome::Failed, ScriptOutcome::Failed),
        };
        self.capture.finish(outcome, status.code(), signal, None);
        let request = ServiceRequest::ScriptUse(ScriptUseInput {
            context: self.context,
            invocation_id: Uuid::new_v4().to_string(),
            script_name: self.identity,
            outcome: legacy,
        });
        Ok((status, request))
    }
}

fn script_identity(program: &OsStr, arguments: &[OsString], working_directory: &Path) -> String {
    let target = interpreter_target(program, arguments)
        .filter(|path| working_directory.join(path).is_file())
        .unwrap_or(program);
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
    let flags = interpreter_file_flags(program)?;
    interpreter_file_operand(arguments, flags)
}

fn interpreter_file_operand<'a>(arguments: &'a [OsString], flags: &[&str]) -> Option<&'a OsStr> {
    let mut operands = arguments.iter();
    while let Some(operand) = operands.next() {
        let value = operand.to_str()?;
        if value == "--" {
            return operands.next().map(OsString::as_os_str);
        }
        if !value.starts_with('-') {
            return Some(operand.as_os_str());
        }
        if !flags.contains(&value) {
            return None;
        }
    }
    None
}

fn interpreter_file_flags(program: &str) -> Option<&'static [&'static str]> {
    match program {
        "bash" | "sh" | "zsh" | "fish" => Some(&["-e", "-u", "-eu", "-ue", "-f"]),
        "node" | "ruby" | "perl" => Some(&[]),
        name if name == "python"
            || name.strip_prefix("python").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == '.')
            }) =>
        {
            Some(&["-u", "-B", "-E", "-I", "-s", "-S", "-O", "-OO"])
        }
        _ => None,
    }
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
