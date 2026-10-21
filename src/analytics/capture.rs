// @entrypoint resolve
// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use super::{identity, types::*};
use crate::coordination::domain::ToolHookInput;

pub fn resolve(input: &ToolHookInput, scripts: &[String]) -> Vec<Action> {
    let executor = executor_kind(&input.tool_name);
    let command = executor.and_then(|_| {
        input
            .tool_input
            .first_string(&["cmd", "command", "code", "script"])
    });
    if executor.is_none() {
        return vec![fallback(input, None, None)];
    }
    let mut actions = Vec::new();
    if let Some(command) = command {
        for words in crate::coordination::usage_detection::command_words(command) {
            if named_script_command(&words, scripts) {
                continue;
            }
            if let Some(action) = cli_action(&words) {
                actions.push(action);
            }
            if actions.len() >= MAX_ACTIONS {
                break;
            }
        }
    }
    for script in scripts
        .iter()
        .take(MAX_ACTIONS.saturating_sub(actions.len()))
    {
        actions.push(Action {
            name: script.clone(),
            kind: ResourceKind::Script,
            origin: Origin::Unknown,
            category_id: "unclassified".into(),
            description: format!("Run repository script {script}"),
            pattern_key: identity(&[POLICY_VERSION, script]),
            model_input: None,
        });
    }
    if actions.is_empty() {
        actions.push(fallback(input, command, executor));
    }
    actions
}

