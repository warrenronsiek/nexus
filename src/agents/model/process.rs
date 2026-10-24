// @feature work-history
// @spec docs/features/work-history.md
// @feature usage-analytics
// @spec docs/features/usage-analytics.md
// @feature analyst
// @feature commit-review
// @feature agent-memory
// @spec docs/features/analyst.md
// @spec docs/features/commit-review.md
// @spec docs/features/agent-memory.md
// @boundary bounded-model-process
use super::{extract_text, provider, provider_name};
use crate::config::{ModelConfig, ModelProvider};
use crate::process_capture::{self, CapturedOutput, OutputReader};
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MODEL_PROMPT_LIMIT_BYTES: usize = 1_048_576;
const MODEL_OUTPUT_LIMIT_BYTES: usize = 1_048_576;
const MODEL_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(10);
static IN_PROCESS_PROVIDER_EXECUTION_DISABLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub(super) struct ModelTimeout {
    pub(super) provider: ModelProvider,
    pub(super) seconds: u64,
}

impl std::fmt::Display for ModelTimeout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} model timed out after {} seconds",
            provider_name(self.provider),
            self.seconds
        )
    }
}

impl std::error::Error for ModelTimeout {}

#[derive(Debug)]
struct ModelPromptLimit {
    provider: ModelProvider,
}

impl std::fmt::Display for ModelPromptLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} model prompt exceeded the {}-byte input limit",
            provider_name(self.provider),
            MODEL_PROMPT_LIMIT_BYTES
        )
    }
}

impl std::error::Error for ModelPromptLimit {}

#[derive(Debug)]
struct ModelOutputLimit {
    provider: ModelProvider,
    stream: &'static str,
}

impl std::fmt::Display for ModelOutputLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} model {} exceeded the {}-byte output limit",
            provider_name(self.provider),
            self.stream,
            MODEL_OUTPUT_LIMIT_BYTES
        )
    }
}

impl std::error::Error for ModelOutputLimit {}

pub(crate) fn is_timeout(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<ModelTimeout>().is_some())
}

pub(crate) fn disable_in_process_provider_execution() {
    IN_PROCESS_PROVIDER_EXECUTION_DISABLED.store(true, Ordering::Release);
}

fn ensure_in_process_provider_execution_allowed(disabled: bool) -> Result<()> {
    if disabled {
        bail!("in-process model providers are disabled for nexus-server");
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum InvocationScope<'a> {
    ReadOnly {
        directory: Option<&'a Path>,
        capture: Option<&'a crate::runtime::work_capture::CaptureContext>,
    },
    Classification,
}
impl<'a> InvocationScope<'a> {
    fn working_directory(self) -> Option<&'a Path> {
        match self {
            Self::ReadOnly { directory, .. } => directory,
            Self::Classification => None,
        }
    }
}

impl<'a> From<Option<&'a Path>> for InvocationScope<'a> {
    fn from(directory: Option<&'a Path>) -> Self {
        Self::ReadOnly {
            directory,
            capture: None,
        }
    }
}
impl<'a> From<&'a crate::runtime::work_capture::CaptureContext> for InvocationScope<'a> {
    fn from(capture: &'a crate::runtime::work_capture::CaptureContext) -> Self {
        Self::ReadOnly {
            directory: capture.hook.project_root.as_deref().map(Path::new),
            capture: Some(capture),
        }
    }
}

pub(super) struct Invocation<'a> {
    pub(super) scope: InvocationScope<'a>,
    pub(super) provider: ModelProvider,
    pub(super) model: &'a ModelConfig,
    pub(super) prompt: &'a str,
    pub(super) timeout: Duration,
}

impl Invocation<'_> {
    pub(super) fn run(self) -> Result<String> {
        ensure_in_process_provider_execution_allowed(
            IN_PROCESS_PROVIDER_EXECUTION_DISABLED.load(Ordering::Acquire),
        )?;
        if self.prompt.len() > MODEL_PROMPT_LIMIT_BYTES {
            return Err(ModelPromptLimit {
                provider: self.provider,
            }
            .into());
        }
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .context("model timeout exceeds the supported monotonic clock range")?;
        let mut command = provider::command(self.provider, self.model);
        if matches!(self.scope, InvocationScope::Classification) {
            provider::restrict_classification(&mut command, self.provider);
        }
        let isolated_directory;
        let directory = match self.scope.working_directory().filter(|path| path.is_dir()) {
            Some(directory) => directory,
            None => {
                isolated_directory =
                    tempfile::tempdir().context("create isolated model directory")?;
                isolated_directory.path()
            }
        };
        command.current_dir(directory);
        if let InvocationScope::ReadOnly {
            capture: Some(context),
            ..
        } = self.scope
        {
            context.apply(&mut command);
        }
        let process = ModelProcess::start(command, self.provider, self.model, self.prompt)?;
        process.finish(self.provider, self.timeout, deadline)
    }
}

