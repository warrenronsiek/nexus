// @feature work-history
// @spec docs/features/work-history.md
// @feature installation
// @feature code-concerns
// @spec docs/features/code-concerns.md
// @feature sos
// @feature usage-analytics
// @feature agent-memory
// @feature evals
// @spec docs/features/installation.md
// @spec docs/features/sos.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/evals.md
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
    for eval_module in [
        "evals.ts",
        "evals-pane.ts",
        "evals-view.ts",
        "evals-state.ts",
        "evals-requests.ts",
    ] {
        assert!(
            home.join(".pi/agent/extensions/nexus")
                .join(eval_module)
                .is_file(),
            "machine setup did not install {eval_module}"
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
fn machine_setup_installs_feature_skills_and_refreshes_bundled_content() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let command_directory = temporary.path().join("commands");
    let host_log = temporary.path().join("hosts.log");
    fs::create_dir_all(&command_directory).unwrap();
    fake_host(&command_directory, "codex");
    let names = [
        "nexus",
        "nexus-setup",
        "nexus-coordination",
        "nexus-agent-memory",
        "nexus-papercuts",
        "nexus-sos",
        "nexus-code-concerns",
        "nexus-usage-analytics",
        "nexus-observability",
        "nexus-development",
        "nexus-build-eval",
        "nexus-eval-audit",
        "nexus-hillclimb",
        "nexus-eval-history",
        "nexus-work-history",
    ];
    for root in [&codex_home, &home.join(".claude")] {
        for name in ["nexus", "nexus-build-eval", "nexus-work-history"] {
            let skill = root.join("skills").join(name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), "stale bundled content").unwrap();
            fs::write(skill.join("personal-notes.md"), "keep me").unwrap();
        }
    }

    for _ in 0..2 {
        let setup = nexus_command(&home, &codex_home, &command_directory, &host_log)
            .arg("setup")
            .output()
            .unwrap();
        assert_success(&setup);
        let report: Value = serde_json::from_slice(&setup.stdout).unwrap();
        for name in names {
            assert!(
                report["skills"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == name),
                "setup did not report {name}"
            );
        }
    }
    for root in [&codex_home, &home.join(".claude")] {
        assert_installed_skills(root, &names);
    }
}

fn assert_installed_skills(root: &Path, names: &[&str]) {
    for name in names {
        let path = root.join("skills").join(name).join("SKILL.md");
        let source = fs::read_to_string(&path).unwrap();
        assert!(source.starts_with("---\nname: "), "{}", path.display());
    }
    assert!(root
        .join("skills/nexus-eval-history/references/nexus-evals.md")
        .is_file());
    for name in ["nexus", "nexus-build-eval", "nexus-work-history"] {
        assert_eq!(
            fs::read_to_string(root.join("skills").join(name).join("personal-notes.md")).unwrap(),
            "keep me"
        );
    }
    assert_installed_skill_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("skills"),
        &root.join("skills"),
    );
    assert_work_history_resources(root);
}

fn assert_installed_skill_tree(source: &Path, installed: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let source_path = entry.unwrap().path();
        let target = installed.join(source_path.file_name().unwrap());
        if source_path.is_dir() {
            assert_installed_skill_tree(&source_path, &target);
        } else {
            assert_eq!(
                fs::read(&target).unwrap_or_else(|error| panic!("{}: {error}", target.display())),
                fs::read(&source_path).unwrap(),
                "installed resource differs: {}",
                target.display()
            );
        }
    }
}

fn assert_work_history_resources(root: &Path) {
    let skill = root.join("skills/nexus-work-history");
    assert!(skill.join("references/integration.md").is_file());
    let workload = root.join("workload.sh");
    let marker = root.join("executions");
    fs::write(
        &workload,
        "printf 'executed\\n' >> \"$1\"\nprintf 'preserved\\n'\nexit 7\n",
    )
    .unwrap();
    let output = Command::new("/bin/bash")
        .args([
            "-uc",
            "source \"$1\"; nexus_work_exec test fixture /bin/sh \"$2\" \"$3\"",
            "skill-test",
        ])
        .arg(skill.join("scripts/work-capture.sh"))
        .arg(workload)
        .arg(&marker)
        .env("NEXUS_WORK_BINARY", root.join("missing-nexus"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(output.stdout, b"preserved\n");
    assert_eq!(fs::read(marker).unwrap(), b"executed\n");
    assert_success(
        &Command::new("python3")
            .arg(skill.join("scripts/nexus-work-github.py"))
            .arg("--help")
            .output()
            .unwrap(),
    );
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
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
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
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
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

#[test]
fn machine_setup_refreshes_waterfall_helper_modules() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let commands = temporary.path().join("commands");
    let installed = home.join(".pi/agent/extensions/nexus");
    fs::create_dir_all(&commands).unwrap();
    fs::create_dir_all(&installed).unwrap();
    fake_host(&commands, "codex");
    fake_host(&commands, "pi");
    let helpers = [
        (
            "work-activity-style.ts",
            include_str!("../pi-extension/src/work-activity-style.ts"),
        ),
        (
            "work-waterfall-metadata.ts",
            include_str!("../pi-extension/src/work-waterfall-metadata.ts"),
        ),
    ];
    for (module, _) in helpers {
        fs::write(installed.join(module), "stale installed helper").unwrap();
    }
    let setup = nexus_command(
        &home,
        &codex_home,
        &commands,
        &temporary.path().join("hosts.log"),
    )
    .arg("setup")
    .env("PATH", &commands)
    .output()
    .unwrap();
    assert_success(&setup);
    for (module, source) in helpers {
        assert_eq!(fs::read_to_string(installed.join(module)).unwrap(), source);
    }
}

#[test]
fn installed_pi_extension_contains_its_complete_local_module_dependency_closure() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let codex_home = home.join(".codex");
    let commands = temporary.path().join("commands");
    fs::create_dir_all(&commands).unwrap();
    for host in ["codex", "claude", "pi"] {
        fake_host(&commands, host);
    }
    let setup = nexus_command(
        &home,
        &codex_home,
        &commands,
        &temporary.path().join("hosts.log"),
    )
    .arg("setup")
    .output()
    .unwrap();
    assert_success(&setup);
    let root = home.join(".pi/agent/extensions/nexus");
    let imports = regex::Regex::new(r#"(?:from\s*|import\s*)["'](\./[^"']+\.ts)["']"#).unwrap();
    let mut pending = vec![String::from("index.ts")];
    let mut visited = std::collections::HashSet::new();
    while let Some(module) = pending.pop() {
        if !visited.insert(module.clone()) {
            continue;
        }
        let source = fs::read_to_string(root.join(&module)).unwrap_or_else(|error| {
            panic!("installed Pi extension is missing local dependency {module}: {error}")
        });
        for import in imports.captures_iter(&source) {
            pending.push(import[1].trim_start_matches("./").to_owned());
        }
    }
    assert!(
        visited.contains("work-pane.ts"),
        "the installed entry point must reach Work"
    );
    assert!(
        visited.contains("concerns-pane.ts"),
        "the installed entry point must reach Code concerns"
    );
}
