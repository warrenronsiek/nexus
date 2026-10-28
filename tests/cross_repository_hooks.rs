// @feature runtime
// @feature coordination
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @boundary native-shell-hook-fixture
mod support;
use serde_json::json;
use std::process::Command;
use support::Harness;

#[test]
fn shell_workdir_delivers_overlap_from_the_target_checkout() {
    assert_workdir_overlap("");
}

#[test]
fn shell_subdirectory_keeps_repository_relative_claim_paths() {
    assert_workdir_overlap("sub");
}

fn assert_workdir_overlap(subdirectory: &str) {
    let mut harness = Harness::start();
    let target = harness._temp.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target)
        .status()
        .unwrap()
        .success());
    let directory = target.join(subdirectory);
    std::fs::create_dir_all(&directory).unwrap();
    let file = if subdirectory.is_empty() {
        "shared.txt".to_owned()
    } else {
        format!("{subdirectory}/shared.txt")
    };
    let mut hook = json!({"agent":"codex","session_id":"peer","project_root":target,
        "tool_use_id":"peer-edit","tool_name":"Write","tool_input":{"file_path":file}});
    assert_eq!(
        harness.call("nexus_pre_tool_use", hook.clone())["recorded"],
        true
    );
    hook["session_id"] = json!("author");
    hook["project_root"] = json!(harness.root);
    hook["tool_use_id"] = json!("cross-checkout-edit");
    hook["tool_name"] = json!("Bash");
    hook["tool_input"] = json!({"cmd":"touch shared.txt","workdir":directory});
    let result = harness.call("nexus_pre_tool_use", hook);
    assert_eq!(result["permitted"], true);
    assert_eq!(
        result["advisories"].as_array().unwrap().len(),
        1,
        "{result}"
    );
    assert!(result["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("peer"));
}

#[test]
fn git_integration_commands_warn_before_either_command_executes() {
    for operation in [
        "merge --no-commit topic",
        "cherry-pick topic",
        "-C . merge --no-commit topic",
    ] {
        let mut harness = Harness::start();
        let target = harness._temp.path().join("integration-target");
        std::fs::create_dir_all(&target).unwrap();
        let git = |args: &[&str]| {
            let result = Command::new("git")
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                ])
                .args(args)
                .current_dir(&target)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            String::from_utf8(result.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        std::fs::write(target.join("shared.txt"), "base\n").unwrap();
        git(&["add", "shared.txt"]);
        git(&["commit", "-qm", "base"]);
        let base = git(&["rev-parse", "HEAD"]);
        git(&["checkout", "-qb", "topic"]);
        std::fs::write(target.join("shared.txt"), "incoming\n").unwrap();
        git(&["commit", "-qam", "incoming"]);
        git(&["checkout", "-qb", "target", &base]);
        let startup = harness.root.clone();
        let hook = |session: &str| {
            json!({"agent":"codex","session_id":session,"project_root":startup,
            "tool_use_id":format!("{session}-integration"),"tool_name":"Bash",
            "tool_input":{"cmd":format!("git {operation}"),"workdir":target}})
        };
        harness.call("nexus_pre_tool_use", hook("first-agent"));
        let result = harness.call("nexus_pre_tool_use", hook("second-agent"));
        assert_eq!(
            result["advisories"].as_array().unwrap().len(),
            1,
            "{operation}: {result}"
        );
        assert!(result["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("first-agent"));
        assert_eq!(git(&["rev-parse", "HEAD"]), base);
        assert!(git(&["status", "--porcelain"]).is_empty());
    }
}

#[test]
fn stopping_a_host_releases_claims_in_its_other_checkout() {
    assert_stop_cleanup(false, false);
}

#[test]
fn native_session_end_releases_claims_in_visited_checkouts() {
    assert_stop_cleanup(true, false);
}

#[test]
fn malformed_cleanup_journal_is_preserved_for_diagnosis() {
    assert_stop_cleanup(true, true);
}

fn assert_stop_cleanup(native: bool, corrupt: bool) {
    let mut harness = Harness::start();
    let target = harness._temp.path().join("stop-target");
    std::fs::create_dir_all(&target).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target)
        .status()
        .unwrap()
        .success());
    harness.call(
        "nexus_pre_tool_use",
        json!({"agent":"codex","session_id":"departing",
        "project_root":harness.root,"tool_use_id":"target-edit","tool_name":"Bash",
        "tool_input":{"cmd":"touch shared.txt","workdir":target}}),
    );
    let claims = harness.call("nexus_claims", json!({}));
    assert!(claims["claims"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["session_id"] == "departing"));
    let journal = std::fs::read_dir(
        harness
            .database
            .parent()
            .unwrap()
            .join("work-outbox/checkout-visits"),
    )
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .find(|path| path.extension().is_some_and(|ext| ext == "json"))
    .unwrap();
    if corrupt {
        std::fs::write(&journal, b"invalid journal").unwrap();
    }
    if native {
        run_native_stop(&harness);
    } else {
        harness.call(
            "nexus_session_stop",
            json!({"agent":"codex","session_id":"departing","project_root":harness.root}),
        );
    }
    if corrupt {
        assert!(
            journal.is_file(),
            "failed cleanup must preserve its routing evidence"
        );
        return;
    }
    assert!(
        !journal.exists(),
        "completed shutdown must remove its routing journal"
    );
    let claims = harness.call("nexus_claims", json!({}));
    assert!(
        !claims["claims"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["session_id"] == "departing"),
        "{claims}"
    );
}

fn run_native_stop(harness: &Harness) {
    use std::io::Write;
    use std::process::Stdio;
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    for (key, _) in
        std::env::vars().filter(|(key, _)| key.starts_with("NEXUS_") || key.starts_with("GIT_"))
    {
        command.env_remove(key);
    }
    let mut child = command
        .arg("--config")
        .arg(harness._temp.path().join("config.toml"))
        .args(["hook-session-end", "--agent", "codex"])
        .current_dir(&harness.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            serde_json::to_string(&json!({"session_id":"departing","cwd":harness.root}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn unavailable_execution_directory_discloses_missing_coordination_to_the_agent() {
    let mut harness = Harness::start();
    let result = harness.call(
        "nexus_pre_tool_use",
        json!({"agent":"codex","session_id":"unavailable",
        "project_root":harness.root,"tool_use_id":"missing-checkout","tool_name":"Bash",
        "tool_input":{"cmd":"touch shared.txt","workdir":harness._temp.path().join("missing")}}),
    );
    assert_eq!(result["permitted"], true);
    assert_eq!(result["recorded"], false);
    assert!(
        result["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some_and(|text| text.contains("coverage")),
        "{result}"
    );
}

#[test]
fn absolute_file_edits_use_the_target_repository() {
    assert_file_target(true, false);
}

#[test]
fn relative_file_edits_from_subdirectories_keep_their_paths() {
    assert_file_target(false, false);
}

#[test]
fn absolute_file_edits_through_directory_symlinks_share_claims() {
    assert_file_target(true, true);
}

fn assert_file_target(absolute: bool, symlink: bool) {
    let mut harness = Harness::start();
    let target = harness._temp.path().join("file-target");
    std::fs::create_dir_all(target.join("sub")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target)
        .status()
        .unwrap()
        .success());
    harness.call(
        "nexus_pre_tool_use",
        json!({"agent":"codex","session_id":"file-peer","project_root":target,
        "tool_use_id":"peer-file","tool_name":"Write","tool_input":{"file_path":"sub/shared.txt"}}),
    );
    let reported_target = if symlink {
        let alias = harness._temp.path().join("checkout-alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        alias
    } else {
        target.clone()
    };
    let (root, file) = if absolute {
        (harness.root.clone(), reported_target.join("sub/shared.txt"))
    } else {
        (target.join("sub"), std::path::PathBuf::from("shared.txt"))
    };
    let result = harness.call(
        "nexus_pre_tool_use",
        json!({"agent":"codex","session_id":"file-author","project_root":root,
        "tool_use_id":"author-file","tool_name":"Write","tool_input":{"file_path":file}}),
    );
    assert_eq!(
        result["advisories"].as_array().unwrap().len(),
        1,
        "{result}"
    );
}

#[test]
fn exhausted_teardown_journals_do_not_disable_coordination() {
    use std::os::unix::fs::PermissionsExt;
    let mut harness = Harness::start();
    let directory = harness
        .database
        .parent()
        .unwrap()
        .join("work-outbox/checkout-visits");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    for index in 0..4096 {
        std::fs::write(directory.join(format!("foreign-{index}.json")), "[]").unwrap();
    }
    let target = harness._temp.path().join("unrecorded-visit");
    std::fs::create_dir_all(&target).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target)
        .status()
        .unwrap()
        .success());
    for root in [harness.root.clone(), target] {
        let result = harness.call("nexus_pre_tool_use", json!({"agent":"codex","session_id":"journal-full",
            "project_root":root, "tool_use_id":"edit", "tool_name":"Write", "tool_input":{"file_path":"shared.txt"}}));
        assert_eq!(
            result["recorded"], true,
            "teardown storage must not control capture: {result}"
        );
    }
}
