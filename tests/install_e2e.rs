// @feature installation
// @spec docs/features/installation.md
// @boundary child-process-json
mod support;

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use support::{assert_success, git_command, wait_for_path};

#[test]
#[ignore = "requires authenticated Codex and Claude CLIs and consumes model resources"]
fn fresh_install_coordinates_real_agents_across_worktrees() {
    require_command("codex");
    require_command("claude");
    let temporary = tempfile::tempdir().unwrap();
    let install_root = temporary.path().join("install");
    let setup_home = temporary.path().join("setup-home");
    let setup_state = temporary.path().join("setup-state");
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh");
    let install = Command::new(installer)
        .env("HOME", &setup_home)
        .env("CODEX_HOME", setup_home.join(".codex"))
        .env("NEXUS_STATE_DIR", &setup_state)
        .env("NEXUS_INSTALL_ROOT", &install_root)
        .output()
        .unwrap();
    assert_success("machine install", &install);
    let nexus = install_root.join("bin/nexus");

    let repository = temporary.path().join("repository");
    initialize_repository(&repository);
    let worktree_a = temporary.path().join("worktree-codex");
    let worktree_b = temporary.path().join("worktree-claude");
    run_git(
        &repository,
        ["worktree", "add", "-b", "codex-e2e", path(&worktree_a)],
    );
    run_git(
        &repository,
        ["worktree", "add", "-b", "claude-e2e", path(&worktree_b)],
    );
    let state = temporary.path().join("coordination-state");
    let repository_install = Command::new(&nexus)
        .arg("install")
        .current_dir(&repository)
        .env("NEXUS_STATE_DIR", &state)
        .output()
        .unwrap();
    assert_success("repository install", &repository_install);

    let mut daemon = Command::new(&nexus)
        .arg("daemon")
        .current_dir(&repository)
        .env("NEXUS_STATE_DIR", &state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_path(&state.join("nexus.sock"), &mut daemon);

    let codex = codex_call(&nexus, &state, &worktree_a);
    let claude = claude_call(&nexus, &state, &worktree_b, temporary.path());
    let conflicts = Command::new(&nexus)
        .args(["conflicts", "--all"])
        .current_dir(&repository)
        .env("NEXUS_STATE_DIR", &state)
        .output()
        .unwrap();
    daemon.kill().unwrap();
    daemon.wait().unwrap();

    assert_success("Codex tool call", &codex);
    assert_success("Claude tool call", &claude);
    assert_success("conflict query", &conflicts);
    let response: Value = serde_json::from_slice(&conflicts.stdout).unwrap();
    let conflict = response["conflicts"].as_array().unwrap().first().unwrap();
    assert_eq!(conflict["severity"], "critical");
    assert_eq!(conflict["kind"], "hunk_overlap");
    assert!(conflict["path"].as_str().unwrap().ends_with("shared.txt"));
}

fn codex_call(nexus: &Path, state: &Path, worktree: &Path) -> Output {
    let command_value = serde_json::to_string(path(nexus)).unwrap();
    let args_value = "[\"mcp\"]";
    let env_value = format!(
        "{{ NEXUS_STATE_DIR = {} }}",
        serde_json::to_string(path(state)).unwrap()
    );
    Command::new("codex")
        .args([
            "exec",
            "--ephemeral",
            "--ignore-user-config",
            "--sandbox",
            "read-only",
            "--color",
            "never",
            "-C",
            path(worktree),
            "-c",
            &format!("mcp_servers.nexus_e2e.command={command_value}"),
            "-c",
            &format!("mcp_servers.nexus_e2e.args={args_value}"),
            "-c",
            &format!("mcp_servers.nexus_e2e.env={env_value}"),
            &agent_prompt("codex-e2e", worktree),
        ])
        .output()
        .unwrap()
}

fn claude_call(nexus: &Path, state: &Path, worktree: &Path, temporary: &Path) -> Output {
    let mcp = temporary.join("claude-mcp.json");
    fs::write(
        &mcp,
        serde_json::to_vec_pretty(&json!({
            "mcpServers": {
                "nexus_e2e": {
                    "command": nexus,
                    "args": ["mcp"],
                    "env": {"NEXUS_STATE_DIR": state}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "json",
            "--permission-mode",
            "dontAsk",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--mcp-config",
            path(&mcp),
            "--allowedTools",
            "mcp__nexus_e2e__nexus_pre_tool_use",
            "--model",
            "claude-opus-5",
            "--effort",
            "xhigh",
            &agent_prompt("claude-e2e", worktree),
        ])
        .current_dir(worktree)
        .output()
        .unwrap()
}

fn agent_prompt(session: &str, worktree: &Path) -> String {
    format!(
        "Call the nexus_e2e MCP tool nexus_pre_tool_use exactly once with session_id {session}, agent {session}, project_root {}, tool_use_id {session}-edit, tool_name Edit, and tool_input containing file_path shared.txt, line_start 10, and line_end 20. Do not call any other tool. After the MCP call, reply DONE.",
        worktree.display()
    )
}

fn initialize_repository(repository: &Path) {
    run_git(repository.parent().unwrap(), ["init", path(repository)]);
    fs::write(repository.join("shared.txt"), "one\ntwo\nthree\n").unwrap();
    run_git(
        repository,
        ["config", "user.email", "nexus-e2e@example.invalid"],
    );
    run_git(repository, ["config", "user.name", "Nexus E2E"]);
    run_git(repository, ["add", "shared.txt"]);
    run_git(repository, ["commit", "-m", "fixture"]);
}

fn require_command(command: &str) {
    let output = Command::new(command).arg("--version").output().unwrap();
    assert_success(command, &output);
}

fn run_git<const N: usize>(directory: &Path, arguments: [&str; N]) {
    let output = git_command()
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap();
    assert_success("git", &output);
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}