pub fn version_scripts(
    actions: &mut [Action],
    versions: &std::collections::BTreeMap<String, String>,
) {
    for action in actions
        .iter_mut()
        .filter(|action| action.kind == ResourceKind::Script)
    {
        // Unknown versions remain a distinct, uncacheable pattern.
        action.pattern_key = identity(&[
            POLICY_VERSION,
            &action.name,
            versions
                .get(&action.name)
                .map(String::as_str)
                .unwrap_or("unknown-version"),
        ]);
    }
}
fn script_path(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = root.join(name).canonicalize().ok()?;
    (path.starts_with(&root) && path.is_file()).then_some(path)
}
pub fn script_version(root: &std::path::Path, name: &str) -> Option<String> {
    let metadata = std::fs::metadata(script_path(root, name)?).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(identity(&[
        &metadata.len().to_string(),
        &modified.as_nanos().to_string(),
    ]))
}
pub fn script_input(root: &std::path::Path, name: &str) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(script_path(root, name)?).ok()?;
    let mut bytes = Vec::new();
    file.take((MAX_CLASSIFIER_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    let source = std::str::from_utf8(&bytes).ok()?;
    super::classification::redact(source)
}

#[derive(Clone, Copy)]
enum Executor {
    Shell,
    Program,
}
fn executor_kind(name: &str) -> Option<Executor> {
    let name = name
        .rsplit(['.', ':'])
        .next()?
        .rsplit("__")
        .next()?
        .to_ascii_lowercase();
    match name.as_str() {
        "bash" | "sh" | "zsh" | "fish" | "shell" | "terminal" | "exec_command"
        | "shell_command" | "run_command" | "run_shell" | "execute_command" => {
            Some(Executor::Shell)
        }
        "exec" | "script" | "python" | "python3" | "node" | "javascript" | "js" | "ruby"
        | "perl" => Some(Executor::Program),
        _ => None,
    }
}
fn named_script_command(words: &[String], scripts: &[String]) -> bool {
    words.first().is_some_and(|command| {
        command.contains('/')
            && scripts
                .iter()
                .any(|script| std::path::Path::new(command).ends_with(script))
    })
}
struct Delivery {
    name: String,
    kind: ResourceKind,
    origin: Origin,
}
fn inline_delivery(pattern: &str, origin: Origin) -> Delivery {
    Delivery {
        name: format!("Inline program #{}", &pattern[..8]),
        kind: ResourceKind::Inline,
        origin,
    }
}
fn delivery(
    input: &ToolHookInput,
    command: Option<&str>,
    executor: Option<Executor>,
    pattern: &str,
) -> Delivery {
    match (command, executor) {
        (Some(command), Some(Executor::Shell)) => shell_delivery(command, pattern),
        (Some(_), Some(Executor::Program)) => inline_delivery(pattern, Origin::AgentAuthored),
        _ => Delivery {
            name: input.tool_name.clone(),
            kind: ResourceKind::Tool,
            origin: Origin::HostTool,
        },
    }
}
fn shell_delivery(command: &str, pattern: &str) -> Delivery {
    let segments = crate::coordination::usage_detection::command_words(command);
    let Some(words) = segments.first() else {
        return inline_delivery(pattern, Origin::Unknown);
    };
    if inline_shell(words) {
        return inline_delivery(pattern, Origin::AgentAuthored);
    }
    let program = words
        .first()
        .and_then(|word| std::path::Path::new(word).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let executable = !program.is_empty()
        && program.len() <= 64
        && program
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if !executable {
        return inline_delivery(pattern, Origin::Unknown);
    }
    let known =
        CLI_RULES.iter().any(|rule| rule.0.contains(&program)) || EXTRA_CLIS.contains(&program);
    Delivery {
        name: program.into(),
        kind: ResourceKind::Cli,
        origin: if known {
            Origin::StandardCli
        } else {
            Origin::Unknown
        },
    }
}
fn inline_shell(words: &[String]) -> bool {
    let program = words.first().map(String::as_str).unwrap_or("");
    let control = [
        "if", "for", "while", "until", "case", "function", "select", "{", "(",
    ];
    let interpreter = [
        "python", "python3", "node", "ruby", "perl", "bash", "sh", "zsh",
    ];
    control.contains(&program)
        || (interpreter.contains(&program)
            && words.iter().skip(1).any(|word| {
                matches!(word.as_str(), "-c" | "-e" | "--eval" | "-") || word.starts_with("<<")
            }))
}
const EXTRA_CLIS: &[&str] = &[
    "nexus", "npm", "pnpm", "yarn", "make", "just", "python", "python3", "node", "ruby", "perl",
    "uv", "pip", "pip3", "go", "rustc", "awk", "black", "prettier", "eslint", "tsc", "deno", "bun",
    "echo", "pwd", "cd", "sleep",
];
fn operation_arguments<'a>(program: &str, mut args: &'a [String]) -> &'a [String] {
    if program == "git" {
        while let Some(flag) = args.first().map(String::as_str) {
            let consumed = match flag {
                "-C" | "-c" | "--git-dir" | "--work-tree" | "--config-env" => 2,
                "--no-pager" | "--paginate" | "--bare" | "--no-optional-locks" => 1,
                _ if flag.starts_with("--git-dir=") || flag.starts_with("--work-tree=") => 1,
                _ => break,
            };
            args = args.get(consumed..).unwrap_or_default();
        }
    }
    args
}

fn fallback(input: &ToolHookInput, command: Option<&str>, executor: Option<Executor>) -> Action {
    let name = input.tool_name.to_ascii_lowercase();
    let category = match name.rsplit(['.', ':']).next().unwrap_or(&name) {
        "read" | "read_file" => "files.read",
        "write" | "write_file" | "edit" | "edit_file" | "apply_patch" => "files.edit",
        "sleep" | "clocksleep" => "agent.wait",
        "wait_agent" | "collaborationwait_agent" => "agent.coordinate",
        _ => "unclassified",
    };
    let sanitized = command
        .and_then(inline_source)
        .and_then(|source| super::classification::redact(&source));
    let pattern = identity(&[
        POLICY_VERSION,
        &input.tool_name,
        sanitized.as_deref().unwrap_or(""),
    ]);
    let delivery = delivery(input, command, executor, &pattern);
    Action {
        name: delivery.name,
        kind: delivery.kind,
        origin: delivery.origin,
        category_id: category.into(),
        description: if category == "unclassified" {
            "Activity not yet classified".into()
        } else {
            category.replace('.', " ")
        },
        pattern_key: pattern,
        model_input: if category != "unclassified" {
            None
        } else {
            sanitized.or_else(|| {
                super::classification::redact(&format!("Named host tool: {}", input.tool_name))
            })
        },
    }
}

