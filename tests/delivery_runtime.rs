// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
// @boundary delivery-runtime-json: exercise the public CLI and MCP contracts with isolated fixtures.
mod support;

use serde_json::{json, Value};
use std::process::Command;
use support::Harness;

#[test]
fn delivery_is_discoverable_and_cli_mcp_share_project_reads() {
    let help = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["delivery", "--help"])
        .output()
        .unwrap();
    assert!(
        help.status.success(),
        "{}",
        String::from_utf8_lossy(&help.stderr)
    );
    let help = String::from_utf8(help.stdout).unwrap();
    for name in ["schema", "record", "list", "get", "origin", "inbox"] {
        assert!(help.contains(name), "missing {name}: {help}");
    }
    let mut harness = Harness::start();
    let response = harness.call("nexus_delivery", json!({"action":"list"}));
    assert_eq!(response["ok"], true, "{response}");
    let output = harness.feature_cli("delivery", &["list"]);
    support::assert_success("delivery list", &output);
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(cli, response);
}

#[test]
fn checkout_registration_preserves_the_implementer_context_for_origin_lookup() {
    let mut harness = Harness::start();
    harness.initialize_git();
    let base = git_revision(&harness);
    std::fs::write(harness.root.join("tracked.txt"), "initial\nimplemented\n").unwrap();
    support::run_git(&harness.root, &["add", "tracked.txt"]);
    support::run_git(
        &harness.root,
        &["commit", "-m", "Implement the contribution"],
    );
    let head = git_revision(&harness);
    let output = harness.feature_cli_with_context(
        "delivery",
        &[
            "contribute",
            "--id",
            "originating-change",
            "--title",
            "Preserve implementation context",
            "--rationale",
            "The inserted line keeps the original implementation intent",
            "--repository",
            "example/repo",
            "--base",
            &base,
            "--head",
            &head,
        ],
        &[
            ("NEXUS_AGENT", "codex"),
            ("NEXUS_HOST_SESSION_ID", "implementer-thread"),
        ],
    );
    support::assert_success("register checkout contribution", &output);
    let response = harness.call(
        "nexus_delivery",
        json!({
            "action":"resolve_origin", "query":{
                "revision":{"repository":"example/repo","object_format":"sha1","commit":head},
                "path":"tracked.txt","line_start":2,"line_end":2
            }
        }),
    );
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(
        response["matches"][0]["contribution"]["origin"]["session_id"], "implementer-thread",
        "{response}"
    );
    assert_eq!(
        response["matches"][0]["contribution"]["rationale"],
        "The inserted line keeps the original implementation intent"
    );
}

fn git_revision(harness: &Harness) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(&harness.root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    support::assert_success("read revision", &output);
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn oversized_delivery_batch_is_rejected_atomically_before_projection() {
    use nexus::delivery::{DeliveryAction, DeliveryObservationInput, DeliveryQuery};
    use nexus::persistence::Store;
    use nexus::work_history::WorkActor;
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let actor = WorkActor {
        agent: "codex".into(),
        session_id: "batch-test".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    let observations = (0..24).map(|index| serde_json::from_value::<DeliveryObservationInput>(json!({
        "id":format!("event-{index}"),"occurred_at":"2026-11-15T12:00:00Z",
        "body":{"event":"contribution_registered","contribution":{
            "id":format!("change-{index}"),"repository":"example/repo","title":"Bounded registration",
            "origin":{"agent":"codex","session_id":"batch-test"},"rationale":"Record the selected implementation",
            "constraints":vec!["x".repeat(2000);20]
        }}
    })).unwrap()).collect::<Vec<_>>();
    let result =
        store.delivery_command("project", &actor, &DeliveryAction::Record { observations });
    assert!(
        result.is_err(),
        "an oversized aggregate batch must be rejected before recording"
    );
    let response = store
        .delivery_command(
            "project",
            &actor,
            &DeliveryAction::List {
                project_id: None,
                query: DeliveryQuery::default(),
            },
        )
        .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap()["items"], json!([]));
}

#[test]
fn delivery_import_rejects_invalid_original_provenance() {
    use nexus::delivery::{DeliveryAction, DeliveryObservation, DeliveryResponse};
    use nexus::persistence::Store;
    use nexus::work_history::WorkActor;
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let actor = WorkActor {
        agent: "codex".into(),
        session_id: "importer".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    let observation:DeliveryObservation=serde_json::from_value(json!({
        "input":{"id":"bad-provenance","occurred_at":"2026-11-15T12:00:00Z","body":{
            "event":"contribution_registered","contribution":{
                "id":"change","repository":"repo","title":"Imported implementation",
                "origin":{"agent":"codex","session_id":"original"},"rationale":"Retain original implementation context"
            }
        }},
        "project_id":"original-project","recorded_at":"2026-11-15T12:00:01Z",
        "provenance":{"agent":"codex","session_id":"original","model_id":null,"config_hash":"x".repeat(513)},"source":null
    })).unwrap();
    let result = store.delivery_command(
        "target-project",
        &actor,
        &DeliveryAction::Import {
            observations: vec![observation],
        },
    );
    assert!(
        result.is_err(),
        "import must validate the retained actor rather than only the current importer"
    );
    let response = store
        .delivery_command(
            "target-project",
            &actor,
            &DeliveryAction::Export {
                project_id: None,
                after: None,
                limit: None,
            },
        )
        .unwrap();
    assert!(
        matches!(response,DeliveryResponse::Events {observations,..} if observations.is_empty())
    );
}

#[test]
fn portable_delivery_import_rejects_a_scalar_event_id_that_differs_from_its_body() {
    use nexus::delivery::{DeliveryAction, DeliveryObservationInput};
    use nexus::persistence::Store;
    use nexus::work_history::WorkActor;
    let temp = tempfile::tempdir().unwrap();
    let mut source = Store::open(&temp.path().join("source.db")).unwrap();
    let actor = WorkActor {
        agent: "codex".into(),
        session_id: "original".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    let input:DeliveryObservationInput=serde_json::from_value(json!({
        "id":"original-event","occurred_at":"2026-11-15T12:00:00Z","body":{
            "event":"contribution_registered","contribution":{
                "id":"change","repository":"repo","title":"Imported implementation",
                "origin":{"agent":"codex","session_id":"original"},"rationale":"Retain original implementation context"
            }
        }
    })).unwrap();
    source
        .delivery_command(
            "project",
            &actor,
            &DeliveryAction::Record {
                observations: vec![input],
            },
        )
        .unwrap();
    let mut bytes = Vec::new();
    source.write_streamed_snapshot(&mut bytes).unwrap();
    let mut changed = false;
    let mut rewritten = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let mut record: Value = serde_json::from_slice(line).unwrap();
        if record["data"]["table"] == "delivery_observations" {
            for row in record["data"]["rows"].as_array_mut().unwrap() {
                row["id"] = json!("different-scalar-id");
                changed = true;
            }
        }
        rewritten.extend(serde_json::to_vec(&record).unwrap());
        rewritten.push(b'\n');
    }
    assert!(changed, "fixture must contain a retained delivery event");
    let snapshot = temp.path().join("changed.jsonl");
    std::fs::write(&snapshot, rewritten).unwrap();
    let mut target = Store::open(&temp.path().join("target.db")).unwrap();
    assert!(
        target.import_streamed_snapshot(&snapshot).is_err(),
        "scalar journal identity must match immutable source identity"
    );
}
