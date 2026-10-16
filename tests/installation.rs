// @feature installation
// @feature sos
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/installation.md
// @spec docs/features/sos.md
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
    assert!(home.join(".pi/agent/extensions/nexus/display.ts").is_file());
    let launch_context =
        fs::read_to_string(home.join(".pi/agent/extensions/nexus/launch-context.ts")).unwrap();
    assert!(launch_context.contains(env!("CARGO_BIN_EXE_nexus")));
    assert!(!launch_context.contains("config:"));
    assert!(home.join(".pi/agent/extensions/nexus/usage.ts").is_file());
    assert!(home
        .join(".pi/agent/extensions/nexus/usage-pane.ts")
        .is_file());
    for sos_module in ["sos.ts", "sos-pane.ts"] {
        assert!(
            home.join(".pi/agent/extensions/nexus")
                .join(sos_module)
                .is_file(),
            "machine setup did not install {sos_module}"
        );
    }
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
fn machine_setup_honors_an_explicit_remote_configuration() {
    exercise_remote_machine_setup(|command, config_name| {
        command.args(["--config", config_name, "setup"]);
    });
}

#[test]
fn machine_setup_persists_a_relative_nexus_config_environment_path() {
    exercise_remote_machine_setup(|command, config_name| {
        command.arg("setup").env("NEXUS_CONFIG", config_name);
    });
}

fn exercise_remote_machine_setup(configure: impl FnOnce(&mut Command, &str)) {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let command_directory = temporary.path().join("commands");
    let host_log = temporary.path().join("hosts.log");
    let config_name = "remote.toml";
    let config = temporary.path().join(config_name);
    let persisted_config = temporary.path().canonicalize().unwrap().join(config_name);
    fs::create_dir_all(&command_directory).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fake_host(&command_directory, "codex");
    fake_host(&command_directory, "pi");
    fs::write(
        &config,
        r#"[client]
transport = "remote"
endpoint = "https://nexus.example.test"
project_id = "shared-project"
"#,
    )
    .unwrap();

    let mut command = nexus_command(&home, &codex_home, &command_directory, &host_log);
    configure(&mut command, config_name);
    let setup = command.current_dir(temporary.path()).output().unwrap();
    assert_success(&setup);
    let report: Value = serde_json::from_slice(&setup.stdout).unwrap();
    assert_eq!(report["database"], "remote");
    assert!(!home.join(".local/state/nexus/nexus.db").exists());
    assert_remote_integrations(&home, &codex_home, &host_log, &persisted_config);
    assert_session_end_works_from_another_directory(&home, &codex_home);
}

fn assert_remote_integrations(
    home: &Path,
    codex_home: &Path,
    host_log: &Path,
    persisted_config: &Path,
) {
    let registrations = fs::read_to_string(host_log).unwrap();
    assert!(
        registrations.contains(&format!(
            "codex mcp add nexus -- {} --config {} mcp --agent codex",
            env!("CARGO_BIN_EXE_nexus"),
            persisted_config.display()
        )),
        "{registrations}"
    );
    let hooks = serde_json::to_string(&settings(&codex_home.join("hooks.json"))).unwrap();
    assert!(hooks.contains(persisted_config.to_str().unwrap()));
    assert!(hooks.contains("hook-session-end --agent codex"));
    let pi_launch_context =
        fs::read_to_string(home.join(".pi/agent/extensions/nexus/launch-context.ts")).unwrap();
    assert!(pi_launch_context.contains(env!("CARGO_BIN_EXE_nexus")));
    assert!(pi_launch_context.contains(persisted_config.to_str().unwrap()));
}

fn assert_session_end_works_from_another_directory(home: &Path, codex_home: &Path) {
    let session_end = settings(&codex_home.join("hooks.json"))["hooks"]["SessionEnd"][0]["hooks"]
        [0]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    let invocation = Command::new("/bin/sh")
        .args(["-c", &session_end])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("CODEX_HOME", codex_home)
        .env("NEXUS_STATE_DIR", home.join(".local/state/nexus"))
        .output()
        .unwrap();
    assert_success(&invocation);
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
    assert!(registrations.contains("codex mcp add nexus --"));
    assert!(registrations.contains("mcp --agent codex"));
    assert!(registrations.contains("claude mcp add --transport stdio --scope user nexus --"));
    assert!(registrations.contains("mcp --agent claude"));
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
        .env("XDG_CONFIG_HOME", home.join(".config"))
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
