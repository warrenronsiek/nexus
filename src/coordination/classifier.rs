// @feature coordination
// @spec docs/features/coordination.md
// @boundary dynamic-json
use super::domain::{Advisory, Claim, Operation, PathIntent, Severity};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const MAX_TOOL_PATH_INTENTS: usize = 256;
const MAX_TOOL_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_TOOL_PATCH_LINES: usize = 8_192;
const MAX_TOOL_PATH_BYTES: usize = 4_096;
const MAX_TOOL_PATH_AGGREGATE_BYTES: usize = 256 * 1024;
const MAX_SHELL_WORDS: usize = MAX_TOOL_PATH_INTENTS * 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub severity: Severity,
    pub kind: &'static str,
    pub message: String,
}

fn classify(intent: &PathIntent, existing: &Claim) -> Classification {
    if intent.operation.is_destructive() || existing.operation.is_destructive() {
        return Classification {
            severity: Severity::Critical,
            kind: "destructive_overlap",
            message: format!(
                "another session has an active {} claim on {}; proceeding may overwrite or remove its work",
                existing.operation, intent.path
            ),
        };
    }

    match (
        intent.line_start,
        intent.line_end,
        existing.line_start,
        existing.line_end,
    ) {
        (Some(a1), Some(a2), Some(b1), Some(b2)) if ranges_overlap(a1, a2, b1, b2) => {
            Classification {
                severity: Severity::Critical,
                kind: "hunk_overlap",
                message: format!(
                    "another session is editing overlapping lines {}-{} of {}",
                    b1, b2, intent.path
                ),
            }
        }
        (Some(_), Some(_), Some(_), Some(_)) => Classification {
            severity: Severity::Info,
            kind: "same_file_distinct_hunks",
            message: format!(
                "another session is editing a different known region of {}",
                intent.path
            ),
        },
        _ => Classification {
            severity: Severity::Warning,
            kind: "path_overlap",
            message: format!("another session has an active claim on {}", intent.path),
        },
    }
}

pub fn strongest_overlaps(intent: &PathIntent, claims: Vec<Claim>) -> Vec<(Claim, Classification)> {
    let mut by_session = BTreeMap::<String, (Claim, Classification)>::new();
    for claim in claims {
        let classification = classify(intent, &claim);
        let entry = by_session
            .entry(claim.session_id.clone())
            .or_insert_with(|| (claim.clone(), classification.clone()));
        if classification.severity > entry.1.severity {
            *entry = (claim, classification);
        }
    }
    by_session.into_values().collect()
}

