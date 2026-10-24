// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary checksummed-work-transfer-fixture
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::process::Command;
use support::Harness;

fn observation(id: &str, subject: &str) -> Value {
    json!({
        "id":id,"subject":{"scope":"session","id":subject},
        "occurred_at":"2026-10-24T12:00:00Z",
        "producer":{"name":"fixture","version":"1","instance_id":"source"},
        "method":"source_import",
        "body":{"event":"session_opened","title":subject,"context":{}},
        "correction":null,"project_id":"original-project","recorded_at":"2026-10-24T12:01:00Z",
        "provenance":{"agent":"source-ci","session_id":"source-host","model_id":null,"config_hash":"source-config"},
        "source":{"project_id":"original-project","recorded_at":"2026-10-24T12:01:00Z",
          "provenance":{"agent":"source-ci","session_id":"source-host","model_id":null,"config_hash":"source-config"}}
    })
}

fn write_bundle(path: &std::path::Path, observations: &[Value]) {
    let mut bytes = serde_json::to_vec(&json!({"type":"header","format_version":1})).unwrap();
    bytes.push(b'\n');
    for observation in observations {
        bytes.extend(
            serde_json::to_vec(&json!({"type":"observation","observation":observation})).unwrap(),
        );
        bytes.push(b'\n');
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    bytes.extend(
        serde_json::to_vec(&json!({"type":"footer","count":observations.len(),"sha256":hash}))
            .unwrap(),
    );
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn invalid_source_receipt_is_rejected_before_any_imported_history_is_published() {
    for invalid_field in ["project", "actor"] {
        let mut destination = Harness::start();
        let valid = observation("valid-event", "valid-session");
        let mut invalid = observation("invalid-event", "invalid-session");
        if invalid_field == "project" {
            invalid["source"]["project_id"] = json!("");
        } else {
            invalid["source"]["provenance"]["agent"] = json!("");
        }
        let path = destination.root.join("source.jsonl");
        write_bundle(&path, &[valid, invalid]);
        let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
            .current_dir(&destination.root)
            .arg("--config")
            .arg(destination._temp.path().join("config.toml"))
            .args(["work", "import"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "invalid {invalid_field} receipt unexpectedly imported"
        );
        assert!(
            stderr.contains("invalid work"),
            "unexpected rejection: {stderr}"
        );
        let page = destination.call("nexus_work", json!({"action":"list"}));
        assert_eq!(page["items"], json!([]), "late invalid {invalid_field} source must be rejected before publishing the earlier valid frame: {page}");
    }
}
