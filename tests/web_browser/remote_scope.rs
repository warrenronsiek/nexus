// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @boundary browser-http: native remote inspection distinguishes selected project scope from true global evidence.
use super::*;
use nexus::config::{ClientConfig, RemoteClientConfig};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn native_remote_session_reports_its_authenticated_project_scope() {
    let (_upstream, app) = remote_proxy().await;
    let response = app.login().await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let expected = json!({"authenticated":true,"username":"viewer","default_project_id":"project-a","global_project_reads":false});
    assert_eq!(response.json::<Value>().await.unwrap(), expected);
    let session = app
        .client
        .get(format!("{}/ui/api/session", app.origin))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(session, expected);
    let anonymous = app
        .client
        .get(format!("{}/ui/api/session", app.origin))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(anonymous, json!({"authenticated":false}));
}

#[tokio::test]
async fn native_remote_browser_rejects_false_global_project_reads() {
    let (_upstream, app) = remote_proxy().await;
    let cookie = app.cookie().await;
    for kind in [
        "coordination",
        "coordination_sessions",
        "coordination_claims",
        "coordination_conflicts",
        "coordination_events",
        "usage",
        "analytics_summary",
        "analytics_catalog",
        "analytics_opportunities",
        "analytics_examples",
    ] {
        let response = read_scope(&app, &cookie, None, json!({"kind":kind})).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{kind} silently used the configured project"
        );
        let error = response.json::<Value>().await.unwrap();
        assert!(
            error["error"].as_str().unwrap().contains("native remote"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn native_remote_browser_preserves_selected_and_genuine_global_reads() {
    let (upstream, app) = remote_proxy().await;
    seed_report(&upstream, "project-a", "A report").await;
    seed_report(&upstream, "project-b", "First B report").await;
    seed_report(&upstream, "project-b", "Second B report").await;
    let cookie = app.cookie().await;
    let projects = read_scope(&app, &cookie, None, json!({"kind":"projects"})).await;
    assert_eq!(projects.status(), StatusCode::OK);
    let catalog = projects.json::<Value>().await.unwrap();
    let ids: Vec<_> = catalog["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["project_id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"project-a") && ids.contains(&"project-b"),
        "{catalog}"
    );
    for (project, count) in [("project-a", 1), ("project-b", 2)] {
        let response = read_scope(&app, &cookie, Some(project), json!({"kind":"sos_list"})).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.json::<Value>().await.unwrap()["untriaged_report_count"],
            count
        );
    }
    for query in [
        json!({"kind":"analytics_categories"}),
        json!({"kind":"memory_context","scope":"global"}),
        json!({"kind":"memory_search","scope":"global","regex":"."}),
    ] {
        let response = read_scope(&app, &cookie, None, query).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap()["ok"], true);
    }
}

async fn read_scope(app: &Browser, cookie: &str, project: Option<&str>, query: Value) -> Response {
    app.post("/ui/api/read")
        .header("cookie", cookie)
        .json(&json!({"project_id":project,"query":query}))
        .send()
        .await
        .unwrap()
}

async fn seed_report(upstream: &Browser, project: &str, message: &str) {
    let response = upstream.client.post(format!("{}/api/v1/request", upstream.origin)).bearer_auth(AGENT_SECRET)
        .json(&json!({"project_id":project,"method":"sos","params":{"action":"report","agent":"app","session_id":"source","model":"test-model","message":message,"type":"input","environment":"test","version":"1","trace":"source","metadata":{}}}))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json::<Value>().await.unwrap()["ok"], true);
}

async fn remote_proxy() -> (Browser, Browser) {
    let upstream = Browser::start(None).await;
    let temporary = tempfile::tempdir().unwrap();
    let secret = temporary.path().join("agent-secret");
    std::fs::write(&secret, AGENT_SECRET).unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    let credentials = temporary.path().join("web.json");
    configure_credentials(&credentials, "viewer", PASSWORD).unwrap();
    let mut config = Config {
        client: ClientConfig::Remote(RemoteClientConfig {
            endpoint: upstream.origin.clone(),
            project_id: "project-a".into(),
            secret_file: Some(secret),
        }),
        ..Config::default()
    };
    config.memory.enabled = false;
    let loaded = LoadedConfig {
        config,
        sources: vec![],
        hash: "remote-browser-test".into(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(nexus::runtime::server::serve_browser_listener(
        listener,
        loaded,
        None,
        BrowserConfig::load(&credentials, None).unwrap(),
    ));
    (
        upstream,
        Browser {
            _temporary: temporary,
            origin,
            client: Client::new(),
            task,
        },
    )
}
