// @feature runtime
// @feature coordination
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @boundary dynamic-json: simulated lifecycle wire responses
use super::support::remote_client_config;
use axum::{extract::State, routing::post, Json, Router};
use nexus::{coordination::api::ServiceRequest, runtime::client::NexusClient};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

async fn exercise(responses: Vec<Value>) -> (Value, Vec<Value>, Duration) {
    let state = Arc::new(Mutex::new((VecDeque::from(responses), Vec::new())));
    let app = Router::new()
        .route("/api/v1/request", post(reply))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temporary = tempfile::tempdir().unwrap();
    let secret = temporary.path().join("secret");
    std::fs::write(&secret, "test-secret-that-is-long-enough-for-auth").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    let loaded = remote_client_config(address, "retry-project", secret);
    let request = ServiceRequest::decode(
        "session_stop",
        json!({"session_id":"retry-session","agent":"codex","project_root":"/repo"}),
    )
    .unwrap();
    let started = Instant::now();
    let response = NexusClient::new(&loaded, None)
        .lifecycle_request(&request)
        .await;
    let elapsed = started.elapsed();
    task.abort();
    let captured = state.lock().unwrap().1.clone();
    (response, captured, elapsed)
}

type Replies = Arc<Mutex<(VecDeque<Value>, Vec<Value>)>>;
async fn reply(State(state): State<Replies>, Json(request): Json<Value>) -> Json<Value> {
    if request["method"] != "session_stop" {
        return Json(json!({"ok":true}));
    }
    let mut state = state.lock().unwrap();
    state.1.push(request);
    let response = if state.0.len() > 1 {
        state.0.pop_front().unwrap()
    } else {
        state.0.front().unwrap().clone()
    };
    Json(response)
}

fn busy() -> Value {
    json!({"permitted":true,"recorded":false,"retryable":true,"advisories":[]})
}

#[tokio::test]
async fn lifecycle_retry_preserves_request_and_successful_advisories() {
    let success =
        json!({"permitted":true,"recorded":true,"advisories":[{"message":"peer overlap"}]});
    let (response, captured, _) = exercise(vec![busy(), busy(), success.clone()]).await;
    assert_eq!(response["recorded"], true, "{response}");
    assert_eq!(response["advisories"], success["advisories"]);
    assert_eq!(captured.len(), 3);
    assert!(captured.iter().all(|request| request == &captured[0]));
}

#[tokio::test]
async fn lifecycle_retry_never_replays_uncertain_or_recorded_results() {
    for response in [
        json!({"permitted":true,"recorded":false,"advisories":[]}),
        json!({"permitted":true,"recorded":true,"retryable":true,"advisories":[]}),
    ] {
        let (_, captured, _) = exercise(vec![response]).await;
        assert_eq!(captured.len(), 1);
    }
}

#[tokio::test]
async fn lifecycle_retry_exhaustion_remains_bounded_and_permissive() {
    let (response, captured, elapsed) = exercise(vec![busy()]).await;
    assert_eq!(response["permitted"], true);
    assert_eq!(response["recorded"], false);
    assert!(captured.len() > 1);
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}
