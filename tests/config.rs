// @feature configuration
// @feature agent-memory
// @spec docs/features/configuration.md
// @spec docs/features/agent-memory.md
use nexus::config::{Config, ModelProvider};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    let mut same_provider = Config::default();
    same_provider.review.provider = same_provider.review.implementation_provider;
    let error = same_provider.validate().unwrap_err().to_string();
    assert!(error.contains("review.provider must differ"));
}

#[test]
fn ui_configuration_defaults_to_a_valid_local_dashboard() {
    let config = Config::default();

    assert!(config.ui.enabled);
    assert_eq!(
        config.ui.bind_address,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7337)
    );
    assert_eq!(config.ui.refresh_interval_ms, 2_000);
    assert_eq!(config.ui.recent_event_limit, 500);
    assert_eq!(config.ui.recent_record_limit, 100);
    config.validate().unwrap();

    let mut remote = config.clone();
    remote.ui.bind_address = "0.0.0.0:7337".parse().unwrap();
    assert!(remote
        .validate()
        .unwrap_err()
        .to_string()
        .contains("loopback"));

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
