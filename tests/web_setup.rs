// @feature installation
// @feature observability-ui
// @spec docs/features/installation.md
// @spec docs/features/observability-ui.md
// @boundary isolated-cli-fixture: Python drives the actual binary with private temporary homes and no provider or service access.
use std::process::Command;

#[test]
fn setup_provisions_browser_credentials_without_changing_them_on_refresh() {
    let output = Command::new("python3")
        .args(["tests/web_setup.py", env!("CARGO_BIN_EXE_nexus")])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
