// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary isolated-release-fixtures: exercise version identity and verified local update assets without live installation or service restarts.
use std::process::Command;

#[test]
fn installed_version_identifies_the_exact_source_revision() {
    let source = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(source.status.success());
    let revision = String::from_utf8(source.stdout).unwrap();
    let version = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    let text = String::from_utf8(version.stdout).unwrap();
    assert!(
        text.contains(revision.trim()),
        "source revision absent: {text}"
    );
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
}

#[test]
fn update_help_exposes_check_and_explicit_compose_adoption() {
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["update", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--check"));
    assert!(help.contains("--compose-file"));
    assert!(help.contains("--force"));
}

#[test]
fn update_asset_refresh_supports_a_headless_remote_installation() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.toml");
    std::fs::write(&config,"schema_version = 1\n[client]\ntransport = \"remote\"\nendpoint = \"http://127.0.0.1:7337\"\nproject_id = \"fixture\"\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .arg("--config")
        .arg(&config)
        .args(["setup", "--refresh"])
        .env("HOME", directory.path())
        .env("CODEX_HOME", directory.path().join("codex"))
        .env("CLAUDE_CONFIG_DIR", directory.path().join("claude"))
        .env("PATH", "")
        .env_remove("NEXUS_ENDPOINT")
        .env_remove("NEXUS_PROJECT_ID")
        .env_remove("NEXUS_CLIENT_SECRET_FILE")
        .env_remove("NEXUS_CONFIG")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["hosts"], serde_json::json!([]));
    assert_eq!(report["database"], "remote");
    assert!(directory
        .path()
        .join("codex/skills/nexus-setup/SKILL.md")
        .is_file());
}
