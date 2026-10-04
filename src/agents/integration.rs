// @feature runtime
// @spec docs/features/runtime.md
// @entrypoint instructions
// @boundary dynamic-json
use serde_json::{json, Value};
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

pub fn instructions(host: Host, executable: &Path) -> Value {
    let executable_text = executable.to_string_lossy();
    let quoted = shell_words::quote(&executable_text);
    let registration_command = match host {
        Host::Codex => format!("codex mcp add nexus -- {quoted} mcp"),
        Host::Claude => {
            format!("claude mcp add --transport stdio --scope user nexus -- {quoted} mcp")
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
        "settings_fragment":hook_configuration(host, executable),
        "note":"Merge the hooks object with existing settings. Every Nexus hook is advisory and returns no blocking decision."
    })
}

pub fn hook_configuration(host: Host, executable: &Path) -> Value {
    let agent = host.name();
    let turn_id = turn_id(host);
    let executable_text = executable.to_string_lossy();
    let executable = shell_words::quote(&executable_text);
    let mut hooks = serde_json::Map::new();
    hooks.insert(
        "UserPromptSubmit".into(),
        json!([{"hooks":[user_prompt_hook(agent, turn_id)]}]),
    );
    hooks.insert(
        "PreToolUse".into(),
        json!([{"matcher":"*","hooks":[tool_hook("nexus_pre_tool_use", agent, turn_id, None)]}]),
    );
    hooks.insert(
        "PostToolUse".into(),
        json!([{"matcher":"*","hooks":[tool_hook("nexus_post_tool_use", agent, turn_id, Some(("tool_output", "${tool_response}")))]}]),
    );
    hooks.insert(
        "SessionEnd".into(),
        json!([{"hooks":[session_end_hook(&executable, agent)]}]),
    );

    if host == Host::Claude {
        hooks.insert(
            "PostToolUseFailure".into(),
            json!([{"matcher":"*","hooks":[tool_hook("nexus_post_tool_failure", agent, turn_id, Some(("error", "${error}")))]}]),
        );
    }
    json!({"hooks":hooks})
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

fn tool_hook(tool: &str, agent: &str, turn_id: &str, completion: Option<(&str, &str)>) -> Value {
    let mut input = json!({
        "session_id":"${session_id}","project_root":"${cwd}","agent":agent,
        "turn_id":turn_id,"model":"${model}","tool_use_id":"${tool_use_id}",
        "tool_name":"${tool_name}","tool_input":"${tool_input}"
    });
    if let Some((field, value)) = completion {
        input
            .as_object_mut()
            .expect("tool hook input is an object")
            .insert(field.to_owned(), Value::String(value.to_owned()));
    }
    mcp_hook(tool, input)
}

fn session_end_hook(executable: &str, agent: &str) -> Value {
    json!({
        "type":"command",
        "command":format!("{executable} hook-session-end --agent {agent}"),
        "timeout":3,
        "statusMessage":"Releasing coordination claims"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_hooks_call_nexus_without_decision_fields() {
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"));
            let encoded = serde_json::to_string(&generated).unwrap();
            assert!(encoded.contains("nexus_pre_tool_use"));
            assert!(encoded.contains("${tool_input}"));
            assert!(!encoded.contains("permissionDecision"));
            assert!(!encoded.contains("decision"));
        }
    }

    #[test]
    fn every_generated_hook_timeout_exceeds_the_lifecycle_response_budget() {
        let budget = crate::runtime::daemon::LIFECYCLE_RESPONSE_BUDGET.as_secs_f64();
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"));
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
            let generated = hook_configuration(host, Path::new("/opt/nexus"));
            let hook = &generated["hooks"]["SessionEnd"][0]["hooks"][0];
            assert_eq!(hook["type"], "command");
            assert!(hook["command"]
                .as_str()
                .unwrap()
                .contains("hook-session-end"));
        }
        assert!(
            hook_configuration(Host::Claude, Path::new("/opt/nexus"))["hooks"]
                ["PostToolUseFailure"]
                .is_array()
        );
    }

    #[test]
    fn generated_tool_hooks_forward_usage_correlation_metadata() {
        for host in [Host::Codex, Host::Claude] {
            let generated = hook_configuration(host, Path::new("/opt/nexus"));
            for event in ["PreToolUse", "PostToolUse"] {
                let input = &generated["hooks"][event][0]["hooks"][0]["input"];
                assert!(input["turn_id"].is_string(), "{host:?} {event}");
                assert!(input["model"].is_string(), "{host:?} {event}");
            }
        }
    }
}
