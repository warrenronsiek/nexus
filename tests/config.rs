// @feature configuration
// @feature agent-memory
// @spec docs/features/configuration.md
// @spec docs/features/agent-memory.md
use nexus::config::{ClientConfig, Config, ModelProvider, RemoteClientConfig};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn configuration_files_are_regular_bounded_and_nonblocking() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let temporary = tempfile::tempdir().unwrap();
    let fifo = temporary.path().join("config.fifo");
    let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
    let started = Instant::now();
    let error = Config::load(Some(&fifo), None).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(error.to_string().contains("regular file"), "{error:#}");

    let oversized = temporary.path().join("oversized.toml");
    std::fs::write(&oversized, " ".repeat(1024 * 1024 + 1)).unwrap();
    let error = Config::load(Some(&oversized), None).unwrap_err();
    assert!(
        error.to_string().contains("1048576-byte limit"),
        "{error:#}"
    );

    let target = temporary.path().join("target.toml");
    let symlink = temporary.path().join("linked.toml");
    std::fs::write(&target, "schema_version = 1\n").unwrap();
    std::os::unix::fs::symlink(&target, &symlink).unwrap();
    assert_eq!(
        Config::load(Some(&symlink), None).unwrap().sources,
        vec![symlink]
    );
}

#[test]
fn review_defaults_allow_deep_cross_model_analysis() {
    let review = Config::default().review;

    assert_eq!(review.timeout_seconds, 600);
    assert_eq!(review.codex.reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(review.claude.reasoning_effort.as_deref(), Some("xhigh"));
}

#[test]
fn user_choices_load_from_toml_and_environment_has_precedence() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
schema_version = 1
[coordination]
claim_ttl_seconds = 321
[analyst]
enabled = true
provider = "claude"
[analyst.claude]
command = "custom-claude"
model = "configured-model"
"#,
    )
    .unwrap();

    std::env::set_var("NEXUS_ANALYST_PROVIDER", "codex");
    std::env::set_var("NEXUS_IMPLEMENTATION_PROVIDER", "claude");
    std::env::set_var("NEXUS_REVIEW_PROVIDER", "codex");
    let loaded = Config::load(Some(&path), None).unwrap();
    std::env::remove_var("NEXUS_ANALYST_PROVIDER");
    std::env::remove_var("NEXUS_IMPLEMENTATION_PROVIDER");
    std::env::remove_var("NEXUS_REVIEW_PROVIDER");
    assert_eq!(loaded.config.coordination.claim_ttl_seconds, 321);
    assert!(loaded.config.analyst.enabled);
    assert_eq!(loaded.config.analyst.provider, ModelProvider::Codex);
    assert_eq!(
        loaded.config.review.implementation_provider,
        ModelProvider::Claude
    );
    assert_eq!(loaded.config.review.provider, ModelProvider::Codex);
    assert_eq!(loaded.config.analyst.claude.command, "custom-claude");
    assert_eq!(
        loaded.config.analyst.claude.model.as_deref(),
        Some("configured-model")
    );
    assert_eq!(loaded.sources, vec![path]);
    assert_eq!(loaded.hash.len(), 64);
}

#[test]
fn unknown_and_invalid_properties_are_rejected() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let unknown = temp.path().join("unknown.toml");
    std::fs::write(&unknown, "schema_version = 1\nblocking = true\n").unwrap();
    let error = Config::load(Some(&unknown), None).unwrap_err().to_string();
    assert!(error.contains("invalid Nexus configuration"));

    let invalid = temp.path().join("invalid.toml");
    std::fs::write(
        &invalid,
        "schema_version = 1\n[coordination]\nclaim_ttl_seconds = 0\n",
    )
    .unwrap();
    let error = Config::load(Some(&invalid), None).unwrap_err().to_string();
    assert!(error.contains("claim_ttl_seconds must be positive"));

    let zero_reconcile = temp.path().join("zero-reconcile.toml");
    std::fs::write(
        &zero_reconcile,
        "schema_version = 1\n[coordination]\nreconcile_seconds = 0\n",
    )
    .unwrap();
    let error = Config::load(Some(&zero_reconcile), None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("reconcile_seconds must be positive"));

    let mut same_provider = Config::default();
    same_provider.review.provider = same_provider.review.implementation_provider;
    let error = same_provider.validate().unwrap_err().to_string();
    assert!(error.contains("review.provider must differ"));
}

