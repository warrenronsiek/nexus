// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary cli-json: inspect the public CLI response.
use std::process::Command;

#[test]
fn audit_exposes_its_typed_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["audit", "schema"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let encoded = schema.to_string();
    for action in ["append", "list", "get", "journal"] {
        assert!(encoded.contains(action), "{action} missing from schema");
    }
}
