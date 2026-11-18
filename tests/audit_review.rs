// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary review-test-json: deterministic providers and receipts cross the public CLI.
use std::process::Command;

#[test]
fn review_commit_accepts_audit_opt_in() {
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["review-commit", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--audit-control"));
    assert!(help.contains("--audit-record"));
}

#[cfg(feature = "sqlite")]
mod support;

#[cfg(feature = "sqlite")]
mod review {
    use super::{
        support::{executable::write_executable, run_git, Harness},
        Command,
    };
    use diesel::{Connection, RunQueryDsl, SqliteConnection};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::Output,
    };

    struct Fixture {
        harness: Harness,
        script: PathBuf,
        digest: String,
    }

    impl Fixture {
        fn new() -> Self {
            let mut harness = Harness::start();
            harness.initialize_git();
            let script = configure(&harness);
            let digest = stage(&harness);
            let sample = include_str!("../skills/nexus-audit/SKILL.md")
                .split("```json\n")
                .nth(1)
                .unwrap()
                .split("```")
                .next()
                .unwrap();
            let defined = harness.call(
                "nexus_audit",
                json!({
                    "action":"append","record":serde_json::from_str::<Value>(sample).unwrap()
                }),
            );
            assert_eq!(defined["ok"], true, "{defined}");
            Self {
                harness,
                script,
                digest,
            }
        }

        fn run(&self, name: &str, architecture: &str, deletion: Option<&str>) -> (PathBuf, Output) {
            configure_provider(&self.script, architecture, deletion);
            let saved = self.harness._temp.path().join(format!("{name}.json"));
            let output = self
                .harness
                .cli()
                .args([
                    "review-commit",
                    "--audit-control",
                    "cross-model-review@1",
                    "--audit-record",
                ])
                .arg(&saved)
                .output()
                .unwrap();
            assert_eq!(
                std::fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
                0o600
            );
            (saved, output)
        }

        fn replay(&self, saved: &Path) -> Value {
            let replayed = self
                .harness
                .feature_cli("audit", &["append", saved.to_str().unwrap()]);
            assert!(
                replayed.status.success(),
                "{}",
                String::from_utf8_lossy(&replayed.stderr)
            );
            serde_json::from_slice(&replayed.stdout).unwrap()
        }
    }

    fn configure(harness: &Harness) -> PathBuf {
        let skills = harness._temp.path().join("skills");
        for skill in ["code-architect", "code-deletion"] {
            let directory = skills.join(skill);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), skill).unwrap();
        }
        let config_path = harness._temp.path().join("config.toml");
        let script = harness._temp.path().join("fake-reviewer");
        let mut config = std::fs::read_to_string(&config_path).unwrap();
        config.push_str(&format!(
            r#"
[review]
enabled = true
skill_root = {:?}
[review.claude]
command = {:?}
model = "fixture-reviewer"
"#,
            skills, script
        ));
        std::fs::write(config_path, config).unwrap();
        script
    }

    fn stage(harness: &Harness) -> String {
        std::fs::write(
            harness.root.join("tracked.txt"),
            "new staged content
",
        )
        .unwrap();
        run_git(&harness.root, &["add", "tracked.txt"]);
        let diff = Command::new("git")
            .current_dir(&harness.root)
            .args([
                "diff",
                "--cached",
                "--no-ext-diff",
                "--unified=80",
                "--",
                ".",
            ])
            .output()
            .unwrap();
        format!("{:x}", Sha256::digest(&diff.stdout))
    }

    fn configure_provider(script: &Path, architecture: &str, deletion: Option<&str>) {
        let architecture = json!({"result":architecture});
        let deletion = deletion.map_or_else(
            || "exit 29".into(),
            |text| format!("printf '%s' '{}'", json!({"result":text})),
        );
        write_executable(
            script,
            &format!(
                r#"#!/bin/sh
case "$(cat)" in
  *'$code-architect'*) printf '%s' '{architecture}' ;;
  *) {deletion} ;;
esac
"#
            ),
        );
    }

    struct Expected<'a> {
        verdict: &'a str,
        status: &'a str,
        findings: usize,
    }

    fn assert_receipt(run: &Value, digest: &str, expected: Expected<'_>) {
        assert_eq!(run["subject"]["sha256"], digest);
        assert_eq!(run["result"]["status"], expected.status);
        assert_eq!(run["result"]["output"]["value"], expected.verdict);
        assert_eq!(
            run["result"]["findings"].as_array().unwrap().len(),
            expected.findings
        );
        assert_eq!(
            run["result"]["review_passes"][0]["model"],
            "fixture-reviewer"
        );
        assert!(run["result"]["review_passes"][0]["instructions_sha256"].is_string());
    }

    #[test]
    fn audit_records_provider_results_and_replays_the_original_receipt() {
        let fixture = Fixture::new();
        let finding = json!({"verdict":"CHANGES_REQUESTED","summary":"Remove stale setting",
            "findings":[{"confidence":"high","file":"tracked.txt","line":1,
                "evidence":"Setting is stale","change":"Remove it","verification":"Inspect checked file"}]}).to_string();
        for (name, architecture, deletion, verdict, status, findings) in [
            ("pass", "PASS", Some("PASS"), "pass", "completed", 0),
            (
                "findings",
                finding.as_str(),
                Some("PASS"),
                "changes_requested",
                "completed",
                1,
            ),
            (
                "partial",
                finding.as_str(),
                Some("malformed"),
                "inconclusive",
                "partial",
                1,
            ),
            (
                "provider-error",
                finding.as_str(),
                None,
                "inconclusive",
                "partial",
                1,
            ),
        ] {
            let (saved, output) = fixture.run(name, architecture, deletion);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let receipt: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_receipt(
                &receipt["record"]["input"]["event"]["run"],
                &fixture.digest,
                Expected {
                    verdict,
                    status,
                    findings,
                },
            );
            assert_eq!(fixture.replay(&saved), receipt);
        }
    }

    #[test]
    fn review_saves_a_bound_replay_before_a_rejected_append() {
        let fixture = Fixture::new();
        let mut database =
            SqliteConnection::establish(fixture.harness.database.to_str().unwrap()).unwrap();
        diesel::sql_query(
            "CREATE TRIGGER reject_test_audit_run BEFORE INSERT ON audit_records
            WHEN NEW.kind = 'run' BEGIN SELECT RAISE(ABORT, 'fixture append interruption'); END;",
        )
        .execute(&mut database)
        .unwrap();
        let (saved, output) = fixture.run("interrupted", "PASS", Some("PASS"));
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("Retry delivery with nexus audit append"));
        diesel::sql_query("DROP TRIGGER reject_test_audit_run")
            .execute(&mut database)
            .unwrap();
        std::fs::remove_file(&fixture.script).unwrap();
        let receipt = fixture.replay(&saved);
        assert_receipt(
            &receipt["record"]["input"]["event"]["run"],
            &fixture.digest,
            Expected {
                verdict: "pass",
                status: "completed",
                findings: 0,
            },
        );
    }
}
