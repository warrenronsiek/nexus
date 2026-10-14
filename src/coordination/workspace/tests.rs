// @feature coordination
// @spec docs/features/coordination.md
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

const TEST_GIT_STREAM_LIMIT_BYTES: usize = 64 * 1024;

#[test]
fn a_hung_git_process_is_killed_at_the_observation_deadline() {
    let temporary = tempfile::tempdir().unwrap();
    let fake_git = temporary.path().join("git");
    std::fs::write(&fake_git, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o700)).unwrap();
    let command = Command::new(fake_git);
    let started = Instant::now();

    let error = command_output_with_timeout(command, Duration::from_millis(50)).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn oversized_git_stdout_is_stopped_and_reaped() {
    assert_oversized_git_stream_is_stopped_and_reaped("stdout");
}

#[test]
fn oversized_git_stderr_is_stopped_and_reaped() {
    assert_oversized_git_stream_is_stopped_and_reaped("stderr");
}

fn assert_oversized_git_stream_is_stopped_and_reaped(stream: &str) {
    let temporary = tempfile::tempdir().unwrap();
    let fake_git = temporary.path().join("git");
    let pid_file = temporary.path().join("git.pid");
    let file_descriptor = if stream == "stderr" { 2 } else { 1 };
    std::fs::write(
        &fake_git,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$1\"\nexec python3 -c 'import os,time; os.write({file_descriptor}, b\"x\" * {}); time.sleep(60)'\n",
            TEST_GIT_STREAM_LIMIT_BYTES + 1,
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(fake_git);
    command.arg(&pid_file);
    let error = command_output_with_capture_limit(
        command,
        Duration::from_secs(2),
        TEST_GIT_STREAM_LIMIT_BYTES,
    )
    .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("capture limit"));
    assert_reaped(&pid_file);
}

fn assert_reaped(pid_file: &std::path::Path) {
    let pid = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    let result = unsafe { libc::kill(pid, 0) };
    assert_eq!(result, -1, "Git process {pid} is still alive");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
