// @feature observability-ui
// @feature installation
// @feature runtime
// @spec docs/features/observability-ui.md
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @entrypoint native_terminal_opens_without_pi_and_manages_memory
use std::process::Command;

#[test]
fn native_terminal_opens_without_pi_and_manages_memory() {
    exercise_terminal("system", "quit");
}

#[test]
fn native_terminal_uses_cached_runtime_with_no_node_on_path() {
    exercise_terminal("cache", "quit");
}

#[test]
fn interrupted_terminal_restores_settings() {
    exercise_terminal("system", "interrupt");
    exercise_terminal("system", "terminate");
}

fn exercise_terminal(runtime_source: &str, exit_mode: &str) {
    let output = Command::new("python3")
        .arg("tests/tui_terminal.py")
        .arg(env!("CARGO_BIN_EXE_nexus"))
        .arg(runtime_source)
        .arg(exit_mode)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