enum ModelProcessSignal {
    OutputLimit(&'static str),
    StdinFailed(std::io::Error),
}

struct ModelPipes {
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

type InputWriter = std::thread::JoinHandle<std::io::Result<()>>;

struct ModelProcess {
    child: Child,
    stdin: InputWriter,
    stdout: OutputReader,
    stderr: OutputReader,
    signals: Receiver<ModelProcessSignal>,
    cancellation: Arc<AtomicBool>,
}

impl ModelProcess {
    fn start(
        command: Command,
        provider: ModelProvider,
        model: &ModelConfig,
        prompt: &str,
    ) -> Result<Self> {
        let mut child = spawn_model_process(command, provider, &model.command)?;
        let pipes = prepare_model_pipes(&mut child)?;
        Ok(Self::start_workers(child, pipes, prompt))
    }

    fn start_workers(child: Child, pipes: ModelPipes, prompt: &str) -> Self {
        let (signal_sender, signals) = mpsc::channel();
        let cancellation = Arc::new(AtomicBool::new(false));
        let stdout_signals = signal_sender.clone();
        let stdout = process_capture::spawn_bounded_capture(
            pipes.stdout,
            MODEL_OUTPUT_LIMIT_BYTES,
            cancellation.clone(),
            move || {
                let _ = stdout_signals.send(ModelProcessSignal::OutputLimit("stdout"));
            },
        );
        let stderr_signals = signal_sender.clone();
        let stderr = process_capture::spawn_bounded_capture(
            pipes.stderr,
            MODEL_OUTPUT_LIMIT_BYTES,
            cancellation.clone(),
            move || {
                let _ = stderr_signals.send(ModelProcessSignal::OutputLimit("stderr"));
            },
        );
        let prompt = prompt.as_bytes().to_vec();
        let writer_cancellation = cancellation.clone();
        let stdin = std::thread::spawn(move || {
            let result = write_model_prompt(pipes.stdin, &prompt, &writer_cancellation);
            if let Err(error) = &result {
                let signal_error = std::io::Error::new(error.kind(), error.to_string());
                let _ = signal_sender.send(ModelProcessSignal::StdinFailed(signal_error));
            }
            result
        });
        Self {
            child,
            stdin,
            stdout,
            stderr,
            signals,
            cancellation,
        }
    }

    fn finish(
        mut self,
        provider: ModelProvider,
        timeout: Duration,
        deadline: Instant,
    ) -> Result<String> {
        let status =
            wait_for_model_status(&mut self.child, provider, timeout, deadline, &self.signals);
        self.cancellation.store(true, Ordering::Release);
        let workers = JoinedModelWorkers::join(self.stdin, self.stdout, self.stderr);
        let status = status?;
        let (stdout, stderr) = workers.results()?;
        validate_captured_output(provider, status, stdout, stderr)
    }
}

fn spawn_model_process(
    mut command: Command,
    provider: ModelProvider,
    executable: &str,
) -> Result<Child> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "start {} model command {}",
                provider_name(provider),
                executable
            )
        })
}

fn prepare_model_pipes(child: &mut Child) -> Result<ModelPipes> {
    let pipes = take_model_pipes(child).and_then(|pipes| {
        configure_model_pipes(&pipes)?;
        Ok(pipes)
    });
    if pipes.is_err() {
        let _ = terminate_model_process(child);
    }
    pipes
}

fn take_model_pipes(child: &mut Child) -> Result<ModelPipes> {
    match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
        (Some(stdin), Some(stdout), Some(stderr)) => Ok(ModelPipes {
            stdin,
            stdout,
            stderr,
        }),
        _ => bail!("model process pipes unavailable"),
    }
}

fn configure_model_pipes(pipes: &ModelPipes) -> Result<()> {
    process_capture::set_nonblocking(&pipes.stdin).context("configure model stdin")?;
    process_capture::set_nonblocking(&pipes.stdout).context("configure model stdout")?;
    process_capture::set_nonblocking(&pipes.stderr).context("configure model stderr")?;
    Ok(())
}