#[test]
fn configured_durations_reject_values_that_cannot_be_safely_scheduled() {
    let mut claim_ttl = Config::default();
    claim_ttl.coordination.claim_ttl_seconds = i64::MAX;
    assert!(claim_ttl
        .validate()
        .unwrap_err()
        .to_string()
        .contains("coordination.claim_ttl_seconds must be at most"));

    let mut reconcile = Config::default();
    reconcile.coordination.reconcile_seconds = u64::MAX;
    assert!(reconcile
        .validate()
        .unwrap_err()
        .to_string()
        .contains("coordination.reconcile_seconds must be at most"));

    let mut memory = Config::default();
    memory.memory.consolidation_interval_seconds = u64::MAX;
    assert!(memory
        .validate()
        .unwrap_err()
        .to_string()
        .contains("memory.consolidation_interval_seconds must be at most"));

    let mut analyst = Config::default();
    analyst.analyst.timeout_seconds = u64::MAX;
    assert!(analyst
        .validate()
        .unwrap_err()
        .to_string()
        .contains("analyst.timeout_seconds must be at most"));

    let mut review = Config::default();
    review.review.timeout_seconds = u64::MAX;
    assert!(review
        .validate()
        .unwrap_err()
        .to_string()
        .contains("review.timeout_seconds must be at most"));

    let mut ui = Config::default();
    ui.ui.refresh_interval_ms = u64::MAX;
    assert!(ui
        .validate()
        .unwrap_err()
        .to_string()
        .contains("ui.refresh_interval_ms must be at most"));
}

#[test]
fn ui_configuration_defaults_to_bounded_terminal_polling() {
    let config = Config::default();

    assert_eq!(config.ui.refresh_interval_ms, 2_000);
    assert_eq!(config.ui.recent_event_limit, 64);
    assert_eq!(config.ui.recent_record_limit, 100);
    config.validate().unwrap();

    let mut no_refresh = config;
    no_refresh.ui.refresh_interval_ms = 0;
    assert!(no_refresh
        .validate()
        .unwrap_err()
        .to_string()
        .contains("refresh_interval_ms must be positive"));
}

#[test]
fn memory_configuration_defaults_to_bounded_background_consolidation() {
    let config = Config::default();

    assert!(config.memory.enabled);
    assert_eq!(config.memory.consolidation_interval_seconds, 15);
    assert_eq!(config.memory.consolidation_batch_size, 8);
    assert_eq!(config.memory.context_max_items, 64);
    assert_eq!(config.memory.context_max_bytes, 16 * 1024);
    config.validate().unwrap();

    let mut invalid = config;
    invalid.memory.consolidation_interval_seconds = 0;
    assert!(invalid
        .validate()
        .unwrap_err()
        .to_string()
        .contains("memory.consolidation_interval_seconds must be positive"));

    let mut excessive_items = Config::default();
    excessive_items.memory.context_max_items = 65;
    assert!(excessive_items
        .validate()
        .unwrap_err()
        .to_string()
        .contains("memory.context_max_items must be at most 64"));

    let mut excessive_bytes = Config::default();
    excessive_bytes.memory.context_max_bytes = 16 * 1024 + 1;
    assert!(excessive_bytes
        .validate()
        .unwrap_err()
        .to_string()
        .contains("memory.context_max_bytes must be at most 16384"));

    let mut zero_analyst_timeout = Config::default();
    zero_analyst_timeout.analyst.timeout_seconds = 0;
    assert!(zero_analyst_timeout
        .validate()
        .unwrap_err()
        .to_string()
        .contains("analyst.timeout_seconds must be positive"));
}

