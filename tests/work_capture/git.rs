// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary delayed-capture-fixture: a controlled Git shim pauses project resolution after source metadata is read, without delaying the workload or changing captured metadata.
use super::*;
use std::path::PathBuf;
use std::process::{Child, Stdio};

struct DelayedHook {
    child: Child,
    entered: PathBuf,
    release: PathBuf,
}
impl Drop for DelayedHook {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.release, b"release");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl DelayedHook {
    fn start(harness: &Harness) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = harness._temp.path().join("delayed-git");
        std::fs::create_dir(&directory).unwrap();
        let armed = directory.join("metadata-complete");
        let entered = directory.join("capture-pending");
        let release = directory.join("release-capture");
        let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("git"))
            .find(|path| path.is_file())
            .unwrap();
        let shim = directory.join("git");
        std::fs::write(
            &shim,
            format!(
                r#"#!/bin/sh
if [ "$1" = rev-parse ] && [ "$2" = --verify ] && [ "$3" = HEAD ]; then
  '{}' "$@" || exit $?
  touch '{}'
  exit 0
fi
if [ "$1" = -C ] && [ -f '{}' ]; then
  touch '{}'
  while [ ! -f '{}' ] && [ -d '{}' ]; do sleep 0.01; done
fi
exec '{}' "$@"
"#,
                real_git.display(),
                armed.display(),
                armed.display(),
                entered.display(),
                release.display(),
                directory.display(),
                real_git.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths(
            std::iter::once(directory)
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let child = cli(harness)
            .args(["work", "git-hook", "post-commit"])
            .env("PATH", path)
            .env("NEXUS_COMMIT_OPERATION_ID", "delayed-commit")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child,
            entered,
            release,
        }
    }
    fn await_capture(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.entered.exists() {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "hook exited before capture admission"
            );
            assert!(
                Instant::now() < deadline,
                "capture did not reach the project-resolution barrier"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn release_after_foreground_budget(&mut self) -> Option<std::process::ExitStatus> {
        // Hold admitted capture beyond its 75 ms foreground wait, then release it.
        let deadline = Instant::now() + Duration::from_millis(150);
        let mut early_exit = None;
        while Instant::now() < deadline {
            early_exit = self.child.try_wait().unwrap();
            if early_exit.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::write(&self.release, b"release").unwrap();
        early_exit
    }
}

#[test]
fn git_hook_keeps_admitted_capture_alive_until_local_persistence_completes() {
    let mut harness = Harness::start();
    harness.initialize_git();
    let mut hook = DelayedHook::start(&harness);
    hook.await_capture();
    let early_exit = hook.release_after_foreground_budget();
    assert!(early_exit.is_none(), "the Git hook exited with an admitted capture worker blocked before persistence: {early_exit:?}");
    assert!(hook.child.wait().unwrap().success());
    wait_for_work_delivery(&harness);
    let events = harness.call(
        "nexus_work",
        json!({"action":"events","subject":{"scope":"activity","id":"delayed-commit"}}),
    );
    assert!(
        events["observations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["body"]["event"] == "linked"
                && event["body"]["relation"] == "produced"),
        "{events}"
    );
}
