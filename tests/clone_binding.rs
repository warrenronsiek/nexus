// @boundary project-toml: preserve caller-owned settings while updating only a project binding.
// @feature installation
// @feature coordination
// @spec docs/features/installation.md
// @spec docs/features/coordination.md
use std::path::Path;
use std::process::Command;

#[test]
fn independent_clone_can_share_a_project_without_copying_credentials() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    let target = fixture.path().join("clone");
    for root in [&source, &target] {
        std::fs::create_dir_all(root).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
    }
    let configuration = fixture.path().join("user.toml");
    std::fs::write(&configuration, "schema_version=1\n[client]\ntransport='remote'\nendpoint='http://127.0.0.1:7337'\nproject_id='default-project'\nsecret_file='/private/client-secret'\n").unwrap();
    std::fs::write(source.join(".git/nexus.toml"), "schema_version=1\n[client]\nproject_id='shared-project'\nsecret_file='/private/source-secret'\n").unwrap();
    let target_config = target.join(".git/nexus.toml");
    std::fs::write(&target_config, "schema_version=1\n[client]\nproject_id='separate-project'\n[coordination]\nclaim_ttl_seconds=123\n").unwrap();
    let output = run(&configuration, &target, &source);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let updated: toml::Value =
        toml::from_str(&std::fs::read_to_string(&target_config).unwrap()).unwrap();
    assert_eq!(
        updated["client"]["project_id"].as_str(),
        Some("shared-project")
    );
    assert!(updated["client"].get("secret_file").is_none());
    assert_eq!(
        updated["coordination"]["claim_ttl_seconds"].as_integer(),
        Some(123)
    );
    let before = std::fs::read(&target_config).unwrap();
    std::fs::write(source.join(".git/nexus.toml"), "schema_version=1\n[client]\nendpoint='http://127.0.0.1:7338'\nproject_id='other-service-project'\n").unwrap();
    let rejected = run(&configuration, &target, &source);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("different Nexus services"));
    assert_eq!(std::fs::read(&target_config).unwrap(), before);
}

fn run(configuration: &Path, root: &Path, source: &Path) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
    for (key, _) in
        std::env::vars().filter(|(key, _)| key.starts_with("NEXUS_") || key.starts_with("GIT_"))
    {
        command.env_remove(key);
    }
    command
        .args([
            "--config",
            configuration.to_str().unwrap(),
            "install",
            "--share-project-with",
            source.to_str().unwrap(),
        ])
        .current_dir(root)
        .output()
        .unwrap()
}
