// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary synthetic-host-title-source: isolated metadata fixtures never read a user's transcripts or contact a provider.
mod support;
use std::process::{Command, Stdio};
use std::time::Duration;
use support::Harness;
use wait_timeout::ChildExt;

#[test]
fn a_nonregular_title_source_cannot_block_recording_the_host_boundary() {
    let harness = Harness::start();
    let pipe = harness._temp.path().join("transcript-pipe");
    assert!(Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap()
        .success());
    let event = harness._temp.path().join("event.json");
    std::fs::write(
        &event,
        serde_json::to_vec(&serde_json::json!({
            "session_id":"title-source", "hook_event_name":"SessionStart", "cwd":harness.root,
            "transcript_path":pipe
        }))
        .unwrap(),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args([
            "--config",
            harness._temp.path().join("config.toml").to_str().unwrap(),
            "work",
            "host-event",
            "--host",
            "claude",
        ])
        .arg(event)
        .current_dir(&harness.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = child.wait_timeout(Duration::from_secs(2)).unwrap();
    if status.is_none() {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "optional title enrichment blocked the observed host boundary: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
