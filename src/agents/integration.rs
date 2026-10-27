// @feature runtime
// @feature work-history
// @spec docs/features/work-history.md
// @feature agent-memory
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @entrypoint instructions
// @boundary dynamic-json
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Codex,
    Claude,
}

impl Host {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

pub fn instructions(host: Host, executable: &Path, explicit_config: Option<&Path>) -> Value {
    let invocation = shell_command(&mcp_server_arguments(host, executable, explicit_config));
    let registration_command = match host {
        Host::Codex => format!("codex mcp add nexus -- {invocation}"),
        Host::Claude => {
            format!("claude mcp add --transport stdio --scope user nexus -- {invocation}")
        }
    };
    let settings_path = match host {
        Host::Codex => "~/.codex/hooks.json",
        Host::Claude => "~/.claude/settings.json",
    };
    json!({
        "host":host.name(),
        "registration_command":registration_command,
        "hooks_file":settings_path,
        "settings_fragment":hook_configuration(host, executable, explicit_config),
        "note":"Merge the hooks object with existing settings. Every Nexus hook is advisory and returns no blocking decision."
    })
}

pub fn hook_configuration(host: Host, executable: &Path, explicit_config: Option<&Path>) -> Value {
    let agent = host.name();
    let turn_id = turn_id(host);
    let mut hooks = serde_json::Map::new();
    hooks.insert(
        "UserPromptSubmit".into(),
        json!([{"hooks":[user_prompt_hook(agent, turn_id)]}]),
    );
    hooks.insert(
        "PreToolUse".into(),
        json!([{"matcher":"*","hooks":[tool_hook("nexus_pre_tool_use", host, None)]}]),
    );
    hooks.insert(
        "PostToolUse".into(),
        json!([{"matcher":"*","hooks":[tool_hook("nexus_post_tool_use", host, None)]}]),
    );
    hooks.insert(
        "SessionEnd".into(),
        json!([{"hooks":[session_end_hook(executable, explicit_config, agent)]}]),
    );

    // Command adapters consume the native event envelope; they never return decisions.
    for event in [
        "SessionStart",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        if host == Host::Codex {
            "Interrupt"
        } else {
            "StopFailure"
        },
    ] {
        hooks.insert(
            event.into(),
            json!([{"hooks":[work_event_hook(executable, explicit_config, agent)]}]),
        );
    }
    hooks
        .get_mut("UserPromptSubmit")
        .expect("prompt hook exists")[0]["hooks"]
        .as_array_mut()
        .expect("hook array")
        .push(work_event_hook(executable, explicit_config, agent));

    if host == Host::Claude {
        hooks.insert(
            "PostToolUseFailure".into(),
            json!([{"matcher":"*","hooks":[tool_hook("nexus_post_tool_failure", host, Some(("error", "${error}")))]}]),
        );
    }
    json!({"hooks":hooks})
}

pub fn mcp_server_arguments(
    host: Host,
    executable: &Path,
    explicit_config: Option<&Path>,
) -> Vec<OsString> {
    let mut arguments = nexus_command_arguments(executable, explicit_config);
    arguments.extend([
        OsString::from("mcp"),
        OsString::from("--agent"),
        OsString::from(host.name()),
    ]);
    arguments
}

fn nexus_command_arguments(executable: &Path, explicit_config: Option<&Path>) -> Vec<OsString> {
    let mut arguments = vec![executable.as_os_str().to_owned()];
    if let Some(path) = explicit_config {
        arguments.extend([OsString::from("--config"), path.as_os_str().to_owned()]);
    }
    arguments
}

fn shell_command(arguments: &[OsString]) -> String {
    arguments
        .iter()
        .map(|argument| shell_words::quote(&argument.to_string_lossy()).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn turn_id(host: Host) -> &'static str {
    match host {
        Host::Codex => "${turn_id}",
        Host::Claude => "${prompt_id}",
    }
}

fn mcp_hook(tool: &str, input: Value) -> Value {
    json!({
        "type":"mcp_tool",
        "server":"nexus",
        "tool":tool,
        "input":input,
        "timeout":5,
        "statusMessage":"Checking coordination context"
    })
}

fn user_prompt_hook(agent: &str, turn_id: &str) -> Value {
    mcp_hook(
        "nexus_user_prompt",
        json!({
            "session_id":"${session_id}","project_root":"${cwd}","agent":agent,
            "turn_id":turn_id,"model":"${model}","prompt":"${prompt}"
        }),
    )
}

fn tool_hook(tool: &str, host: Host, completion: Option<(&str, &str)>) -> Value {
    let agent = host.name();
    let turn_id = turn_id(host);
    let mut input = json!({
        "session_id":"${session_id}","project_root":"${cwd}","agent":agent,
        "turn_id":turn_id,"model":"${model}","tool_use_id":"${tool_use_id}",
        "tool_name":"${tool_name}","tool_input":"${tool_input}"
    });
    if host == Host::Claude {
        input["agent_id"] = Value::String("${agent_id}".into());
        if completion.is_some() {
            input["is_interrupt"] = Value::String("${is_interrupt}".into());
        }
    }
    if let Some((field, value)) = completion {
        input
            .as_object_mut()
            .expect("tool hook input is an object")
            .insert(field.to_owned(), Value::String(value.to_owned()));
    }
    mcp_hook(tool, input)
}

fn session_end_hook(executable: &Path, explicit_config: Option<&Path>, agent: &str) -> Value {
    let mut arguments = nexus_command_arguments(executable, explicit_config);
    arguments.extend([
        OsString::from("hook-session-end"),
        OsString::from("--agent"),
        OsString::from(agent),
    ]);
    json!({
        "type":"command",
        "command":shell_command(&arguments),
        "timeout":3,
        "statusMessage":"Releasing coordination claims"
    })
}

fn work_event_hook(executable: &Path, explicit_config: Option<&Path>, agent: &str) -> Value {
    let mut arguments = nexus_command_arguments(executable, explicit_config);
    arguments
        .extend(["work", "host-event", "--host", agent, "--advisory", "-"].map(OsString::from));
    json!({"type":"command","command":shell_command(&arguments),"timeout":3,"statusMessage":"Recording observed work boundary"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_hooks_call_nexus_without_decision_fields() {
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"), None);
            let encoded = serde_json::to_string(&generated).unwrap();
            assert!(encoded.contains("nexus_pre_tool_use"));
            assert!(encoded.contains("${tool_input}"));
            assert!(!encoded.contains("permissionDecision"));
            assert!(!encoded.contains("decision"));
        }
    }

    #[test]
    fn every_generated_hook_timeout_exceeds_the_lifecycle_response_budget() {
        let budget = crate::runtime::client::LIFECYCLE_RESPONSE_BUDGET.as_secs_f64();
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"), None);
            for (event, groups) in generated["hooks"].as_object().unwrap() {
                for hook in groups[0]["hooks"].as_array().unwrap() {
                    let timeout = hook["timeout"].as_f64().unwrap();
                    assert!(timeout > budget, "{event} timeout {timeout}s <= {budget}s");
                }
            }
        }
    }

    #[test]
    fn all_hosts_use_command_hooks_for_session_end() {
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"), None);
            let hook = &generated["hooks"]["SessionEnd"][0]["hooks"][0];
            assert_eq!(hook["type"], "command");
            assert!(hook["command"]
                .as_str()
                .unwrap()
                .contains("hook-session-end"));
        }
        assert!(
            hook_configuration(Host::Claude, Path::new("/opt/nexus"), None)["hooks"]
                ["PostToolUseFailure"]
                .is_array()
        );
    }

    #[test]
    fn generated_tool_hooks_forward_usage_correlation_metadata() {
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"), None);
            for event in ["PreToolUse", "PostToolUse"] {
                let input = &generated["hooks"][event][0]["hooks"][0]["input"];
                assert!(input["turn_id"].is_string(), "{host:?} {event}");
                assert!(input["model"].is_string(), "{host:?} {event}");
            }
        }
    }

    #[test]
    fn host_registration_attributes_simple_memory_writes_without_session_correlation() {
        for host in [Host::Codex, Host::Claude] {
            let generated = instructions(host, Path::new("/opt/nexus"), None);
            let command = generated["registration_command"].as_str().unwrap();
            assert!(
                command.contains(&format!("mcp --agent {}", host.name())),
                "{command}"
            );
        }
    }

    #[test]
    fn explicit_configuration_is_preserved_in_registered_and_session_end_commands() {
        let generated = instructions(
            Host::Codex,
            Path::new("/opt/nexus binary"),
            Some(Path::new("/tmp/nexus config.toml")),
        );
        let registration = generated["registration_command"].as_str().unwrap();
        let session_end = generated["settings_fragment"]["hooks"]["SessionEnd"][0]["hooks"][0]
            ["command"]
            .as_str()
            .unwrap();

        for command in [registration, session_end] {
            assert!(command.contains("'/opt/nexus binary'"), "{command}");
            assert!(
                command.contains("--config '/tmp/nexus config.toml'"),
                "{command}"
            );
        }
        assert!(registration.ends_with("mcp --agent codex"));
        assert!(session_end.ends_with("hook-session-end --agent codex"));
    }
}
