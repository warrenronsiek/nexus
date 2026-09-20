// @feature configuration
// @spec docs/features/configuration.md
use nexus::config::{Config, ModelProvider};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

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