pub fn advisory(classification: Classification, path: &str, other_session_id: &str) -> Advisory {
    Advisory {
        id: Uuid::new_v4().to_string(),
        severity: classification.severity,
        kind: classification.kind.to_owned(),
        message: classification.message,
        path: path.to_owned(),
        other_session_id: other_session_id.to_owned(),
        conflict_id: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolIntentExtractor {
    Patch,
    SinglePath(Operation),
    Edit,
    Shell,
    Unsupported,
}

impl ToolIntentExtractor {
    fn for_tool(tool_name: &str) -> Self {
        let name = tool_name.to_ascii_lowercase();
        if name.contains("apply_patch") || name == "applypatch" {
            Self::Patch
        } else if name == "write" || name.ends_with("__write") || name.contains("write_file") {
            Self::SinglePath(Operation::Write)
        } else if name == "edit" || name.ends_with("__edit") || name.contains("edit_file") {
            Self::Edit
        } else if name.contains("delete") || name.contains("remove_file") {
            Self::SinglePath(Operation::Delete)
        } else if name == "bash"
            || name == "shell"
            || name == "exec_command"
            || name.ends_with("__bash")
        {
            Self::Shell
        } else {
            Self::Unsupported
        }
    }

    fn extract(self, input: &Value) -> Vec<PathIntent> {
        match self {
            Self::Patch => extract_patch(input),
            Self::SinglePath(operation) => extract_single_path(input, operation),
            Self::Edit => extract_edit(input),
            Self::Shell => extract_shell(input),
            Self::Unsupported => Vec::new(),
        }
    }
}

#[derive(Default)]
struct BoundedPathIntents {
    intents: Vec<PathIntent>,
    identities: BTreeSet<(String, Operation, Option<u32>, Option<u32>)>,
    aggregate_path_bytes: usize,
}

impl BoundedPathIntents {
    fn normalize(intents: Vec<PathIntent>, project_root: Option<&str>) -> Vec<PathIntent> {
        let mut bounded = Self::default();
        for intent in intents.into_iter().take(MAX_TOOL_PATH_INTENTS) {
            bounded.push(intent, project_root);
        }
        bounded.intents
    }

    fn push(&mut self, mut intent: PathIntent, project_root: Option<&str>) {
        if !is_bounded_path(&intent.path) {
            return;
        }
        intent.path = canonical_claim_path(&intent.path, project_root);
        if !is_bounded_path(&intent.path) {
            return;
        }

        let identity = (
            intent.path.clone(),
            intent.operation,
            intent.line_start,
            intent.line_end,
        );
        if !self.identities.insert(identity) {
            return;
        }
        let Some(total) = self.aggregate_path_bytes.checked_add(intent.path.len()) else {
            return;
        };
        if total > MAX_TOOL_PATH_AGGREGATE_BYTES {
            return;
        }

        self.aggregate_path_bytes = total;
        self.intents.push(intent);
    }
}

mod execution;
pub use execution::extract_path_intents;
pub(crate) use execution::{bounded_reported_intents, shell_invocation};

fn is_bounded_path(path: &str) -> bool {
    !path.is_empty() && path.len() <= MAX_TOOL_PATH_BYTES
}

fn whole_path_intent(path: String, operation: Operation) -> PathIntent {
    PathIntent {
        path,
        operation,
        line_start: None,
        line_end: None,
    }
}

fn extract_single_path(input: &Value, operation: Operation) -> Vec<PathIntent> {
    path_value(input)
        .into_iter()
        .map(|path| whole_path_intent(path, operation))
        .collect()
}

fn extract_edit(input: &Value) -> Vec<PathIntent> {
    let Some(path) = path_value(input) else {
        return Vec::new();
    };
    let line_start = input
        .get("line_start")
        .or_else(|| input.get("start_line"))
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    let line_end = input
        .get("line_end")
        .or_else(|| input.get("end_line"))
        .and_then(Value::as_u64)
        .map(|n| n as u32)
        .or(line_start);
    vec![PathIntent {
        path,
        operation: Operation::Write,
        line_start,
        line_end,
    }]
}

fn path_value(input: &Value) -> Option<String> {
    ["file_path", "path", "file", "filename"]
        .into_iter()
        .find_map(|key| input.get(key).and_then(Value::as_str).map(str::to_owned))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchFile {
    path: String,
    operation: Operation,
    claims_whole_file: bool,
}

impl PatchFile {
    fn intent(&self, lines: (Option<u32>, Option<u32>)) -> PathIntent {
        PathIntent {
            path: self.path.clone(),
            operation: self.operation,
            line_start: lines.0,
            line_end: lines.1,
        }
    }
}

fn parse_patch_file(captures: &regex::Captures<'_>) -> Option<PatchFile> {
    let (operation, claims_whole_file) = match &captures[1] {
        "Update" => (Operation::Write, false),
        "Add" => (Operation::Write, true),
        "Delete" => (Operation::Delete, true),
        _ => return None,
    };
    let path = captures[2].trim();
    is_bounded_path(path).then(|| PatchFile {
        path: path.to_owned(),
        operation,
        claims_whole_file,
    })
}

fn parse_patch_hunk(captures: &regex::Captures<'_>) -> (Option<u32>, Option<u32>) {
    let start = captures[1].parse::<u32>().ok();
    let count = captures
        .get(2)
        .and_then(|value| value.as_str().parse::<u32>().ok())
        .unwrap_or(1);
    (
        start,
        start.map(|value| value.saturating_add(count.saturating_sub(1))),
    )
}

struct PatchScanner {
    file_header: Regex,
    hunk_header: Regex,
    current: Option<PatchFile>,
    intents: Vec<PathIntent>,
}

impl PatchScanner {
    fn new() -> Self {
        Self {
            file_header: Regex::new(r"^\*\*\* (Update|Add|Delete) File: (.+)$")
                .expect("valid patch file regex"),
            hunk_header: Regex::new(r"^@@\s+(?:-[0-9]+(?:,[0-9]+)?\s+)?\+([0-9]+)(?:,([0-9]+))?")
                .expect("valid patch hunk regex"),
            current: None,
            intents: Vec::new(),
        }
    }

    fn extract(mut self, patch: &str) -> Vec<PathIntent> {
        for line in patch.lines().take(MAX_TOOL_PATCH_LINES) {
            if self.intents.len() >= MAX_TOOL_PATH_INTENTS {
                break;
            }
            self.observe(line);
        }
        self.finish()
    }

    fn observe(&mut self, line: &str) {
        if let Some(captures) = self.file_header.captures(line) {
            self.current = parse_patch_file(&captures);
            if let Some(file) = self.current.as_ref() {
                if file.claims_whole_file {
                    self.intents.push(file.intent((None, None)));
                }
            }
            return;
        }
        let Some(captures) = self.hunk_header.captures(line) else {
            return;
        };
        if let Some(file) = self.current.as_ref() {
            self.intents.push(file.intent(parse_patch_hunk(&captures)));
        }
    }

    fn finish(mut self) -> Vec<PathIntent> {
        if self.intents.is_empty() {
            if let Some(file) = self.current {
                self.intents.push(file.intent((None, None)));
            }
        }
        self.intents
    }
}

fn extract_patch(input: &Value) -> Vec<PathIntent> {
    let patch = input
        .as_str()
        .or_else(|| input.get("patch").and_then(Value::as_str))
        .or_else(|| input.get("command").and_then(Value::as_str))
        .or_else(|| input.get("input").and_then(Value::as_str));
    let Some(patch) = patch else {
        return Vec::new();
    };
    if patch.len() > MAX_TOOL_PAYLOAD_BYTES {
        return Vec::new();
    }
    PatchScanner::new().extract(patch)
}

fn shell_command(input: &Value) -> Option<&str> {
    input
        .get("cmd")
        .or_else(|| input.get("command"))
        .and_then(Value::as_str)
        .or_else(|| input.as_str())
}

fn extract_shell(input: &Value) -> Vec<PathIntent> {
    let command = shell_command(input);
    let Some(command) = command else {
        return Vec::new();
    };
    if command.len() > MAX_TOOL_PAYLOAD_BYTES {
        return Vec::new();
    }
    if command.split_whitespace().take(MAX_SHELL_WORDS + 1).count() > MAX_SHELL_WORDS {
        return Vec::new();
    }
    let Ok(words) = shell_words::split(command) else {
        return Vec::new();
    };
    let Some(program) = words.first() else {
        return Vec::new();
    };

    match program.as_str() {
        "rm" | "unlink" => operand_intents(&words, Operation::Delete),
        "mv" => move_intents(&words),
        "touch" | "truncate" => operand_intents(&words, Operation::Write),
        _ => redirection_intents(&words),
    }
}

fn operand_intents(words: &[String], operation: Operation) -> Vec<PathIntent> {
    let mut intents = Vec::new();
    for path in words.iter().skip(1).filter(|word| !word.starts_with('-')) {
        if intents.len() >= MAX_TOOL_PATH_INTENTS {
            break;
        }
        intents.push(whole_path_intent(path.clone(), operation));
    }
    intents
}

fn move_intents(words: &[String]) -> Vec<PathIntent> {
    let mut paths = words.iter().skip(1).filter(|word| !word.starts_with('-'));
    let mut intents = Vec::new();
    if let Some(source) = paths.next() {
        intents.push(whole_path_intent(source.clone(), Operation::Rename));
    }
    if let Some(target) = paths.next() {
        intents.push(whole_path_intent(target.clone(), Operation::Write));
    }
    intents
}

fn redirection_intents(words: &[String]) -> Vec<PathIntent> {
    let mut intents = Vec::new();
    for pair in words.windows(2) {
        if pair[0] == ">" || pair[0] == ">>" {
            intents.push(whole_path_intent(pair[1].clone(), Operation::Write));
            if intents.len() >= MAX_TOOL_PATH_INTENTS {
                break;
            }
        }
    }
    intents
}

fn canonical_claim_path(path: &str, project_root: Option<&str>) -> String {
    let path = Path::new(path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(root) = project_root {
        Path::new(root).join(path)
    } else {
        path.to_path_buf()
    };
    let normalized = PathBuf::from(normalize_path(&joined.to_string_lossy()));
    project_root
        .and_then(|root| normalized.strip_prefix(Path::new(root)).ok())
        .map(|relative| normalize_path(&relative.to_string_lossy()))
        .unwrap_or_else(|| normalized.to_string_lossy().into_owned())
}

fn normalize_path(path: &str) -> String {
    let mut result = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result.to_string_lossy().into_owned()
}

fn ranges_overlap(a1: u32, a2: u32, b1: u32, b2: u32) -> bool {
    a1 <= b2 && b1 <= a2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::domain::ClaimState;
    use chrono::Utc;
    use serde_json::json;

    fn claim(start: Option<u32>, end: Option<u32>, operation: Operation) -> Claim {
        Claim {
            id: "c".into(),
            project_id: "p".into(),
            session_id: "other".into(),
            tool_use_id: "tool".into(),
            path: "/repo/a.rs".into(),
            operation,
            line_start: start,
            line_end: end,
            state: ClaimState::Claimed,
            expires_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn overlapping_hunks_are_critical() {
        let intent = PathIntent {
            path: "/repo/a.rs".into(),
            operation: Operation::Write,
            line_start: Some(10),
            line_end: Some(20),
        };
        let result = classify(&intent, &claim(Some(15), Some(30), Operation::Write));
        assert_eq!(result.severity, Severity::Critical);
        assert_eq!(result.kind, "hunk_overlap");
    }

    #[test]
    fn distinct_hunks_are_informational() {
        let intent = PathIntent {
            path: "/repo/a.rs".into(),
            operation: Operation::Write,
            line_start: Some(1),
            line_end: Some(2),
        };
        let result = classify(&intent, &claim(Some(20), Some(30), Operation::Write));
        assert_eq!(result.severity, Severity::Info);
    }

    #[test]
    fn keeps_only_the_strongest_overlap_per_session() {
        let intent = PathIntent {
            path: "/repo/a.rs".into(),
            operation: Operation::Write,
            line_start: Some(10),
            line_end: Some(20),
        };
        let overlaps = strongest_overlaps(
            &intent,
            vec![
                claim(Some(30), Some(40), Operation::Write),
                claim(Some(15), Some(25), Operation::Write),
            ],
        );

        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].1.severity, Severity::Critical);
        assert_eq!(overlaps[0].1.kind, "hunk_overlap");
    }

    #[test]
    fn extracts_apply_patch_hunks() {
        let value = json!({"patch": "*** Begin Patch\n*** Update File: src/a.rs\n@@ -8,2 +10,3 @@\n x\n*** End Patch"});
        let result = extract_path_intents("apply_patch", &value.into(), Some("/repo"));
        assert_eq!(
            result,
            vec![PathIntent {
                path: "src/a.rs".into(),
                operation: Operation::Write,
                line_start: Some(10),
                line_end: Some(12)
            }]
        );
    }

    #[test]
    fn extracts_codex_apply_patch_command_shape() {
        let value = json!({"command": "*** Begin Patch\n*** Update File: src/a.rs\n@@ -2,1 +2,1 @@\n-old\n+new\n*** End Patch"});
        let result = extract_path_intents("apply_patch", &value.into(), Some("/repo"));
        assert_eq!(result[0].path, "src/a.rs");
        assert_eq!(result[0].line_start, Some(2));
    }

    #[test]
    fn extracts_shell_delete() {
        let result = extract_path_intents(
            "Bash",
            &json!({"command":"rm -f src/a.rs"}).into(),
            Some("/repo"),
        );
        assert_eq!(result[0].operation, Operation::Delete);
        assert_eq!(result[0].path, "src/a.rs");
    }

    #[test]
    fn extracts_each_supported_shell_mutation_shape() {
        let cases = [
            (
                "mv -f src/old.rs src/new.rs",
                vec![
                    whole_path_intent("src/old.rs".into(), Operation::Rename),
                    whole_path_intent("src/new.rs".into(), Operation::Write),
                ],
            ),
            (
                "touch -c src/a.rs src/b.rs",
                vec![
                    whole_path_intent("src/a.rs".into(), Operation::Write),
                    whole_path_intent("src/b.rs".into(), Operation::Write),
                ],
            ),
            (
                "printf value > generated.txt >> appended.txt",
                vec![
                    whole_path_intent("generated.txt".into(), Operation::Write),
                    whole_path_intent("appended.txt".into(), Operation::Write),
                ],
            ),
        ];

        for (command, expected) in cases {
            let actual =
                extract_path_intents("Bash", &json!({"command": command}).into(), Some("/repo"));
            assert_eq!(actual, expected, "command: {command}");
        }
    }

    #[test]
    fn patch_file_actions_preserve_whole_file_and_fallback_claims() {
        let actions = json!({
            "patch": "*** Begin Patch\n*** Add File: src/new.rs\n+new\n*** Delete File: src/old.rs\n-old\n*** End Patch"
        });
        assert_eq!(
            extract_path_intents("apply_patch", &actions.into(), Some("/repo")),
            vec![
                whole_path_intent("src/new.rs".into(), Operation::Write),
                whole_path_intent("src/old.rs".into(), Operation::Delete),
            ]
        );

        let update_without_hunk = json!({"patch": "*** Update File: src/a.rs\n-old\n+new"});
        assert_eq!(
            extract_path_intents("apply_patch", &update_without_hunk.into(), Some("/repo")),
            vec![whole_path_intent("src/a.rs".into(), Operation::Write)]
        );
    }

    #[test]
    fn shell_classification_rejects_pathological_token_counts_before_parsing() {
        let command = format!("rm {}", "a ".repeat(500_000));
        assert!(command.len() <= MAX_TOOL_PAYLOAD_BYTES);

        let intents =
            extract_path_intents("Bash", &json!({"command":command}).into(), Some("/repo"));

        assert!(intents.is_empty());
    }

    #[test]
    fn tool_payloads_cannot_expand_into_unbounded_path_intents() {
        let patch = (0..(MAX_TOOL_PATH_INTENTS + 100))
            .map(|index| format!("*** Add File: generated/{index}.rs"))
            .collect::<Vec<_>>()
            .join("\n");

        let intents =
            extract_path_intents("apply_patch", &json!({"patch":patch}).into(), Some("/repo"));

        assert_eq!(intents.len(), MAX_TOOL_PATH_INTENTS);
    }

    #[test]
    fn patch_paths_are_bounded_before_hunk_expansion() {
        let long_path = "x".repeat(MAX_TOOL_PATH_BYTES + 1);
        let hunks = (0..MAX_TOOL_PATH_INTENTS)
            .map(|index| format!("@@ -1,1 +{},1 @@", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let patch = format!("*** Update File: {long_path}\n{hunks}");

        assert!(extract_patch(&json!({"patch": patch})).is_empty());
    }

    #[test]
    fn oversized_project_roots_are_ignored_before_canonicalization() {
        let oversized_root = format!("/{}/..", "r".repeat(MAX_TOOL_PATH_BYTES));
        let intents = extract_path_intents(
            "write",
            &json!({"path": "src/lib.rs"}).into(),
            Some(&oversized_root),
        );

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].path, "src/lib.rs");
    }
}
