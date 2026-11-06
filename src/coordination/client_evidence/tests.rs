// @feature coordination
// @spec docs/features/coordination.md
use super::*;
use crate::coordination::domain::Operation;

fn intent(index: usize, padding: usize) -> PathIntent {
    PathIntent {
        path: format!("src/{index:04}-{}.rs", "x".repeat(padding)),
        operation: Operation::Write,
        line_start: None,
        line_end: None,
    }
}

fn observation(paths: impl IntoIterator<Item = PathIntent>) -> workspace::ChangedPathObservation {
    let output = paths
        .into_iter()
        .flat_map(|intent| format!("?? {}\0", intent.path).into_bytes())
        .collect();
    workspace::ChangedPathObservation::from_output(output)
}

#[test]
fn changed_path_pages_are_byte_bounded_and_rotate_without_starvation() {
    let paths = observation((0..2_100).map(|index| intent(index, 180)));
    let first = page_changed_paths(&paths, 0);

    assert!(first.truncated);
    assert!(first.paths.len() <= MAX_CHANGED_PATHS);
    assert!(
        first
            .paths
            .iter()
            .map(|intent| intent.path.len())
            .sum::<usize>()
            <= MAX_CHANGED_PATH_BYTES
    );
    assert!(first.next_offset > 0);

    let paths = observation((0..2_100).map(|index| intent(index, 180)));
    let second = page_changed_paths(&paths, first.next_offset);
    assert!(second
        .paths
        .iter()
        .any(|intent| intent.path.starts_with("src/2099-")));
}

#[test]
fn changed_path_paging_does_not_materialize_the_complete_git_record_set() {
    let paths = observation((0..50_000).map(|index| intent(index, 0)));

    let page = page_changed_paths(&paths, 0);

    assert_eq!(page.paths.len(), MAX_CHANGED_PATHS);
    assert!(page.truncated);
    assert_eq!(page.next_offset, MAX_CHANGED_PATHS);
}

#[test]
fn reconciliation_rejects_aggregate_path_payloads_above_the_wire_budget() {
    let evidence = ClientFilesystemEvidence::reconciliation(
        "session",
        "codex",
        (0..65).map(|index| intent(index, 4_050)).collect(),
        true,
    );

    assert!(evidence
        .validate_reconciliation()
        .unwrap_err()
        .to_string()
        .contains("aggregate byte limit"));
}

#[test]
fn derived_skill_documents_are_bounded_relative_and_tied_to_a_skill() {
    let evidence = |skill: Option<&str>, content: String, source_path: &str| {
        let mut evidence = ClientFilesystemEvidence::reconciliation("s", "codex", vec![], false);
        evidence.derived_usage = Some(ClientDerivedUsage {
            skill: skill.map(str::to_owned),
            skill_document: Some(crate::analytics::skills::SkillDocument {
                content,
                source_path: source_path.into(),
            }),
            ..ClientDerivedUsage::default()
        });
        evidence.validate()
    };
    assert!(evidence(Some("tdd"), "# TDD".into(), ".codex/skills/tdd/SKILL.md").is_ok());
    assert!(evidence(None, "# TDD".into(), ".codex/skills/tdd/SKILL.md").is_err());
    assert!(evidence(Some("tdd"), "# TDD".into(), "/home/u/skills/tdd/SKILL.md").is_err());
    assert!(evidence(
        Some("tdd"),
        "x".repeat(64 * 1024 + 1),
        "skills/tdd/SKILL.md"
    )
    .is_err());
    let legacy: ClientDerivedUsage = serde_json::from_value(serde_json::json!({
        "scripts": [], "skill": "tdd"
    }))
    .unwrap();
    assert!(legacy.skill_document.is_none());
}

#[test]
fn reconciliation_rejects_identities_outside_session_projection_bounds() {
    let oversized_agent =
        ClientFilesystemEvidence::reconciliation("session", "a".repeat(257), Vec::new(), false);
    assert!(oversized_agent
        .validate_reconciliation()
        .unwrap_err()
        .to_string()
        .contains("agent must contain 1 to 256 bytes"));

    for evidence in [
        ClientFilesystemEvidence::reconciliation("session\0suffix", "codex", Vec::new(), false),
        ClientFilesystemEvidence::reconciliation("session", "codex\nsuffix", Vec::new(), false),
    ] {
        assert!(evidence
            .validate_reconciliation()
            .unwrap_err()
            .to_string()
            .contains("control characters"));
    }
}
