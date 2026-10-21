// @feature analyst
// @feature commit-review
// @feature agent-memory
// @spec docs/features/analyst.md
// @spec docs/features/commit-review.md
// @spec docs/features/agent-memory.md
use super::*;
use crate::config::Config;
use std::os::unix::process::CommandExt;

mod executable_support {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/executable.rs"
    ));
}

use executable_support::write_executable;

fn claude_model(command: &Path) -> ModelConfig {
    let mut model = Config::default().analyst.claude;
    model.command = command.to_string_lossy().into_owned();
    model
}

fn invoke_claude(
    model: &ModelConfig,
    prompt: &str,
    working_directory: Option<&Path>,
    timeout: Duration,
) -> Result<String> {
    Invocation {
        scope: InvocationScope::ReadOnly(working_directory),
        provider: ModelProvider::Claude,
        model,
        prompt,
        timeout,
    }
    .run()
}

#[test]
fn service_policy_can_disable_in_process_provider_execution() {
    assert!(ensure_in_process_provider_execution_allowed(false).is_ok());
    let error = ensure_in_process_provider_execution_allowed(true).unwrap_err();
    assert_eq!(
        error.to_string(),
        "in-process model providers are disabled for nexus-server"
    );
}

#[test]
fn absent_working_directory_runs_in_an_empty_isolated_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("print-working-directory");
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\ncount=$(find . -mindepth 1 -maxdepth 1 | wc -l | tr -d ' ')\nprintf '{\"result\":\"%s|%s\"}' \"$PWD\" \"$count\"\n",
    );

    let output = invoke_claude(
        &claude_model(&script),
        "prompt",
        None,
        Duration::from_secs(2),
    )
    .unwrap();
    let observation = extract_text(ModelProvider::Claude, &output);
    let (directory, count) = observation.rsplit_once('|').unwrap();

    assert_ne!(Path::new(&directory), std::env::current_dir().unwrap());
    assert_eq!(count, "0");
}

#[test]
fn oversized_stdout_is_rejected_at_the_bound() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("large-output");
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\nyes x | head -c 1048577\n",
    );

    let error = invoke_claude(
        &claude_model(&script),
        "prompt",
        None,
        Duration::from_secs(2),
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "claude model stdout exceeded the 1048576-byte output limit"
    );
}

#[test]
fn oversized_stderr_is_rejected_without_echoing_it() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("large-error");
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\nyes private-provider-detail | head -c 1048577 >&2\nexit 7\n",
    );

    let error = invoke_claude(
        &claude_model(&script),
        "prompt",
        None,
        Duration::from_secs(2),
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "claude model stderr exceeded the 1048576-byte output limit"
    );
    assert!(!error.to_string().contains("private-provider-detail"));
}

#[test]
fn provider_timeout_includes_prompt_delivery() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("does-not-read-stdin");
    write_executable(&script, "#!/bin/sh\nsleep 1\n");
    let prompt = "x".repeat(MODEL_PROMPT_LIMIT_BYTES);
    let started = Instant::now();

    let error = invoke_claude(
        &claude_model(&script),
        &prompt,
        None,
        Duration::from_millis(50),
    )
    .unwrap_err();

    assert!(is_timeout(&error), "{error:#}");
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn successful_provider_exit_does_not_hide_incomplete_prompt_delivery() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("exits-without-reading-stdin");
    write_executable(
        &script,
        "#!/bin/sh\nprintf '{\"result\":\"accepted\"}'\nexit 0\n",
    );
    let prompt = "x".repeat(MODEL_PROMPT_LIMIT_BYTES);

    let error = invoke_claude(
        &claude_model(&script),
        &prompt,
        None,
        Duration::from_secs(2),
    )
    .expect_err("a successful exit cannot accept a partially delivered prompt");

    assert!(
        error.to_string().contains("write model prompt"),
        "{error:#}"
    );
}

#[test]
fn late_stdin_failure_wins_over_a_successful_process_exit() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    command.process_group(0);
    let child = command.spawn().unwrap();
    let (signal_sender, signals) = mpsc::channel();
    let stdin = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let _ = signal_sender.send(ModelProcessSignal::StdinFailed(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "late prompt failure",
        )));
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "late prompt failure",
        ))
    });
    let empty_output = || {
        std::thread::spawn(|| {
            Ok(CapturedOutput {
                bytes: Vec::new(),
                exceeded: false,
            })
        })
    };
    let process = ModelProcess {
        child,
        stdin,
        stdout: empty_output(),
        stderr: empty_output(),
        signals,
        cancellation: Arc::new(AtomicBool::new(false)),
    };

    let error = process
        .finish(
            ModelProvider::Claude,
            Duration::from_secs(2),
            Instant::now() + Duration::from_secs(2),
        )
        .expect_err("prompt delivery failure must take precedence over successful exit");

    assert!(
        error.to_string().contains("write model prompt"),
        "{error:#}"
    );
}

#[test]
fn unrepresentable_provider_timeout_is_rejected_before_process_start() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("records-provider-start");
    let provider_marker = temporary.path().join("provider-started");
    write_executable(
        &script,
        "#!/bin/sh\n: > \"${0%/*}/provider-started\"\ncat >/dev/null\n",
    );

    let error = invoke_claude(&claude_model(&script), "prompt", None, Duration::MAX).unwrap_err();

    assert!(error.to_string().contains("monotonic clock range"));
    assert!(!provider_marker.exists());
}

#[test]
fn oversized_prompt_is_rejected_before_provider_start() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("records-provider-start");
    let provider_marker = temporary.path().join("provider-started");
    write_executable(
        &script,
        "#!/bin/sh\n: > \"${0%/*}/provider-started\"\ncat >/dev/null\n",
    );
    let prompt = "x".repeat(MODEL_PROMPT_LIMIT_BYTES + 1);

    let error = invoke_claude(
        &claude_model(&script),
        &prompt,
        None,
        Duration::from_secs(2),
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "claude model prompt exceeded the 1048576-byte input limit"
    );
    assert!(!provider_marker.exists());
}

#[test]
fn provider_exit_terminates_descendants_that_hold_capture_pipes() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("leaves-descendant");
    let descendant_marker = temporary.path().join("descendant-survived");
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\nmarker=\"${0%/*}/descendant-survived\"\n(sleep 0.1; : > \"$marker\"; sleep 2) &\nexit 0\n",
    );

    let output = invoke_claude(
        &claude_model(&script),
        "prompt",
        None,
        Duration::from_secs(2),
    )
    .unwrap();

    assert!(output.is_empty());
    std::thread::sleep(Duration::from_millis(500));
    assert!(!descendant_marker.exists());
}

#[test]
fn provider_runs_in_its_own_process_group() {
    let temporary = tempfile::tempdir().unwrap();
    let script = temporary.path().join("print-process-group");
    write_executable(
        &script,
        "#!/bin/sh\ncat >/dev/null\ngroup=$(/bin/ps -o pgid= -p $$ | tr -d ' ')\nprintf '{\"result\":\"%s|%s\"}' \"$$\" \"$group\"\n",
    );

    let output = invoke_claude(
        &claude_model(&script),
        "prompt",
        None,
        Duration::from_secs(2),
    )
    .unwrap();

    let identity = extract_text(ModelProvider::Claude, &output);
    let (process, group) = identity.split_once('|').unwrap();
    assert_eq!(process, group);
}
