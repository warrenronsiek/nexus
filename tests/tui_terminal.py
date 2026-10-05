# @feature observability-ui
# @feature installation
# @feature runtime
# @spec docs/features/observability-ui.md
# @spec docs/features/installation.md
# @spec docs/features/runtime.md
# @entrypoint exercise_terminal_dashboard
# @boundary child-process-json
"""Exercise the installed dashboard in a real terminal with no Pi on PATH."""

import fcntl
import json
import os
from pathlib import Path
import pty
import platform
import select
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time


def wait_for_text(process: subprocess.Popen[bytes], master: int, expected: bytes, timeout_seconds: float = 15) -> None:
    output = bytearray()
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        readable, _, _ = select.select([master], [], [], 0.1)
        if readable:
            output.extend(os.read(master, 65536))
            if expected in output:
                return
        if process.poll() is not None:
            break
    raise AssertionError(f"Missing {expected!r}; exit={process.poll()}; output={output[-6000:]!r}")


def isolated_project(temporary: Path) -> tuple[Path, Path, dict[str, str]]:
    node = shutil.which("node")
    git = shutil.which("git")
    assert node is not None and git is not None, "Repository checks require Node and Git"
    root = temporary / "project with spaces"
    state = temporary / "state"
    executables = temporary / "bin"
    for path in (root, state, executables):
        path.mkdir()
    (executables / "node").symlink_to(node)
    (executables / "git").symlink_to(git)
    environment = {key: value for key, value in os.environ.items() if not key.startswith("NEXUS_")}
    environment.update(PATH=str(executables), HOME=str(temporary), NEXUS_STATE_DIR=str(state))
    assert shutil.which("pi", path=environment["PATH"]) is None
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    config = temporary / "terminal config.toml"
    config.write_text(
        f'schema_version = 1\n[storage]\ndatabase_path = "{state}/configured.db"\n'
        f'[runtime]\nsocket_path = "{state}/configured.sock"\nlock_path = "{state}/configured.lock"\n'
        f'[ui]\nbind_address = "127.0.0.1:{port}"\n'
        '[memory]\nconsolidation_interval_seconds = 3600\n'
    )
    return root, config, environment


def assert_created_note(binary: str, config: Path, root: Path, environment: dict[str, str]) -> None:
    searched = subprocess.run(
        [binary, "--config", str(config), "memory", "search", "--scope", "project", "created from the native terminal"],
        cwd=root, env=environment, capture_output=True, check=True, timeout=5,
    )
    result = json.loads(searched.stdout)
    assert result["ok"] is True and len(result["memories"]) == 1, result
    assert result["memories"][0]["provenance"]["agent"] == "nexus-tui", result


def select_runtime(environment: dict[str, str], source: str) -> None:
    executables = Path(environment["PATH"])
    node = (executables / "node").resolve()
    if source == "system":
        return
    (executables / "node").unlink()
    if source == "cache":
        operating_system = "darwin" if sys.platform == "darwin" else "linux"
        architecture = "arm64" if platform.machine() in ("arm64", "aarch64") else "x64"
        managed = Path(environment["NEXUS_STATE_DIR"]) / "tui" / f"node-v24.21.0-{operating_system}-{architecture}" / "bin/node"
        managed.parent.mkdir(parents=True)
        managed.symlink_to(node)
    elif source == "download":
        for command in ("curl", "tar", "gzip"):
            executable = shutil.which(command)
            assert executable is not None, f"Provisioning requires {command}"
            (executables / command).symlink_to(executable)
    else:
        raise ValueError(f"Unknown runtime fixture: {source}")
    assert shutil.which("node", path=environment["PATH"]) is None


def close_dashboard(process: subprocess.Popen[bytes], master: int, exit_mode: str) -> None:
    if exit_mode == "quit":
        os.write(master, b"q")
    elif exit_mode == "interrupt":
        os.write(master, b"\x03")
    elif exit_mode == "terminate":
        os.kill(process.pid, signal.SIGTERM)
    else:
        raise ValueError(f"Unknown terminal exit fixture: {exit_mode}")
    # Terminal restoration writes output; keep consuming it until the process exits.
    deadline = time.monotonic() + 8
    while process.poll() is None and time.monotonic() < deadline:
        readable, _, _ = select.select([master], [], [], 0.1)
        if readable:
            os.read(master, 65536)
    assert process.wait(timeout=1) == 0


def exercise_terminal_dashboard(binary: str, runtime_source: str, exit_mode: str) -> None:
    with tempfile.TemporaryDirectory(prefix="nexus-tui-") as directory:
        root, config, environment = isolated_project(Path(directory))
        select_runtime(environment, runtime_source)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 0, 0))
        original_settings = termios.tcgetattr(slave)
        process = subprocess.Popen(
            [binary, "--config", str(config), "tui"],
            cwd=root, env=environment, stdin=slave, stdout=slave, stderr=slave,
            start_new_session=True,
        )
        try:
            wait_for_text(process, master, b"Coordination", 150 if runtime_source == "download" else 15)
            added = subprocess.run(
                [binary, "--config", str(config), "memory", "add", "--scope", "project", "terminal fixture memory"],
                cwd=root, env=environment, capture_output=True, check=True, timeout=5,
            )
            assert json.loads(added.stdout)["ok"] is True
            os.write(master, b"\t\t\t")
            wait_for_text(process, master, b"terminal fixture memory")
            os.write(master, b"/")
            wait_for_text(process, master, b"Search memory")
            os.write(master, b"terminal.*fixture\r")
            wait_for_text(process, master, b"Search results")
            os.write(master, b"a")
            wait_for_text(process, master, b"Memory scope")
            os.write(master, b"\r")
            wait_for_text(process, master, b"Add immutable memory")
            os.write(master, b"created from the native terminal\r")
            wait_for_text(process, master, b"created from the native terminal")
            close_dashboard(process, master, exit_mode)
            mask = termios.ICANON | termios.ECHO
            assert termios.tcgetattr(slave)[3] & mask == original_settings[3] & mask
            assert_created_note(binary, config, root, environment)
        finally:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            if process.poll() is None:
                process.wait(timeout=5)
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    exercise_terminal_dashboard(sys.argv[1], sys.argv[2], sys.argv[3])
