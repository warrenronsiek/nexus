// @feature installation
// @spec docs/features/installation.md
// @boundary isolated-environment: a child test process supplies a headless fake home without changing sibling tests.
use nexus::installation::{refresh_machine, setup_machine};
use std::path::Path;
use std::process::Command;

fn assert_headless_refresh() {
    let config = std::env::var_os("NEXUS_TEST_REFRESH_CONFIG").unwrap();
    let report = refresh_machine(Path::new("/fixture/nexus"), Some(Path::new(&config))).unwrap();
    assert!(report.hosts.is_empty());
    assert_eq!(report.database, "remote");
    assert!(setup_machine(Path::new("/fixture/nexus"), Some(Path::new(&config))).is_err());
    let codex = std::env::var_os("CODEX_HOME").unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(&codex).join("skills/nexus/SKILL.md")).unwrap(),
        include_str!("../skills/nexus/SKILL.md")
    );
}

fn isolated_refresh_command(root: &Path) -> Command {
    let config = root.join("config.toml");
    std::fs::write(&config, "schema_version = 1\n[client]\ntransport = \"remote\"\nendpoint = \"http://127.0.0.1:7337\"\nproject_id = \"fixture\"\n").unwrap();
    let codex = root.join("codex");
    let skill = codex.join("skills/nexus/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, "stale").unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "asset_refresh_replaces_stale_skills_without_an_installed_host",
            "--nocapture",
        ])
        .env("NEXUS_TEST_REFRESH_CHILD", "1")
        .env("NEXUS_TEST_REFRESH_CONFIG", &config)
        .env("HOME", root)
        .env("CODEX_HOME", codex)
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("PATH", "")
        .env_remove("NEXUS_ENDPOINT")
        .env_remove("NEXUS_PROJECT_ID")
        .env_remove("NEXUS_CONFIG");
    command
}

#[test]
fn asset_refresh_replaces_stale_skills_without_an_installed_host() {
    if std::env::var_os("NEXUS_TEST_REFRESH_CHILD").is_some() {
        assert_headless_refresh();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let output = isolated_refresh_command(directory.path()).output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