#[test]
fn remote_client_configuration_is_typed_and_environment_selectable() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let secret_file = temp.path().join("nexus.secret");
    let config_file = temp.path().join("config.toml");
    std::fs::write(
        &config_file,
        format!(
            r#"
schema_version = 1
[client]
transport = "remote"
endpoint = "https://nexus.example.test:7443"
project_id = "shared-nexus"
secret_file = "{}"
"#,
            secret_file.display()
        ),
    )
    .unwrap();

    let loaded = Config::load(Some(&config_file), None).unwrap();
    let ClientConfig::Remote(remote) = &loaded.config.client else {
        panic!("expected remote client configuration");
    };
    assert_eq!(remote.endpoint, "https://nexus.example.test:7443");
    assert_eq!(remote.project_id, "shared-nexus");
    assert_eq!(remote.secret_file.as_deref(), Some(secret_file.as_path()));

    std::env::set_var("NEXUS_ENDPOINT", "http://127.0.0.1:7337");
    std::env::set_var("NEXUS_PROJECT_ID", "environment-project");
    std::env::set_var("NEXUS_CLIENT_SECRET_FILE", &secret_file);
    let loaded = Config::load(None, None).unwrap();
    std::env::remove_var("NEXUS_ENDPOINT");
    std::env::remove_var("NEXUS_PROJECT_ID");
    std::env::remove_var("NEXUS_CLIENT_SECRET_FILE");

    let ClientConfig::Remote(remote) = loaded.config.client else {
        panic!("expected environment to select remote transport");
    };
    assert_eq!(remote.endpoint, "http://127.0.0.1:7337");
    assert_eq!(remote.project_id, "environment-project");
    assert_eq!(remote.secret_file.as_deref(), Some(secret_file.as_path()));
}

#[test]
fn remote_client_endpoint_must_be_an_http_url_without_embedded_credentials() {
    for endpoint in [
        "http://",
        "http:// user.example.test",
        "http://user:password@nexus.example.test",
        "https://nexus.example.test/#fragment",
    ] {
        let config = Config {
            client: ClientConfig::Remote(RemoteClientConfig {
                endpoint: endpoint.into(),
                project_id: "shared".into(),
                secret_file: None,
            }),
            ..Config::default()
        };
        assert!(config.validate().is_err(), "accepted endpoint {endpoint:?}");
    }
}

#[test]
fn remote_project_ids_reject_control_characters() {
    let config = Config {
        client: ClientConfig::Remote(RemoteClientConfig {
            endpoint: "https://nexus.example.test".into(),
            project_id: "shared\0project".into(),
            secret_file: None,
        }),
        ..Config::default()
    };

    assert!(config
        .validate()
        .unwrap_err()
        .to_string()
        .contains("client.project_id must not contain control characters"));
}

#[test]
fn server_credentials_do_not_select_or_leak_into_client_configuration() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let configuration = temporary.path().join("config.toml");
    std::fs::write(&configuration, "schema_version = 1\n").unwrap();
    let secret = "server-secret-that-must-not-enter-config";
    let secret_file = "/run/secrets/nexus-server";

    std::env::set_var("NEXUS_SECRET", secret);
    std::env::set_var("NEXUS_SECRET_FILE", secret_file);
    let loaded = Config::load(Some(&configuration), None).unwrap();
    std::env::remove_var("NEXUS_SECRET");
    std::env::remove_var("NEXUS_SECRET_FILE");

    assert_eq!(loaded.config.client, ClientConfig::Local);
    let serialized = toml::to_string(&loaded.config).unwrap();
    assert!(!serialized.contains(secret));
    assert!(!serialized.contains(secret_file));
}

#[test]
fn layered_configuration_can_switch_client_transport_in_either_direction() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(temporary.path())
        .arg("init")
        .status()
        .unwrap()
        .success());
    let user = temporary.path().join("user.toml");
    let project = temporary.path().join(".git/nexus.toml");

    std::fs::write(
        &user,
        r#"[client]
transport = "remote"
endpoint = "https://nexus.example.test"
project_id = "shared"
"#,
    )
    .unwrap();
    std::fs::write(&project, "[client]\ntransport = \"local\"\n").unwrap();
    let loaded = Config::load(Some(&user), Some(temporary.path())).unwrap();
    assert_eq!(loaded.config.client, ClientConfig::Local);

    std::fs::write(&user, "[client]\ntransport = \"local\"\n").unwrap();
    std::fs::write(
        &project,
        r#"[client]
transport = "remote"
endpoint = "https://project.example.test"
project_id = "project-shared"
"#,
    )
    .unwrap();
    let loaded = Config::load(Some(&user), Some(temporary.path())).unwrap();
    let ClientConfig::Remote(remote) = loaded.config.client else {
        panic!("project configuration should select remote transport");
    };
    assert_eq!(remote.endpoint, "https://project.example.test");
    assert_eq!(remote.project_id, "project-shared");
}
