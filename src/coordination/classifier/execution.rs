// @feature coordination
// @spec docs/features/coordination.md
use super::{is_bounded_path, BoundedPathIntents, ToolIntentExtractor, MAX_TOOL_PATH_INTENTS};
use crate::coordination::domain::{PathIntent, ToolPayload};

pub(crate) struct ShellInvocation<'a> {
    pub command: &'a str,
    pub directory: Option<&'a str>,
}

pub(crate) fn shell_invocation<'a>(
    tool_name: &str,
    input: &'a ToolPayload,
) -> Option<ShellInvocation<'a>> {
    if !matches!(
        ToolIntentExtractor::for_tool(tool_name),
        ToolIntentExtractor::Shell
    ) {
        return None;
    }
    Some(ShellInvocation {
        command: super::shell_command(input.as_json())?,
        directory: input.first_string(&["workdir", "cwd"]),
    })
}

pub fn extract_path_intents(
    tool_name: &str,
    input: &ToolPayload,
    project_root: Option<&str>,
) -> Vec<PathIntent> {
    let project_root = project_root.filter(|root| is_bounded_path(root));
    let extractor = ToolIntentExtractor::for_tool(tool_name);
    BoundedPathIntents::normalize(extractor.extract(input.as_json()), project_root)
}

pub(crate) fn bounded_reported_intents(
    reported: &[PathIntent],
    root: Option<&str>,
) -> Vec<PathIntent> {
    BoundedPathIntents::normalize(
        reported
            .iter()
            .take(MAX_TOOL_PATH_INTENTS)
            .cloned()
            .collect(),
        root,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_shell_metadata_accepts_object_aliases_and_bare_commands() {
        for payload in [
            json!({"cmd":"rm shared.txt","workdir":"/repo"}),
            json!({"command":"rm shared.txt","cwd":"/repo"}),
            json!("rm shared.txt"),
        ] {
            let input = ToolPayload::from(payload);
            let invocation = shell_invocation("Bash", &input).unwrap();
            assert_eq!(invocation.command, "rm shared.txt");
            assert_eq!(
                extract_path_intents("Bash", &input, None)[0].path,
                "shared.txt"
            );
        }
        assert!(shell_invocation("Read", &json!({"cmd":"rm shared.txt"}).into()).is_none());
    }
}
