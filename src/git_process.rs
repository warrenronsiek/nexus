// @feature configuration
// @feature coordination
// @feature installation
// @spec docs/features/configuration.md
// @spec docs/features/coordination.md
// @spec docs/features/installation.md
// @boundary git-process-output
use crate::process_capture::{self, CapturedOutput, OutputReader};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const COMMON_DIRECTORY_TIMEOUT: Duration = Duration::from_secs(5);
const COMMON_DIRECTORY_STREAM_LIMIT: usize = 64 * 1024;

pub(crate) fn common_directory(root: &Path) -> std::io::Result<Option<PathBuf>> {
    common_directory_with_git(
        root,
        Path::new("git"),
        COMMON_DIRECTORY_TIMEOUT,
        COMMON_DIRECTORY_STREAM_LIMIT,
    )
}

fn common_directory_with_git(
    root: &Path,
    git: &Path,
    timeout: Duration,
    stream_limit: usize,
) -> std::io::Result<Option<PathBuf>> {
    let mut command = Command::new(git);
    command
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-common-dir"]);
    let output = bounded_output(command, timeout, stream_limit)?;
    if !output.status.success() {
        return Ok(None);
    }
    let raw = String::from_utf8(output.stdout)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let common = PathBuf::from(raw.trim());
    Ok(Some(if common.is_absolute() {
        common
    } else {
        root.join(common)
    }))
}

pub(crate) fn bounded_output(
    mut command: Command,
    timeout: Duration,
    stream_limit: usize,
) -> std::io::Result<Output> {
    command.process_group(0);
    let mut process = start_process(command, stream_limit)?;
    let status = wait_for_status(&mut process.child, timeout, stream_limit, &process.signals);
    process.cancellation.store(true, Ordering::Release);
    finish_output(status, process.stdout, process.stderr, stream_limit)
}

struct BoundedProcess {
    child: std::process::Child,
    stdout: OutputReader,
    stderr: OutputReader,
    signals: Receiver<ProcessSignal>,
    cancellation: Arc<AtomicBool>,
}

enum ProcessSignal {
    OutputLimit(&'static str),
}

enum WaitOutcome {
    Exited(std::process::ExitStatus),
    OutputLimit(&'static str),
    TimedOut,
    Failed(std::io::Error),
}

fn start_process(mut command: Command, stream_limit: usize) -> std::io::Result<BoundedProcess> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped Git stdout");
    let stderr = child.stderr.take().expect("piped Git stderr");
    if let Err(error) = process_capture::set_nonblocking(&stdout)
        .and_then(|()| process_capture::set_nonblocking(&stderr))
    {
        let _ = terminate_process(&mut child);
        return Err(error);
    }
    let (signal_sender, signals) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    let stdout_signals = signal_sender.clone();
    let stdout_reader = process_capture::spawn_bounded_capture(
        stdout,
        stream_limit,
        cancellation.clone(),
        move || {
            let _ = stdout_signals.send(ProcessSignal::OutputLimit("stdout"));
        },
    );
    let stderr_reader = process_capture::spawn_bounded_capture(
        stderr,
        stream_limit,
        cancellation.clone(),
        move || {
            let _ = signal_sender.send(ProcessSignal::OutputLimit("stderr"));
        },
    );
    Ok(BoundedProcess {
        child,
        stdout: stdout_reader,
        stderr: stderr_reader,
        signals,
        cancellation,
    })
}

fn finish_output(
    status: std::io::Result<std::process::ExitStatus>,
    stdout_reader: OutputReader,
    stderr_reader: OutputReader,
    stream_limit: usize,
) -> std::io::Result<Output> {
    let stdout = join_output(stdout_reader, "stdout");
    let stderr = join_output(stderr_reader, "stderr");
    validate_output(status?, stdout?, stderr?, stream_limit)
}

fn validate_output(
    status: std::process::ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    stream_limit: usize,
) -> std::io::Result<Output> {
    match (stdout.exceeded, stderr.exceeded) {
        (true, _) => Err(output_limit_error("stdout", stream_limit)),
        (_, true) => Err(output_limit_error("stderr", stream_limit)),
        (false, false) => Ok(Output {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
        }),
    }
}

fn wait_for_status(
    child: &mut std::process::Child,
    timeout: Duration,
    stream_limit: usize,
    signals: &Receiver<ProcessSignal>,
) -> std::io::Result<std::process::ExitStatus> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let outcome = loop {
        if let Ok(ProcessSignal::OutputLimit(stream)) = signals.try_recv() {
            break WaitOutcome::OutputLimit(stream);
        }
        match child.try_wait() {
            Ok(Some(status)) => break WaitOutcome::Exited(status),
            Ok(None) => {}
            Err(error) => break WaitOutcome::Failed(error),
        }
        let now = Instant::now();
        if now >= deadline {
            break WaitOutcome::TimedOut;
        }
        std::thread::sleep(STATUS_POLL_INTERVAL.min(deadline.duration_since(now)));
    };
    resolve_wait(child, outcome, stream_limit)
}