// Shell quoting encloses the program; redact the program's literals, not the enclosure.
fn inline_source(command: &str) -> Option<String> {
    let words = shell_words::split(command).ok()?;
    let program = std::path::Path::new(words.first()?).file_name()?.to_str()?;
    let interpreters = [
        "python", "python3", "node", "ruby", "perl", "bash", "sh", "zsh",
    ];
    if interpreters.contains(&program) {
        if let Some(index) = words
            .iter()
            .position(|word| matches!(word.as_str(), "-c" | "-e" | "--eval"))
        {
            return words
                .get(index + 1)
                .map(|source| format!("{program} program:\n{source}"));
        }
    }
    Some(command.into())
}

fn cli_action(words: &[String]) -> Option<Action> {
    let executable = std::path::Path::new(words.first()?).file_name()?.to_str()?;
    let args = operation_arguments(executable, words.get(1..).unwrap_or_default());
    let (operation, category) = cli_operation(executable, args)?;
    let name = if operation.is_empty() {
        executable.into()
    } else {
        format!("{executable} {operation}")
    };
    Some(Action {
        pattern_key: identity(&[POLICY_VERSION, &name]),
        name,
        kind: ResourceKind::Cli,
        origin: Origin::StandardCli,
        category_id: category.into(),
        description: category.replace('.', " "),
        model_input: None,
    })
}

fn cli_operation(program: &str, args: &[String]) -> Option<(String, &'static str)> {
    let first = args.first().map(String::as_str).unwrap_or("");
    if program == "gh" {
        return github_operation(args);
    }
    if program == "sed" {
        return Some((
            String::new(),
            if args.iter().any(|arg| arg == "-i") {
                "files.edit"
            } else {
                "files.read"
            },
        ));
    }
    let rule = CLI_RULES.iter().find(|(programs, verbs, _)| {
        programs.contains(&program) && (verbs.is_empty() || verbs.contains(&first))
    })?;
    Some((
        if rule.1.is_empty() {
            String::new()
        } else {
            first.into()
        },
        rule.2,
    ))
}
type CliRule = (
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
);
const CLI_RULES: &[CliRule] = &[
    (&["npm", "yarn", "pnpm"], &["test"], "validation.test"),
    (
        &["git"],
        &[
            "status",
            "diff",
            "log",
            "show",
            "branch",
            "rev-parse",
            "ls-files",
        ],
        "vcs.inspect",
    ),
    (&["git"], &["fetch", "pull", "push"], "vcs.sync"),
    (
        &["git"],
        &[
            "add",
            "commit",
            "merge",
            "rebase",
            "reset",
            "restore",
            "checkout",
            "switch",
            "cherry-pick",
        ],
        "vcs.change",
    ),
    (&["cargo"], &["test"], "validation.test"),
    (&["cargo"], &["check"], "validation.typecheck"),
    (&["cargo"], &["build"], "build.compile"),
    (&["cargo"], &["clippy"], "validation.lint"),
    (&["cargo"], &["fmt"], "source.format"),
    (&["rg", "grep", "find", "fd"], &[], "code.search"),
    (
        &["cat", "head", "tail", "ls", "stat", "wc"],
        &[],
        "files.read",
    ),
    (&["pytest"], &[], "validation.test"),
    (&["ruff"], &[], "validation.lint"),
    (&["curl", "wget"], &[], "external.http"),
    (&["jq"], &[], "data.transform"),
    (
        &["docker", "kubectl"],
        &["ps", "images", "logs", "inspect", "get", "describe"],
        "infrastructure.inspect",
    ),
    (
        &["docker", "kubectl"],
        &[
            "build", "compose", "run", "start", "stop", "apply", "delete",
        ],
        "infrastructure.change",
    ),
];
fn github_operation(args: &[String]) -> Option<(String, &'static str)> {
    let family = args.first()?.as_str();
    let verb = args.get(1).map(String::as_str).unwrap_or("");
    let category = match (family, verb) {
        ("pr", "checks") | ("run", "view" | "list" | "watch") => "ci.inspect",
        ("pr", "view" | "list" | "diff") => "vcs.review",
        ("pr", "create" | "edit" | "merge" | "close") => "vcs.change",
        ("issue", "view" | "list") => "issues.inspect",
        ("issue", "create" | "edit" | "close") => "issues.change",
        ("api", _) => return Some(("api".into(), "external.api")),
        _ => return None,
    };
    Some((format!("{family} {verb}"), category))
}

