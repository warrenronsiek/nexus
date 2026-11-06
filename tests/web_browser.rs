// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @boundary browser-http: caller-controlled origins, credentials and closed read queries.
use nexus::config::{Config, LoadedConfig};
use nexus::runtime::server::{configure_credentials, serve_listener, BrowserConfig, ServerSecret};
use nexus::NexusService;
use reqwest::{header, Client, Response, StatusCode};
use serde_json::{json, Value};
use std::sync::Arc;

const AGENT_SECRET: &str = "independent-agent-secret-long-enough";
const PASSWORD: &[u8] = b"long browser password only";

#[path = "web_browser/remote_scope.rs"]
mod remote_scope;

struct Browser {
    _temporary: tempfile::TempDir,
    origin: String,
    client: Client,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}
impl Browser {
    async fn start(public_origin: Option<&str>) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let credentials = temporary.path().join("web-credentials.json");
        configure_credentials(&credentials, "viewer", PASSWORD).unwrap();
        let browser = BrowserConfig::load(&credentials, public_origin).unwrap();
        let mut config = Config::default();
        config.storage.database_path = temporary.path().join("nexus.db");
        config.memory.enabled = false;
        let service = Arc::new(
            NexusService::new(LoadedConfig {
                config,
                sources: vec![],
                hash: "browser-test".into(),
            })
            .unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(serve_listener(
            listener,
            service,
            ServerSecret::new(AGENT_SECRET).unwrap(),
            vec![],
            browser,
        ));
        Self {
            _temporary: temporary,
            origin,
            task,
            client: Client::new(),
        }
    }
    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}{path}", self.origin))
            .header("origin", &self.origin)
    }
    async fn login(&self) -> Response {
        self.post("/ui/api/login")
            .json(&json!({"username":"viewer","password":std::str::from_utf8(PASSWORD).unwrap()}))
            .send()
            .await
            .unwrap()
    }
    async fn cookie(&self) -> String {
        let response = self.login().await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Strict")
                && cookie.contains("Path=/ui/api")
        );
        assert!(!cookie.contains("Secure"));
        cookie.split(';').next().unwrap().to_owned()
    }
    async fn read(&self, cookie: &str, query: Value) -> Response {
        self.post("/ui/api/read")
            .header("cookie", cookie)
            .json(&json!({"project_id":"test-project","query":query}))
            .send()
            .await
            .unwrap()
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn browser_reads_require_browser_authentication() {
    let app = Browser::start(None).await;
    for request in [
        app.post("/ui/api/read"),
        app.post("/ui/api/read").bearer_auth(AGENT_SECRET),
    ] {
        let response = request
            .json(&json!({"project_id":null,"query":{"kind":"projects"}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn browser_cookie_reads_are_separate_from_agent_writes_and_revoked_at_logout() {
    let app = Browser::start(None).await;
    let before: Value = app
        .client
        .get(format!("{}/ui/api/session", app.origin))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before, json!({"authenticated":false}));
    let cookie = app.cookie().await;
    let session: Value = app
        .client
        .get(format!("{}/ui/api/session", app.origin))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        session,
        json!({"authenticated":true,"username":"viewer","default_project_id":null,"global_project_reads":true})
    );
    let response = app.read(&cookie, json!({"kind":"projects"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json::<Value>().await.unwrap()["ok"], true);
    for path in ["/api/v1/request", "/mcp", "/api/v1/sos"] {
        let response = app
            .post(path)
            .header("cookie", &cookie)
            .json(&json!({"method":"memory_consolidate"}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} accepted a browser cookie"
        );
    }
    let logout = app
        .post("/ui/api/logout")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);
    assert!(logout.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        app.read(&cookie, json!({"kind":"projects"})).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn browser_read_contract_rejects_mutations_and_unknown_fields() {
    let app = Browser::start(None).await;
    let cookie = app.cookie().await;
    for query in [
        json!({"kind":"memory_add","content":"bad"}),
        json!({"kind":"work_record","observations":[]}),
        json!({"kind":"analytics_inventory"}),
        json!({"kind":"projects","method":"memory_consolidate"}),
        json!({"kind":"eval_list","filter":{"action":"record"}}),
    ] {
        assert_eq!(
            app.read(&cookie, query.clone()).await.status(),
            StatusCode::BAD_REQUEST,
            "accepted {query}"
        );
    }
    let response = app
        .post("/ui/api/read")
        .header("cookie", &cookie)
        .json(
            &json!({"project_id":"test-project","query":{"kind":"projects"},"client_evidence":{}}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn browser_posts_require_same_origin_and_loopback_host_without_forwarded_bypass() {
    let app = Browser::start(None).await;
    let cookie = app.cookie().await;
    for path in ["/ui/api/login", "/ui/api/logout", "/ui/api/read"] {
        for origin in [None, Some("https://evil.example"), Some("null")] {
            let mut request = app
                .client
                .post(format!("{}{path}", app.origin))
                .header("cookie", &cookie);
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            assert_eq!(
                request.json(&json!({})).send().await.unwrap().status(),
                StatusCode::FORBIDDEN,
                "{path} {origin:?}"
            );
        }
    }
    let response = app
        .post("/ui/api/read")
        .header("host", "evil.example")
        .header(
            "x-forwarded-host",
            app.origin.strip_prefix("http://").unwrap(),
        )
        .header("x-forwarded-proto", "https")
        .header("cookie", &cookie)
        .json(&json!({"project_id":null,"query":{"kind":"projects"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn explicit_https_origin_has_secure_cookie_and_exact_authority() {
    let app = Browser::start(Some("https://nexus.example.test")).await;
    let response = app
        .client
        .post(format!("{}/ui/api/login", app.origin))
        .header("host", "nexus.example.test")
        .header("origin", "https://nexus.example.test")
        .json(&json!({"username":"viewer","password":std::str::from_utf8(PASSWORD).unwrap()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("; Secure"));
    assert_eq!(app.login().await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn login_does_not_disclose_whether_username_or_password_was_wrong() {
    let app = Browser::start(None).await;
    let mut bodies = vec![];
    for (username, password) in [
        ("missing", "long browser password only"),
        ("viewer", "wrong browser password only"),
    ] {
        let response = app
            .post("/ui/api/login")
            .json(&json!({"username":username,"password":password}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        bodies.push(response.text().await.unwrap());
    }
    assert_eq!(bodies[0], bodies[1]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_browser_proxy_preserves_selected_project_for_context_bound_reads() {
    let (app, daemon) = local_proxy().await;
    let cookie = app.cookie().await;
    let response = app.read(&cookie, json!({"kind":"sos_list"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value = response.json().await.unwrap();
    daemon.abort();
    assert_eq!(
        result["untriaged_report_count"], 1,
        "selected project was replaced by daemon working-directory inference: {result}"
    );
}

async fn local_proxy() -> (Browser, tokio::task::JoinHandle<anyhow::Result<()>>) {
    nexus::runtime::server::disable_in_process_model_providers();
    let temporary = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.storage.database_path = temporary.path().join("nexus.db");
    config.runtime.socket_path = temporary.path().join("s");
    config.runtime.lock_path = temporary.path().join("l");
    config.memory.enabled = false;
    let loaded = LoadedConfig {
        config,
        sources: vec![],
        hash: "proxy-test".into(),
    };
    seed_proxy_sos(&loaded);
    let daemon = tokio::spawn(nexus::runtime::daemon::serve(loaded.clone()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !loaded.config.runtime.socket_path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture daemon did not start"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let credentials = temporary.path().join("web.json");
    configure_credentials(&credentials, "viewer", PASSWORD).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(nexus::runtime::server::serve_browser_listener(
        listener,
        loaded,
        None,
        BrowserConfig::load(&credentials, None).unwrap(),
    ));
    (
        Browser {
            _temporary: temporary,
            origin,
            task,
            client: Client::new(),
        },
        daemon,
    )
}

fn seed_proxy_sos(loaded: &LoadedConfig) {
    use nexus::coordination::api::ServiceRequest;
    let service = NexusService::new(loaded.clone()).unwrap();
    let report = ServiceRequest::decode("sos", json!({"action":"report","agent":"app","session_id":"app-one","model":"test-model","message":"A recorded unusual situation","type":"input","environment":"test","version":"1","trace":"source","metadata":{}})).unwrap();
    let response = serde_json::to_value(service.handle_remote("test-project", report)).unwrap();
    assert_eq!(response["ok"], true);
}

#[tokio::test]
async fn browser_assets_are_whitelisted_and_route_reloads_keep_the_same_security_headers() {
    let app = Browser::start(None).await;
    for path in ["/", "/ui/work", "/ui/evals"] {
        let response = app
            .client
            .get(format!("{}{path}", app.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "missing SPA route {path}"
        );
        assert!(response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert!(response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'"));
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    }
    for path in [
        "/ui/not-a-page",
        "/assets/not-a-file.js",
        "/assets/%2e%2e/Cargo.toml",
        "/Cargo.toml",
    ] {
        let response = app
            .client
            .get(format!("{}{path}", app.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "unlisted resource {path} was served"
        );
    }
}

async fn read_json(app: &Browser, cookie: &str, query: Value) -> Value {
    app.read(cookie, query).await.json().await.unwrap()
}

#[tokio::test]
async fn browser_reads_command_histograms_for_a_category() {
    let app = Browser::start(None).await;
    let observed = app
        .client
        .post(format!("{}/api/v1/request", app.origin))
        .bearer_auth(AGENT_SECRET)
        .json(
            &json!({"project_id":"test-project","method":"post_tool_use","params":{
            "session_id":"s","agent":"codex","tool_use_id":"read-files","tool_name":"Bash",
            "tool_input":{"command":"cat README.md; tail -n 5 log.txt; cat Cargo.toml"}}}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(observed.status(), StatusCode::OK);
    let cookie = app.cookie().await;

    let query = json!({"kind":"analytics_commands","category_id":"files.read"});
    let histogram = read_json(&app, &cookie, query).await;

    assert_eq!(histogram["category_id"], "files.read", "{histogram}");
    assert_eq!(histogram["bin_seconds"], 21_600);
    let commands = histogram["commands"].as_array().unwrap();
    let totals: Vec<_> = commands
        .iter()
        .map(|command| (command["name"].clone(), command["count"].clone()))
        .collect();
    assert_eq!(
        totals,
        [(json!("cat"), json!(2)), (json!("tail"), json!(1))]
    );
    let bins = commands[0]["bins"].as_array().unwrap();
    assert_eq!(bins.len(), 28);
    assert_eq!(bins[0]["start"], histogram["window_start"]);
    assert_eq!(bins[27]["count"], 2);
}

#[tokio::test]
async fn browser_skill_reads_are_null_until_text_is_captured_and_inputs_are_closed() {
    let app = Browser::start(None).await;
    let cookie = app.cookie().await;

    let skill = read_json(
        &app,
        &cookie,
        json!({"kind":"skill_get","name":"never-captured"}),
    )
    .await;

    assert_eq!(
        skill,
        json!({"ok":true,"name":"never-captured","description":null,"content":null,
            "content_hash":null,"source_path":null,"observed_at":null})
    );
    for query in [
        json!({"kind":"analytics_commands"}),
        json!({"kind":"analytics_commands","category_id":"Not A Slug"}),
        json!({"kind":"skill_get","name":""}),
    ] {
        let response = app.read(&cookie, query.clone()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
    }
}

#[tokio::test]
async fn browser_read_requests_are_bounded_and_project_bound_reads_require_selection() {
    let app = Browser::start(None).await;
    let cookie = app.cookie().await;
    let oversized = app.post("/ui/api/read").header("cookie", &cookie)
        .json(&json!({"project_id":"test-project","query":{"kind":"memory_search","regex":"x".repeat(65536)}})).send().await.unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    for kind in [
        "sos_list",
        "work_dashboard",
        "eval_dashboard",
        "concern_dashboard",
        "memory_status",
    ] {
        let response = app
            .post("/ui/api/read")
            .header("cookie", &cookie)
            .json(&json!({"project_id":null,"query":{"kind":kind}}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "unbound {kind} inferred the server project"
        );
    }
}
