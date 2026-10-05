// @feature installation
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/installation.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @boundary child-process-json
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn machine_setup_installs_integrations_idempotently() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let command_directory = temporary.path().join("commands");
    let host_log = temporary.path().join("hosts.log");
    fs::create_dir_all(&command_directory).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(
        codex_home.join("hooks.json"),
        r#"{"theme":"user-choice","hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"keep-me"}]}]}}"#,
    )
    .unwrap();
    fake_host(&command_directory, "codex");
    fake_host(&command_directory, "claude");
    fake_host(&command_directory, "pi");

    for _ in 0..2 {
        let setup = nexus_command(&home, &codex_home, &command_directory, &host_log)
            .arg("setup")
            .output()
            .unwrap();
        assert_success(&setup);
    }
    assert_machine_setup(&home, &codex_home, &host_log);
    assert!(home.join(".pi/agent/extensions/nexus/index.ts").is_file());
    assert!(home
        .join(".pi/agent/extensions/nexus/component.ts")
        .is_file());
    assert!(home.join(".pi/agent/extensions/nexus/capture.ts").is_file());
    assert!(home.join(".pi/agent/extensions/nexus/usage.ts").is_file());
    assert!(home
        .join(".pi/agent/extensions/nexus/usage-pane.ts")
        .is_file());
    for memory_module in [
        "host-context.ts",
        "mcp-client.ts",
        "memory-context.ts",
        "memory-pane.ts",
        "memory-prompts.ts",
        "memory.ts",
    ] {
        assert!(
            home.join(".pi/agent/extensions/nexus")
                .join(memory_module)
                .is_file(),
            "machine setup did not install {memory_module}"
        );
    }
}

#[test]
fn repository_install_preserves_existing_configuration() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let command_directory = temporary.path().join("commands");
    let host_log = temporary.path().join("hosts.log");
    let repository = temporary.path().join("repository");
    run(Command::new("git").args(["init", repository.to_str().unwrap()]));
    let repository_config = repository.join(".git/nexus.toml");
    let user_config = "schema_version = 1\n\n[privacy]\nstore_prompt_synopsis = false\n";
    fs::write(&repository_config, user_config).unwrap();
    let install = nexus_command(&home, &codex_home, &command_directory, &host_log)
        .arg("install")
        .current_dir(&repository)
        .output()
        .unwrap();
    assert_success(&install);
    assert_eq!(fs::read_to_string(repository_config).unwrap(), user_config);
    let report: Value = serde_json::from_slice(&install.stdout).unwrap();
    assert_eq!(report["mode"], "advisory_only");
    assert_eq!(report["database"], "ready");
}

fn assert_machine_setup(home: &Path, codex_home: &Path, host_log: &Path) {
    assert!(codex_home.join("skills/code-architect/SKILL.md").is_file());
    assert!(codex_home.join("skills/code-deletion/SKILL.md").is_file());
    assert!(codex_home.join("skills/tdd/SKILL.md").is_file());
    let codex_hooks = settings(&codex_home.join("hooks.json"));
    assert_eq!(codex_hooks["theme"], "user-choice");
    assert!(serde_json::to_string(&codex_hooks)
        .unwrap()
        .contains("keep-me"));
    assert_eq!(count_server(&codex_hooks, "nexus"), 3);
    assert_hook(&codex_hooks, "nexus_pre_tool_use");
    assert_session_end_hook(&codex_hooks, "codex");
    assert_session_end_hook(&settings(&home.join(".claude/settings.json")), "claude");
    let registrations = fs::read_to_string(host_log).unwrap();
    assert!(registrations.contains("codex mcp add nexus"));
    assert!(registrations.contains("claude mcp add --transport stdio --scope user nexus"));
}

fn nexus_command(
    home: &Path,
    codex_home: &Path,
    command_directory: &Path,
    host_log: &Path,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = vec![command_directory.to_path_buf()];
    path.extend(std::env::split_paths(&inherited_path));
    command
        .env("HOME", home)
        .env("CODEX_HOME", codex_home)
        .env("NEXUS_STATE_DIR", home.join(".local/state/nexus"))
        .env("NEXUS_TEST_HOST_LOG", host_log)
        .env("PATH", std::env::join_paths(path).unwrap());
    command
}

fn fake_host(directory: &Path, name: &str) {
    let path = directory.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf '%s %s\\n' '{name}' \"$*\" >> \"$NEXUS_TEST_HOST_LOG\"\n"),
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn settings(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn assert_hook(settings: &Value, tool: &str) {
    assert!(serde_json::to_string(settings).unwrap().contains(tool));
}

fn assert_session_end_hook(settings: &Value, agent: &str) {
    let command = format!("hook-session-end --agent {agent}");
    let serialized = serde_json::to_string(settings).unwrap();
    assert!(serialized.contains(&command));
    assert_eq!(serialized.matches(&command).count(), 1);
}

fn count_server(value: &Value, server: &str) -> usize {
    match value {
        Value::Object(object) => {
            usize::from(object.get("server").and_then(Value::as_str) == Some(server))
                + object
                    .values()
                    .map(|value| count_server(value, server))
                    .sum::<usize>()
        }
        Value::Array(values) => values.iter().map(|value| count_server(value, server)).sum(),
        _ => 0,
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert_success(&output);
}
