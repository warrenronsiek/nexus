// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary package-test-json: exercise the real tool and package formats.
use std::process::Command;

#[test]
fn audit_export_and_verification_are_public_commands() {
    for command in ["export", "verify-export"] {
        let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
            .args(["audit", command, "--help"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(feature = "sqlite")]
mod package {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{path::Path, process::Output};

    use super::support::{assert_success, Harness};

    fn output(result: Output) -> Value {
        assert_success("audit CLI", &result);
        serde_json::from_slice(&result.stdout).unwrap()
    }

    fn append(harness: &mut Harness, id: &str, at: &str, event: Value) {
        let response = harness.call(
            "nexus_audit",
            json!({
                "action":"append","record":{"id":id,"occurred_at":at,"event":event}
            }),
        );
        assert_eq!(response["ok"], true, "{response}");
    }

    fn upload(harness: &mut Harness, bytes: &[u8]) -> String {
        let digest = format!("{:x}", Sha256::digest(bytes));
        for action in [
            json!({"action":"blob_begin","digest":digest,"byte_size":bytes.len()}),
            json!({"action":"blob_put","digest":digest,"index":0,"data":STANDARD.encode(bytes)}),
            json!({"action":"blob_seal","digest":digest}),
        ] {
            let response = harness.call("nexus_evals", action);
            assert_eq!(response["ok"], true, "{response}");
        }
        digest
    }

    fn seed(harness: &mut Harness, digest: &str) {
        append(
            harness,
            "control",
            "2026-10-01T00:00:00Z",
            json!({"kind":"define",
            "control":{"key":"review","version":1},"definition":{"owner":"Eng","objective":"Review",
            "scope":"PRs","trigger":"Before merge","procedure":"Inspect changes",
            "effective_at":"2026-10-01T00:00:00Z","reason":"Initial","results":{"kind":"categorical",
            "codes":[{"code":"issues","label":"Changes requested"}]}}}),
        );
        for (id, at) in [
            ("sample", "2026-11-01T00:00:00Z"),
            ("outside", "2026-12-01T00:00:00Z"),
        ] {
            append(
                harness,
                id,
                at,
                json!({"kind":"run","control":{"key":"review","version":1},
                "run":{"subject":{"kind":"external","source":"Test","id":id},"executor":"Model",
                "result":{"status":"completed","output":{"kind":"code","value":"issues"},
                "findings":[{"id":"finding","description":"=Formula injection as evidence text","recommendation":"Document it"}],
                "evidence":[{"kind":"stored_blob","digest":digest,"description":"Verified run sample"}]}}}),
            );
        }
        append(
            harness,
            "fix",
            "2026-12-02T00:00:00Z",
            json!({"kind":"disposition","run_id":"sample",
            "finding_id":"finding","disposition":"remediated","reason":"Documented in the fix",
            "evidence":[{"kind":"external","uri":"https://example.org/fix","description":"Fix ticket"}]}),
        );
        append(
            harness,
            "verify",
            "2026-12-03T00:00:00Z",
            json!({"kind":"verification","run_id":"sample",
            "disposition_id":"fix","verifier":"Second reviewer","procedure":"Read ticket",
            "outcome":"confirmed","reason":"Evidence matches"}),
        );
    }

    fn check_full_package(harness: &Harness, full: &Path, digest: &str, artifact: &[u8]) {
        let manifest = output(harness.feature_cli(
            "audit",
            &[
                "export",
                full.to_str().unwrap(),
                "--from",
                "2026-11-01T00:00:00Z",
                "--until",
                "2026-12-01T00:00:00Z",
                "--include-blob",
                digest,
            ],
        ));
        assert_eq!(manifest["selected_run_ids"], json!(["sample"]));
        assert_eq!(
            manifest["counts"],
            json!({"runs":1,"findings":1,"followups":2,"journal_records":4})
        );
        assert_eq!(manifest["blob_contents"][digest], "included");
        assert_eq!(
            std::fs::read(full.join("evidence").join(digest)).unwrap(),
            artifact
        );
        let charts = std::fs::read_to_string(full.join("charts.csv")).unwrap();
        assert!(charts
            .contains("\"daily_runs\",\"review\",\"1\",\"2026-11-01\",\"2026-11-01\",\"\",\"1\""));
        let findings = std::fs::read_to_string(full.join("findings.csv")).unwrap();
        assert!(findings.contains("'=Formula"));
        let followups = std::fs::read_to_string(full.join("followups.csv")).unwrap();
        assert!(followups.contains("Second reviewer"));
        assert_eq!(
            output(harness.feature_cli("audit", &["verify-export", full.to_str().unwrap()]))
                ["verified"],
            true
        );
    }

    fn check_redacted_sample(harness: &Harness, sample: &Path) {
        let manifest = output(harness.feature_cli(
            "audit",
            &[
                "export",
                sample.to_str().unwrap(),
                "--run",
                "sample",
                "--redact-evidence",
            ],
        ));
        assert_eq!(manifest["redactions"], json!({"fix":1,"sample":1}));
        let journal = std::fs::read_to_string(sample.join("journal.jsonl")).unwrap();
        assert!(journal.contains("Redacted in export"));
        assert!(!journal.contains("example.org"));
        std::fs::write(sample.join("runs.csv"), b"changed").unwrap();
        let checked = harness.feature_cli("audit", &["verify-export", sample.to_str().unwrap()]);
        assert!(!checked.status.success());
        assert!(String::from_utf8_lossy(&checked.stderr).contains("digest mismatch for runs.csv"));
    }

    #[test]
    fn exports_fixed_period_or_sample_with_late_followups_and_detects_tampering() {
        let mut harness = Harness::start();
        let artifact = b"original review evidence";
        let digest = upload(&mut harness, artifact);
        seed(&mut harness, &digest);
        check_full_package(
            &harness,
            &harness._temp.path().join("full"),
            &digest,
            artifact,
        );
        check_redacted_sample(&harness, &harness._temp.path().join("sample"));
    }
}

#[cfg(feature = "sqlite")]
mod support;