fn write_model_prompt(
    mut stdin: impl Write,
    prompt: &[u8],
    cancellation: &AtomicBool,
) -> std::io::Result<()> {
    let mut written = 0;
    while written < prompt.len() {
        if cancellation.load(Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "model process ended before prompt delivery completed",
            ));
        }
        match stdin.write(&prompt[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "model stdin accepted zero bytes",
                ));
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(MODEL_STATUS_POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

enum ModelWaitOutcome {
    Exited(ExitStatus),
    OutputLimit(&'static str),
    StdinFailed(std::io::Error),
    TimedOut,
    Failed(std::io::Error),
}

fn wait_for_model_status(
    child: &mut Child,
    provider: ModelProvider,
    timeout: Duration,
    deadline: Instant,
    signals: &Receiver<ModelProcessSignal>,
) -> Result<ExitStatus> {
    let outcome = observe_model_status(child, deadline, signals);
    resolve_model_wait(child, provider, timeout, outcome)
}

fn observe_model_status(
    child: &mut Child,
    deadline: Instant,
    signals: &Receiver<ModelProcessSignal>,
) -> ModelWaitOutcome {
    loop {
        if let Some(outcome) = pending_model_signal(signals) {
            break outcome;
        }
        match child.try_wait() {
            Ok(Some(status)) => break ModelWaitOutcome::Exited(status),
            Ok(None) => {}
            Err(error) => break ModelWaitOutcome::Failed(error),
        }
        let now = Instant::now();
        if now >= deadline {
            break ModelWaitOutcome::TimedOut;
        }
        std::thread::sleep(MODEL_STATUS_POLL_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn pending_model_signal(signals: &Receiver<ModelProcessSignal>) -> Option<ModelWaitOutcome> {
    signals.try_iter().next().map(|signal| match signal {
        ModelProcessSignal::OutputLimit(stream) => ModelWaitOutcome::OutputLimit(stream),
        ModelProcessSignal::StdinFailed(error) => ModelWaitOutcome::StdinFailed(error),
    })
}

fn resolve_model_wait(
    child: &mut Child,
    provider: ModelProvider,
    timeout: Duration,
    outcome: ModelWaitOutcome,
) -> Result<ExitStatus> {
    match outcome {
        ModelWaitOutcome::Exited(status) => terminate_model_process(child).map(|()| status),
        ModelWaitOutcome::OutputLimit(stream) => terminate_model_process(child)
            .and_then(|()| Err(ModelOutputLimit { provider, stream }.into())),
        ModelWaitOutcome::StdinFailed(error) => {
            terminate_model_process(child).and_then(|()| Err(error).context("write model prompt"))
        }
        ModelWaitOutcome::TimedOut => terminate_model_process(child).and_then(|()| {
            Err(ModelTimeout {
                provider,
                seconds: timeout.as_secs(),
            }
            .into())
        }),
        ModelWaitOutcome::Failed(error) => {
            let _ = terminate_model_process(child);
            Err(error.into())
        }
    }
}

fn terminate_model_process(child: &mut Child) -> Result<()> {
    let group_result = signal_model_process_group(child);
    let parent_result = terminate_model_parent(child);
    parent_result?;
    group_result
}

fn signal_model_process_group(child: &Child) -> Result<()> {
    let process_group = i32::try_from(child.id()).context("model process id exceeds i32")?;
    // SAFETY: every provider is placed in a fresh process group whose id is the
    // positive child pid. Negating it targets only that group.
    let signal_result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if signal_result == -1 {
        let error = std::io::Error::last_os_error();
        if !matches!(
            error.raw_os_error(),
            Some(code) if code == libc::ESRCH || code == libc::EPERM
        ) {
            return Err(error).context("terminate model process group");
        }
    }
    Ok(())
}

fn terminate_model_parent(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        if let Err(error) = child.kill() {
            if child.try_wait()?.is_none() {
                return Err(error).context("terminate model parent process");
            }
            return Ok(());
        }
        child.wait()?;
    }
    Ok(())
}

struct JoinedModelWorkers {
    stdin: Result<()>,
    stdout: Result<CapturedOutput>,
    stderr: Result<CapturedOutput>,
}

impl JoinedModelWorkers {
    fn join(stdin: InputWriter, stdout: OutputReader, stderr: OutputReader) -> Self {
        Self {
            stdin: join_model_input(stdin),
            stdout: join_model_output(stdout, "stdout"),
            stderr: join_model_output(stderr, "stderr"),
        }
    }

    fn results(self) -> Result<(CapturedOutput, CapturedOutput)> {
        self.stdin?;
        Ok((self.stdout?, self.stderr?))
    }
}

fn join_model_input(writer: InputWriter) -> Result<()> {
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("model stdin writer panicked"))?
        .context("write model prompt")
}

fn join_model_output(reader: OutputReader, stream: &str) -> Result<CapturedOutput> {
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("model {stream} reader panicked"))?
        .map_err(Into::into)
}

fn validate_captured_output(
    provider: ModelProvider,
    status: ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
) -> Result<String> {
    if stdout.exceeded {
        return Err(ModelOutputLimit {
            provider,
            stream: "stdout",
        }
        .into());
    }
    if stderr.exceeded {
        return Err(ModelOutputLimit {
            provider,
            stream: "stderr",
        }
        .into());
    }
    validate_model_output(provider, status, stdout.bytes, stderr.bytes)
}

fn validate_model_output(
    provider: ModelProvider,
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> Result<String> {
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

#[cfg(test)]
mod tests;