pub fn known_categories() -> Vec<Category> {
    let mut ids: Vec<&str> = CLI_RULES
        .iter()
        .map(|rule| rule.2)
        .chain([
            "ci.inspect",
            "vcs.review",
            "issues.inspect",
            "issues.change",
            "external.api",
            "files.edit",
            "agent.wait",
            "agent.coordinate",
        ])
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let name = id.replace('.', " ");
            Category {
                id: id.into(),
                definition: format!("Agent operations that perform {name}"),
                name,
            }
        })
        .collect()
}

pub fn known_literal_operations(source: &str) -> Option<String> {
    let operations: Vec<_> = crate::coordination::usage_detection::command_words(source)
        .iter()
        .filter_map(|words| cli_action(words).map(|action| action.name))
        .collect();
    (!operations.is_empty()).then(|| operations.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn inline_interpreter_source_is_described_instead_of_its_shell_quoting() {
        let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","tool_use_id":"t","tool_name":"bash","tool_input":{"command":"python -c 'import requests; requests.get(endpoint)'"}})).unwrap();
        let action = resolve(&input, &[]).remove(0);
        let source = action.model_input.unwrap();
        assert!(source.contains("requests.get"), "{source}");
        assert!(!source.contains("private.example"), "{source}");
    }
    #[test]
    fn ordinary_cli_calls_are_not_agent_authored_programs() {
        for (command, name, origin) in [
            ("npm test", "npm test", Origin::StandardCli),
            (
                "git -C /private/checkout status",
                "git status",
                Origin::StandardCli,
            ),
            ("vendor_command inspect", "vendor_command", Origin::Unknown),
        ] {
            let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","tool_use_id":"t","tool_name":"Bash","tool_input":{"command":command}})).unwrap();
            let action = resolve(&input, &[]).remove(0);
            assert_eq!(action.origin, origin, "{command}");
            assert_eq!(action.name, name, "{command}");
            assert_eq!(action.kind, ResourceKind::Cli);
        }
    }
    #[test]
    fn named_host_tools_retain_their_identity_when_they_accept_code_arguments() {
        let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","tool_use_id":"t","tool_name":"database.query","tool_input":{"command":"SELECT private_value FROM private_table"}})).unwrap();
        let action = resolve(&input, &[]).remove(0);
        assert_eq!(action.name, "database.query");
        assert_eq!(action.origin, Origin::HostTool);
    }
    #[test]
    fn a_repository_script_named_like_a_cli_does_not_create_a_second_cli_operation() {
        let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","tool_use_id":"t","tool_name":"Bash","tool_input":{"command":"./scripts/cargo build"}})).unwrap();
        let actions = resolve(&input, &["scripts/cargo".into()]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "scripts/cargo");
    }
    #[test]
    fn generic_script_tools_classify_their_program_instead_of_only_the_wrapper_name() {
        for field in ["code", "script"] {
            let input:ToolHookInput=serde_json::from_value(json!({"session_id":"s","tool_use_id":"t","tool_name":"script","tool_input":{(field):"requests.get(endpoint)"}})).unwrap();
            let action = resolve(&input, &[]).remove(0);
            assert_eq!(action.kind, ResourceKind::Inline);
            assert!(action.model_input.unwrap().contains("requests.get"));
        }
    }
}
