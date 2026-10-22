// @feature evals
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/runtime.md
// @boundary caller-metric-records-and-reference-exchange
mod support;
use serde_json::{json, Value};
use support::Harness;

fn latency_history() -> Value {
    let bodies = [
        (
            "why",
            json!({"kind":"change","motivation":{"state":"recorded","reason":"user_requirement","problem":"A bare value does not identify its measurement","expected_behavior":"Retain elapsed time and its definition","rationale":"Explain what changed from 10 to 7","provenance":"contemporaneous"}}),
        ),
        (
            "source",
            json!({"kind":"artifact","media_type":"text/x-python","byte_size":31,"backings":[{"kind":"git_file","repository":"router","commit":"a".repeat(40),"object_format":"sha1","path":"evals/elapsed.py","entry_mode":"100644"}]}),
        ),
        (
            "elapsed",
            json!({"kind":"metric","identity":"request-elapsed","change_id":"why","name":"Request elapsed time","description":"Wall clock time for one completed request","unit":"seconds","value_format":"Exact decimal text","calculation":{"description":"Subtract the caller's monotonic start from finish","formula":"finished - started","source_artifacts":["source"],"population":"One completed request per run","aggregation":"Identity; one sample","exclusions":"None","missing_data":"Record unavailable with a reason"}}),
        ),
        (
            "suite",
            json!({"kind":"suite","identity":"suite","change_id":"why","cases":[],"aggregation":"Caller-defined"}),
        ),
        (
            "repo",
            json!({"kind":"repository","identity":"router","change_id":"why","repository":"router","commit":"a".repeat(40),"object_format":"sha1"}),
        ),
        (
            "model",
            json!({"kind":"model","identity":"worker","change_id":"why","label":"worker","provider":"caller","requested_model":"caller-model"}),
        ),
        (
            "target",
            json!({"kind":"target","identity":"target","change_id":"why","components":[{"id":"worker","role":"solve","repository_id":"repo","model":{"assignment":"fixed","model_id":"model"}}]}),
        ),
        (
            "baseline",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"One completed request"}),
        ),
        (
            "candidate",
            json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"completed","selection_rule":"One completed request"}),
        ),
        (
            "baseline-time",
            json!({"kind":"measurement","metric_id":"elapsed","subject":{"scope":"run","id":"baseline"},"value":{"storage":"inline","text":"10"},"explanation":"Caller-observed duration","inputs":["baseline"],"evidence":[]}),
        ),
        (
            "candidate-time",
            json!({"kind":"measurement","metric_id":"elapsed","subject":{"scope":"run","id":"candidate"},"value":{"storage":"inline","text":"7"},"explanation":"Caller-observed duration","inputs":["candidate"],"evidence":[]}),
        ),
        (
            "comparison",
            json!({"kind":"comparison","baseline_ids":["baseline"],"candidate_ids":["candidate"],"grade_ids":[],"measurement_ids":["baseline-time","candidate-time"],"population":"One request per run","exclusions":"None","validity":"matched","differences":[],"result":"Elapsed time changed from 10 seconds to 7 seconds","uncertainty":"Single samples; no statistical claim"}),
        ),
    ];
    Value::Array(bodies.into_iter().map(|(id,body)|json!({"id":id,"title":id,"occurred_at":"2026-10-22T12:00:00Z","body":body})).collect())
}

fn assert_meaning(harness: &mut Harness) {
    let measurement = harness.call("nexus_evals", json!({"action":"get","id":"candidate-time"}));
    assert_eq!(measurement["record"]["body"]["value"]["text"], "7");
    let history = harness.call(
        "nexus_evals",
        json!({"action":"why","id":"comparison","limit":64}),
    );
    let records = history["records"].as_array().unwrap();
    let metric = records.iter().find(|r| r["id"] == "elapsed").unwrap();
    assert_eq!(metric["body"]["unit"], "seconds");
    assert_eq!(
        metric["body"]["calculation"]["formula"],
        "finished - started"
    );
    assert!(records.iter().any(|r| r["id"] == "source"));
    assert!(records
        .iter()
        .any(|r| r["body"]["motivation"]["rationale"] == "Explain what changed from 10 to 7"));
    let filtered = harness.call(
        "nexus_evals",
        json!({"action":"list","kind":"measurement","repository":"router","model":"worker"}),
    );
    assert_eq!(filtered["records"].as_array().unwrap().len(), 2);
}

#[test]
fn metric_meaning_survives_mcp_cli_and_reference_exchange_without_a_numeric_grader() {
    let mut source = Harness::start();
    let result = source.request(
        "tools/call",
        json!({"name":"nexus_evals","arguments":{"action":"record","records":latency_history()}}),
    );
    assert_eq!(
        result["result"]["structuredContent"]["inserted"], 12,
        "{result}"
    );
    assert_meaning(&mut source);
    let bundle = source._temp.path().join("metrics.jsonl");
    assert!(source
        .eval_cli(&["export", bundle.to_str().unwrap()])
        .status
        .success());
    let mut target = Harness::start();
    let imported = target.eval_cli(&["import", bundle.to_str().unwrap()]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert_meaning(&mut target);
}
