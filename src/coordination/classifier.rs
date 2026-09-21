// @feature coordination
// @spec docs/features/coordination.md
// @boundary dynamic-json
use super::domain::{Advisory, Claim, Operation, PathIntent, Severity, ToolPayload};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

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

pub fn extract_path_intents(
    tool_name: &str,
    input: &ToolPayload,
    project_root: Option<&str>,
) -> Vec<PathIntent> {
    let input = input.as_json();
    let lower = tool_name.to_ascii_lowercase();
    let mut intents = if lower.contains("apply_patch") || lower == "applypatch" {
        extract_patch(input)
    } else if lower == "write" || lower.ends_with("__write") || lower.contains("write_file") {
        extract_single_path(input, Operation::Write)
    } else if lower == "edit" || lower.ends_with("__edit") || lower.contains("edit_file") {
        extract_edit(input)
    } else if lower.contains("delete") || lower.contains("remove_file") {
        extract_single_path(input, Operation::Delete)
    } else if lower == "bash"
        || lower == "shell"
        || lower == "exec_command"
        || lower.ends_with("__bash")
    {
        extract_shell(input)
    } else {
        Vec::new()
    };

    for intent in &mut intents {
        intent.path = canonical_claim_path(&intent.path, project_root);
    }
    let mut seen = BTreeSet::new();
    intents.retain(|intent| {
        seen.insert((
            intent.path.clone(),
            intent.operation,
            intent.line_start,
            intent.line_end,
        ))
    });
    intents
}

fn extract_single_path(input: &Value, operation: Operation) -> Vec<PathIntent> {
    path_value(input)
        .into_iter()
        .map(|path| PathIntent {
            path,
            operation,
            line_start: None,
            line_end: None,
        })
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

fn extract_patch(input: &Value) -> Vec<PathIntent> {
    let patch = input
        .as_str()
        .or_else(|| input.get("patch").and_then(Value::as_str))
        .or_else(|| input.get("command").and_then(Value::as_str))
        .or_else(|| input.get("input").and_then(Value::as_str));
    let Some(patch) = patch else {
        return Vec::new();
    };
    let file_re = Regex::new(r"^\*\*\* (Update|Add|Delete) File: (.+)$").unwrap();
    let hunk_re = Regex::new(r"^@@\s+(?:-[0-9]+(?:,[0-9]+)?\s+)?\+([0-9]+)(?:,([0-9]+))?").unwrap();
    let mut intents = Vec::new();
    let mut current: Option<(String, Operation)> = None;
    for line in patch.lines() {
        if let Some(captures) = file_re.captures(line) {
            let op = match &captures[1] {
                "Delete" => Operation::Delete,
                _ => Operation::Write,
            };
            current = Some((captures[2].trim().to_owned(), op));
            if op == Operation::Delete || &captures[1] == "Add" {
                intents.push(PathIntent {
                    path: captures[2].trim().to_owned(),
                    operation: op,
                    line_start: None,
                    line_end: None,
                });
            }
        } else if let (Some(captures), Some((path, operation))) =
            (hunk_re.captures(line), current.as_ref())
        {
            let start = captures[1].parse::<u32>().ok();
            let count = captures
                .get(2)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(1);
            intents.push(PathIntent {
                path: path.clone(),
                operation: *operation,
                line_start: start,
                line_end: start.map(|value| value.saturating_add(count.saturating_sub(1))),
            });
        }
    }
    // Some patch producers omit hunk line numbers. Still claim the file.
    if intents.is_empty() {
        if let Some((path, operation)) = current {
            intents.push(PathIntent {
                path,
                operation,
                line_start: None,
                line_end: None,
            });
        }
    }
    intents
}

fn extract_shell(input: &Value) -> Vec<PathIntent> {
    let command = input
        .get("cmd")
        .or_else(|| input.get("command"))
        .and_then(Value::as_str)
        .or_else(|| input.as_str());
    let Some(command) = command else {
        return Vec::new();
    };
    let Ok(words) = shell_words::split(command) else {
        return Vec::new();
    };
    if words.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    match words[0].as_str() {
        "rm" | "unlink" => {
            for path in words.iter().skip(1).filter(|word| !word.starts_with('-')) {
                result.push(PathIntent {
                    path: path.clone(),
                    operation: Operation::Delete,
                    line_start: None,
                    line_end: None,
                });
            }
        }
        "mv" => {
            let paths: Vec<_> = words
                .iter()
                .skip(1)
                .filter(|word| !word.starts_with('-'))
                .collect();
            if let Some(source) = paths.first() {
                result.push(PathIntent {
                    path: (*source).clone(),
                    operation: Operation::Rename,
                    line_start: None,
                    line_end: None,
                });
            }
            if let Some(target) = paths.get(1) {
                result.push(PathIntent {
                    path: (*target).clone(),
                    operation: Operation::Write,
                    line_start: None,
                    line_end: None,
                });
            }
        }
        "touch" | "truncate" => {
            for path in words.iter().skip(1).filter(|word| !word.starts_with('-')) {
                result.push(PathIntent {
                    path: path.clone(),
                    operation: Operation::Write,
                    line_start: None,
                    line_end: None,
                });
            }
        }
        _ => {
            for index in 0..words.len().saturating_sub(1) {
                if words[index] == ">" || words[index] == ">>" {
                    result.push(PathIntent {
                        path: words[index + 1].clone(),
                        operation: Operation::Write,
                        line_start: None,
                        line_end: None,
                    });
                }
            }
        }
    }
    result
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
}
