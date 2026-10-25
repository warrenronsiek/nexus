// @feature evals
// @feature persistence
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/persistence.md
// @spec docs/features/runtime.md
// @boundary postgres-and-caller-json
#![cfg(feature = "postgres")]
#[path = "support/eval_record.rs"]
mod eval_record;
use base64::{engine::general_purpose::STANDARD, Engine};
use nexus::coordination::api::ServiceRequest;
use nexus::{Config, LoadedConfig, NexusService};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn invoke(service: &NexusService, project: &str, command: Value) -> Value {
    let request=ServiceRequest::decode("evals",json!({"context":{"session_id":"pg-eval","agent":"test","project_root":"/caller-only/repo"},"command":command})).unwrap();
    let result = service.handle_remote(project, request).to_json();
    assert_eq!(result["ok"], true, "{result}");
    result
}

#[test]
fn postgres_eval_history_preserves_bound_project_git_motivations_and_chunked_bytes() {
    let url =
        std::env::var("NEXUS_TEST_DATABASE_URL").expect("PostgreSQL integration URL required");
    let service = NexusService::connect(
        LoadedConfig {
            config: Config::default(),
            sources: vec![],
            hash: "pg-eval-config".into(),
        },
        &url,
    )
    .unwrap();
    let project = format!("eval-pg-{}", uuid::Uuid::new_v4());
    let change = json!({"id":"why","title":"Why this fixture exists","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"change","motivation":{"state":"recorded","reason":"observed_failure","problem":"Router missed an image input","expected_behavior":"Handle the image","rationale":"Preserve the reported failure","provenance":"contemporaneous"},"evidence":[]}});
    let fixture = json!({"id":"image","title":"Git image","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"artifact","media_type":"image/png","byte_size":123,"backings":[{"kind":"git_file","repository":"router-repo","commit":"a".repeat(40),"object_format":"sha1","path":"evals/input.png","entry_mode":"100644"}]}});
    let case = json!({"id":"case","title":"Router image regression","occurred_at":"2026-10-22T12:00:00Z","body":{"kind":"case","identity":"image-routing","change_id":"why","inputs":["image"],"expected_behavior":"Dispatch the image correctly"}});
    let recorded = invoke(
        &service,
        &project,
        json!({"action":"record","records":[case,fixture,change]}),
    );
    assert_eq!(recorded["inserted"], 3);
    assert_eq!(
        invoke(&service, &project, json!({"action":"why","id":"case"}))["records"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        invoke(
            &service,
            &project,
            json!({"action":"list","repository":"router-repo"})
        )["records"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(invoke(
        &service,
        "other-eval-project",
        json!({"action":"get","id":"case"})
    )["record"]
        .is_null());
    verify_blob(&service, &project);
    verify_lifecycle(&service, &project);
    verify_metric_meaning(&service, &project);
    verify_dashboard(&service, &project);
}

fn verify_metric_meaning(service: &NexusService, project: &str) {
    let metric = eval_record::record(
        "metric",
        json!({"kind":"metric","identity":"caller-counter","change_id":"why","name":"Caller counter","description":"An exact caller-defined quantity","unit":"arbitrary events / interval","value_format":"Exact decimal text","calculation":{"description":"Caller computes the reported quantity","formula":"sum(caller events)","source_artifacts":[],"population":"Caller event stream","aggregation":"Caller sum","exclusions":"None reported","missing_data":"Explicit unavailable report"}}),
    );
    let value = "90071992547409931234567890.000000000001";
    let measurement = eval_record::record(
        "measurement",
        json!({"kind":"measurement","metric_id":"metric","subject":{"scope":"run","id":"run"},"value":{"storage":"inline","text":value},"explanation":"Preserve the caller's exact representation"}),
    );
    invoke(
        service,
        project,
        json!({"action":"record","records":[measurement,metric]}),
    );
    let result = invoke(service, project, json!({"action":"get","id":"measurement"}));
    assert_eq!(result["record"]["body"]["value"]["text"], value);
    let history = invoke(
        service,
        project,
        json!({"action":"why","id":"measurement","limit":64}),
    );
    let definition = history["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "metric")
        .unwrap();
    assert_eq!(definition["body"]["unit"], "arbitrary events / interval");
    assert_eq!(
        definition["body"]["calculation"]["formula"],
        "sum(caller events)"
    );
}

fn verify_lifecycle(service: &NexusService, project: &str) {
    let records = [
        ("suite",json!({"kind":"suite","identity":"suite","change_id":"why","cases":[],"aggregation":"mean"})),
        ("target",json!({"kind":"target","identity":"target","change_id":"why","components":[{"id":"agent","role":"solve","model":{"assignment":"unspecified","reason":"Caller has not exposed its model"}}]})),
        ("run",json!({"kind":"run","target_id":"target","suite_id":"suite","runner":"caller","runner_version":"1","state":"planned","selection_rule":"all"})),
        ("running",json!({"kind":"lifecycle","subject":{"scope":"run","id":"run"},"state":"running"})),
        ("completed",json!({"kind":"lifecycle","subject":{"scope":"run","id":"run"},"state":"completed","previous_id":"running"})),
    ].into_iter().map(|(id,body)|eval_record::record(id,body)).collect::<Vec<_>>();
    invoke(
        service,
        project,
        json!({"action":"record","records":records}),
    );
    let result = invoke(service, project, json!({"action":"get","id":"run"}));
    assert_eq!(result["record"]["body"]["state"], "planned");
    assert_eq!(
        result["record"]["lifecycle"]["frontiers"][0]["id"],
        "completed"
    );
    let fork = eval_record::record(
        "failed",
        json!({"kind":"lifecycle","subject":{"scope":"run","id":"run"},"state":"failed","previous_id":"running"}),
    );
    invoke(
        service,
        project,
        json!({"action":"record","records":[fork]}),
    );
    let result = invoke(service, project, json!({"action":"get","id":"run"}));
    assert_eq!(
        result["record"]["lifecycle"]["frontiers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(result["record"]["lifecycle"]["truncated"], false);
}

fn verify_blob(service: &NexusService, project: &str) {
    let bytes = b"generated assessment output";
    let digest = format!("{:x}", Sha256::digest(bytes));
    invoke(
        service,
        project,
        json!({"action":"blob_begin","digest":digest,"byte_size":bytes.len()}),
    );
    invoke(
        service,
        project,
        json!({"action":"blob_put","digest":digest,"index":0,"data":STANDARD.encode(bytes)}),
    );
    assert_eq!(
        invoke(
            service,
            project,
            json!({"action":"blob_seal","digest":digest})
        )["blob"]["verified"],
        true
    );
    let chunk = invoke(
        service,
        project,
        json!({"action":"blob_get","digest":digest,"index":0}),
    );
    assert_eq!(
        STANDARD.decode(chunk["data"].as_str().unwrap()).unwrap(),
        bytes
    );
}

fn verify_dashboard(service: &NexusService, project: &str) {
    let overview = invoke(service, project, json!({"action":"dashboard"}));
    assert_eq!(overview["run_count"], 1);
    let id = overview["groups"][0]["id"].as_str().unwrap();
    let selected = invoke(
        service,
        project,
        json!({"action":"dashboard","group_id":id}),
    );
    assert_eq!(selected["series"][0]["unit"], "arbitrary events / interval");
    assert_eq!(
        selected["series"][0]["points"][0]["value"]["text"],
        "90071992547409931234567890.000000000001"
    );
    assert!(selected["series"][0]["points"][0]["numeric_value"].is_null());
    let empty = invoke(
        service,
        "unrelated-eval-project",
        json!({"action":"dashboard"}),
    );
    assert!(empty["groups"].as_array().unwrap().is_empty());
}
