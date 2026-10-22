// @feature evals
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/runtime.md
// @boundary real-cli-and-mcp-fixtures
#[path = "support/eval_record.rs"]
mod eval_record;
mod support;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Output;
use support::Harness;

fn change() -> Value {
    json!({"id":"change-1","title":"Router regression motivation","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"change","motivation":{"state":"recorded","reason":"observed_failure","problem":"Router sent extraction to the wrong worker","expected_behavior":"Dispatch to the extraction worker","rationale":"Preserve the production failure as a regression case","provenance":"contemporaneous"},"evidence":[]}})
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn callers_record_git_evidence_and_can_recover_why_without_execution() {
    let mut harness = Harness::start();
    let git = json!({"id":"image-1","title":"Committed image fixture","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"artifact","media_type":"image/png","byte_size":321,"digest":null,"backings":[{"kind":"git_file","repository":"router-app","commit":"a".repeat(40),"object_format":"sha1","path":"evals/input.png","blob_oid":null,"entry_mode":"100644"}]}});
    let case = json!({"id":"case-1","title":"Route extraction","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"case","identity":"route","change_id":"change-1","inputs":["image-1"],"expected_behavior":"Choose extractor","assertions":[],"tags":["routing"]}});
    let result = harness.call(
        "nexus_evals",
        json!({"action":"record","records":[case,git,change()]}),
    );
    assert_eq!(result["inserted"], 3, "{result}");
    let detail = harness.call("nexus_evals", json!({"action":"get","id":"case-1"}));
    assert_eq!(detail["record"]["provenance"]["agent"], "mcp");
    let why = harness.call("nexus_evals", json!({"action":"why","id":"case-1"}));
    assert_eq!(why["truncated"], false);
    assert!(why["records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["body"]["motivation"]["rationale"]
            .as_str()
            .is_some_and(|s| s.contains("production failure"))));
    let list = harness.call(
        "nexus_evals",
        json!({"action":"list","repository":"router-app"}),
    );
    assert!(!list["records"].as_array().unwrap().is_empty());
    let invalid = harness.request("tools/call",json!({"name":"nexus_evals","arguments":{"action":"record","agent":"spoofed","records":[change()]}}));
    assert_eq!(invalid["error"]["code"], -32602);
}

#[test]
fn eval_reference_export_can_be_reimported_into_its_origin() {
    let mut harness = Harness::start();
    harness.call(
        "nexus_evals",
        json!({"action":"record","records":[change()]}),
    );
    let output = harness._temp.path().join("history.jsonl");
    let exported = harness.eval_cli(&["export", output.to_str().unwrap()]);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    success(harness.eval_cli(&["import", output.to_str().unwrap()]));
    success(harness.eval_cli(&["import", output.to_str().unwrap()]));
    let page = harness.call("nexus_evals", json!({"action":"list"}));
    assert_eq!(page["records"].as_array().unwrap().len(), 1);
}

#[test]
fn chunked_blob_and_reference_bundle_round_trip_preserve_source_and_verify_before_import() {
    let mut source = Harness::start();
    let bytes = vec![b'x'; 1_100_000];
    let raw = source._temp.path().join("output.bin");
    std::fs::write(&raw, &bytes).unwrap();
    let uploaded = success(source.eval_cli(&["upload", raw.to_str().unwrap()]));
    let digest = uploaded["blob"]["digest"].as_str().unwrap();
    assert_eq!(uploaded["blob"]["verified"], true);
    let artifact = json!({"id":"output-1","title":"Generated output","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"artifact","media_type":"application/octet-stream","byte_size":bytes.len(),"digest":digest,"backings":[{"kind":"stored_blob","digest":digest}]}});
    source.call(
        "nexus_evals",
        json!({"action":"record","records":[artifact]}),
    );
    let bundle = source._temp.path().join("bundle.jsonl");
    assert!(source
        .eval_cli(&["export", bundle.to_str().unwrap()])
        .status
        .success());
    let mut target = Harness::start();
    let bad = target._temp.path().join("incomplete.jsonl");
    remove_bundle_footer(&bundle, &bad);
    assert!(!target
        .eval_cli(&["import", bad.to_str().unwrap()])
        .status
        .success());
    assert!(
        target.call("nexus_evals", json!({"action":"list"}))["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    success(target.eval_cli(&["import", bundle.to_str().unwrap()]));
    success(target.eval_cli(&["import", bundle.to_str().unwrap()]));
    let record = target.call("nexus_evals", json!({"action":"get","id":"output-1"}));
    assert!(record["record"]["source"]["recorded_at"].is_string());
    let restored = target._temp.path().join("restored.bin");
    let result = target.eval_cli(&["download", digest, restored.to_str().unwrap()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(restored).unwrap(), bytes);
}

fn remove_bundle_footer(source: &Path, output: &Path) {
    let mut content = std::fs::read_to_string(source).unwrap();
    let footer = content.trim_end().rfind('\n').unwrap() + 1;
    content.truncate(footer);
    std::fs::write(output, content).unwrap();
}

#[test]
fn eval_schema_and_record_file_are_available_to_callers() {
    let harness = Harness::start();
    let schema = success(harness.eval_cli(&["schema"]));
    assert!(schema["oneOf"].is_array());
    let file = harness._temp.path().join("records.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({"format_version":1,"records":[change()]})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        success(harness.eval_cli(&["record", file.to_str().unwrap()]))["inserted"],
        1
    );
    assert!(!harness
        .eval_cli(&["record", Path::new("missing.json").to_str().unwrap()])
        .status
        .success());
}

#[test]
fn pending_blob_upload_can_be_aborted_without_touching_verified_artifacts() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};
    let mut harness = Harness::start();
    let digest = format!("{:x}", Sha256::digest(b"yes"));
    harness.call(
        "nexus_evals",
        json!({"action":"blob_begin","digest":digest,"byte_size":3}),
    );
    harness.call(
        "nexus_evals",
        json!({"action":"blob_put","digest":digest,"index":0,"data":STANDARD.encode(b"bad")}),
    );
    assert_eq!(
        success(harness.eval_cli(&["abort-upload", &digest]))["removed"],
        true
    );
    assert!(
        harness.call("nexus_evals", json!({"action":"blob_info","digest":digest}))["blob"]
            .is_null()
    );
    let file = harness._temp.path().join("correct.bin");
    std::fs::write(&file, b"yes").unwrap();
    assert_eq!(
        success(harness.eval_cli(&["upload", file.to_str().unwrap()]))["blob"]["verified"],
        true
    );
    assert!(!harness
        .eval_cli(&["abort-upload", &digest])
        .status
        .success());
}

#[test]
fn unavailable_materialization_remains_an_exportable_history_gap() {
    let mut source = Harness::start();
    let artifact = json!({"id":"git-artifact","title":"Pinned Git evidence","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"artifact","media_type":"text/plain","byte_size":3,"digest":null,"backings":[{"kind":"git_file","repository":"router-app","commit":"a".repeat(40),"object_format":"sha1","path":"evals/input.txt","blob_oid":null,"entry_mode":"100644"}]}});
    let verification = json!({"id":"missing-copy","title":"Git evidence could not be materialized","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"verification","artifact_id":"git-artifact","backing":{"kind":"stored_blob","digest":"0".repeat(64)},"state":"unavailable","checked_at":"2026-10-22T12:00:00Z","producer":"importer","reason":"Original Git remote is offline"}});
    let recorded = source.call(
        "nexus_evals",
        json!({"action":"record","records":[artifact,verification]}),
    );
    assert_eq!(recorded["inserted"], 2, "{recorded}");
    let bundle = source._temp.path().join("gap.jsonl");
    let exported = source.eval_cli(&["export", bundle.to_str().unwrap()]);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let mut target = Harness::start();
    let imported = success(target.eval_cli(&["import", bundle.to_str().unwrap()]));
    assert_eq!(imported["blobs"], 0);
    let gap = target.call("nexus_evals", json!({"action":"get","id":"missing-copy"}));
    assert_eq!(gap["record"]["body"]["state"], "unavailable");
}

fn component_history() -> Vec<Value> {
    let definitions = [
        (
            "why",
            json!({"kind":"change","motivation":{"state":"missing","reason":"Imported fixture"}}),
        ),
        (
            "repo",
            json!({"kind":"repository","identity":"repo","change_id":"why","repository":"dispatch","commit":"a".repeat(40),"object_format":"sha1"}),
        ),
        (
            "declared",
            json!({"kind":"model","identity":"declared","change_id":"why","label":"declared","provider":"fixture","requested_model":"declared"}),
        ),
        (
            "actual",
            json!({"kind":"model","identity":"actual","change_id":"why","label":"actual","provider":"fixture","requested_model":"actual"}),
        ),
        (
            "case",
            json!({"kind":"case","identity":"case","change_id":"why","expected_behavior":"Dispatch task"}),
        ),
        (
            "grader",
            json!({"kind":"grader","identity":"grader","change_id":"why","grading":{"method":"human","instructions":"Inspect outcome"},"metric":{"name":"pass","unit":"ratio","direction":"higher"}}),
        ),
        (
            "suite",
            json!({"kind":"suite","identity":"suite","change_id":"why","cases":[{"case_id":"case","grader_ids":["grader"],"weight":1,"split":"development"}],"aggregation":"mean"}),
        ),
        (
            "target",
            json!({"kind":"target","identity":"target","change_id":"why","components":[{"id":"worker","role":"solve","repository_id":"repo","model":{"assignment":"fixed","model_id":"declared"}}]}),
        ),
        (
            "run",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"all"}),
        ),
        (
            "trial",
            json!({"kind":"trial","run_id":"run","case_id":"case","replicate":0,"state":"completed"}),
        ),
        (
            "attempt",
            json!({"kind":"attempt","trial_id":"trial","state":"completed","started_at":"2026-10-22T12:00:00Z"}),
        ),
        (
            "z-call",
            json!({"kind":"call","attempt_id":"attempt","component":"worker","instance":"worker-1","sequence":0,"model_id":"actual"}),
        ),
        (
            "a-grade",
            json!({"kind":"grade","subject":{"scope":"component","attempt_id":"attempt","instance":"worker-1","call_ids":["z-call"]},"grader_id":"grader","pass_id":"pass-1","state":"completed","score":1,"explanation":"Pinned call result"}),
        ),
        (
            "b-unknown",
            json!({"kind":"grade","subject":{"scope":"component","attempt_id":"attempt","instance":"worker-1"},"grader_id":"grader","pass_id":"pass-2","state":"completed","score":1,"explanation":"Original record lacks call attribution"}),
        ),
    ];
    definitions
        .into_iter()
        .map(|(id, body)| serde_json::to_value(eval_record::record(id, body)).unwrap())
        .collect()
}

fn actual_component_grades(harness: &mut Harness) -> Vec<String> {
    let result = harness.call(
        "nexus_evals",
        json!({"action":"list","kind":"grade","repository":"dispatch","model":"actual"}),
    );
    result["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn reference_exchange_preserves_explicit_component_model_attribution() {
    let mut source = Harness::start();
    let recorded = source.call(
        "nexus_evals",
        json!({"action":"record","records":component_history()}),
    );
    assert_eq!(recorded["inserted"], 14, "{recorded}");
    assert_eq!(actual_component_grades(&mut source), vec!["a-grade"]);
    let bundle = source._temp.path().join("components.jsonl");
    let exported = source.eval_cli(&["export", bundle.to_str().unwrap()]);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let mut target = Harness::start();
    success(target.eval_cli(&["import", bundle.to_str().unwrap()]));
    assert_eq!(actual_component_grades(&mut target), vec!["a-grade"]);
    let unknown = target.call("nexus_evals", json!({"action":"get","id":"b-unknown"}));
    assert!(unknown["record"]["source"]["recorded_at"].is_string());
}