fn resolve_wait(
    child: &mut std::process::Child,
    outcome: WaitOutcome,
    stream_limit: usize,
) -> std::io::Result<std::process::ExitStatus> {
    match outcome {
        WaitOutcome::Exited(status) => terminate_process(child).map(|()| status),
        WaitOutcome::OutputLimit(stream) => {
            terminate_process(child).and_then(|()| Err(output_limit_error(stream, stream_limit)))
        }
        WaitOutcome::TimedOut => terminate_process(child).and_then(|()| {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Git command exceeded the Nexus observation deadline",
            ))
        }),
        WaitOutcome::Failed(error) => {
            let _ = terminate_process(child);
            Err(error)
        }
    }
}

fn terminate_process(child: &mut std::process::Child) -> std::io::Result<()> {
    let signal = signal_process_group(child.id());
    let reap = reap_process(child);
    signal.and(reap)
}

fn signal_process_group(process_id: u32) -> std::io::Result<()> {
    let process_group = i32::try_from(process_id).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Git process id exceeds the platform process-group range",
        )
    })?;
    // SAFETY: the command is placed in a fresh process group whose id is the
    // positive child pid, so the negative id targets only that group.
    let signal_result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if signal_result == -1 {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    } else {
        Ok(())
    }
}

fn reap_process(child: &mut std::process::Child) -> std::io::Result<()> {
    match child.try_wait()? {
        Some(_) => Ok(()),
        None => kill_and_reap_process(child),
    }
}

fn kill_and_reap_process(child: &mut std::process::Child) -> std::io::Result<()> {
    match child.kill() {
        Ok(()) => child.wait().map(|_| ()),
        Err(error) => match child.try_wait()? {
            Some(_) => Ok(()),
            None => Err(error),
        },
    }
}

fn join_output(reader: OutputReader, stream: &'static str) -> std::io::Result<CapturedOutput> {
    reader
        .join()
        .map_err(|_| std::io::Error::other(format!("Git {stream} capture worker panicked")))?
}

fn output_limit_error(stream: &'static str, stream_limit: usize) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("Git command {stream} exceeded the {stream_limit}-byte capture limit"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn successful_parent_exit_terminates_background_descendants() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "sleep 30 & descendant=$!; printf '%s\\n' \"$descendant\"; exit 0",
        ]);

        let output = bounded_output(command, Duration::from_secs(2), 1_024).unwrap();
        assert!(output.status.success());
        let process_id = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();

        let still_exists = unsafe { libc::kill(process_id, 0) } == 0;
        if still_exists {
            let _ = unsafe { libc::kill(process_id, libc::SIGKILL) };
        }
        assert!(
            !still_exists,
            "background Git descendant {process_id} survived"
        );
    }

    #[test]
    fn common_directory_bounds_git_time_and_output() {
        let temporary = tempfile::tempdir().unwrap();
        let fake_git = temporary.path().join("git");
        std::fs::write(&fake_git, "#!/bin/sh\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();

        assert!(common_directory_with_git(
            temporary.path(),
            &fake_git,
            Duration::from_millis(50),
            1_024,
        )
        .is_err());
        assert!(started.elapsed() < Duration::from_secs(1));

        std::fs::write(
            &fake_git,
            "#!/bin/sh\nexec python3 -c 'import os; os.write(1, b\"x\" * 1025)'\n",
        )
        .unwrap();
        assert!(common_directory_with_git(
            temporary.path(),
            &fake_git,
            Duration::from_secs(1),
            1_024,
        )
        .is_err());
    }
}
