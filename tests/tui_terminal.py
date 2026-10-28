# @feature work-history
# @spec docs/features/work-history.md
# @feature code-concerns
# @spec docs/features/code-concerns.md
# @feature usage-analytics
# @spec docs/features/usage-analytics.md
# @feature evals
# @spec docs/features/evals.md
# @feature papercuts
# @spec docs/features/papercuts.md
# @feature observability-ui
# @feature installation
# @feature runtime
# @spec docs/features/observability-ui.md
# @spec docs/features/installation.md
# @spec docs/features/runtime.md
# @entrypoint exercise_terminal_dashboard
# @boundary child-process-json
"""Exercise the installed dashboard in a real terminal with no Pi on PATH."""

from __future__ import annotations

import fcntl
import json
import os
from pathlib import Path
import pty
import platform
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


def wait_for_text(process: subprocess.Popen[bytes], master: int, expected: bytes | tuple[bytes, ...], timeout_seconds: float = 15) -> None:
    output = bytearray()
    expected_texts = expected if isinstance(expected, tuple) else (expected,)
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        readable, _, _ = select.select([master], [], [], 0.1)
        if readable:
            output.extend(os.read(master, 65536))
            if all(text in output for text in expected_texts):
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
    environment.update(PATH=str(executables), HOME=str(temporary), NEXUS_STATE_DIR=str(state),
                       XDG_CACHE_HOME=str(temporary / "cache"), XDG_CONFIG_HOME=str(temporary / "config"))
    assert shutil.which("pi", path=environment["PATH"]) is None
    config = temporary / "terminal config.toml"
    config.write_text(
        f'schema_version = 1\n[storage]\ndatabase_path = "{state}/configured.db"\n'
        f'[runtime]\nsocket_path = "{state}/configured.sock"\nlock_path = "{state}/configured.lock"\n'
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
        cache = Path(environment["HOME"]) / "Library/Caches" if operating_system == "darwin" else Path(environment["XDG_CACHE_HOME"])
        managed = cache / "nexus/tui" / f"node-v24.21.0-{operating_system}-{architecture}" / "bin/node"
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


def exercise_memory_actions(process: subprocess.Popen[bytes], master: int) -> None:
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


def exercise_papercuts(binary: str, config: Path, root: Path, environment: dict[str, str], terminal: tuple[int, subprocess.Popen[bytes]]) -> None:
    master, process = terminal
    reported = subprocess.run(
        [binary, "--config", str(config), "papercuts", "report", "Terminal papercut fixture", "--description", "Inspect the complete description"],
        cwd=root, env=environment, capture_output=True, check=True, timeout=5,
    )
    assert json.loads(reported.stdout)["ok"] is True
    os.write(master, b"\t")
    wait_for_text(process, master, b"Terminal papercut fixture")
    os.write(master, b"\r")
    wait_for_text(process, master, b"Inspect the complete description")


def exercise_analytics(process: subprocess.Popen[bytes], master: int) -> None:
    os.write(master, b"\t")
    wait_for_text(process, master, b"1 Activities")
    os.write(master, b"2")
    wait_for_text(process, master, b"Page 1")
    os.write(master, b"4")
    wait_for_text(process, master, b"No usage observed in this window.")


def eval_chart_records() -> list[dict[str, object]]:
    """A caller's latency metric with three independent completed runs."""
    occurred = "2026-10-22T12:00:00Z"
    records: list[dict[str, object]] = []

    def revision(identifier: str, kind: str, body: dict[str, object]) -> None:
        records.append({"id": identifier, "title": identifier, "occurred_at": occurred,
                        "body": {"kind": kind, "identity": identifier, "parents": [],
                                 "change_id": "native-eval-change", **body}})

    revision("native-repo", "repository", {"repository": "fixture-repository", "commit": "a" * 40, "object_format": "sha1"})
    revision("native-model", "model", {"label": "Router model", "provider": "fixture", "requested_model": "router-v1"})
    revision("native-case", "case", {"expected_behavior": "Route the input"})
    revision("native-suite", "suite", {"cases": [{"case_id": "native-case", "grader_ids": [], "weight": 1,
                                                  "split": "development"}], "aggregation": "Caller defined"})
    revision("native-target", "target", {"components": [{"id": "router", "role": "dispatch", "repository_id": "native-repo",
                                                         "model": {"assignment": "fixed", "model_id": "native-model"}}]})
    revision("native-latency", "metric", {"name": "Router latency", "description": "Elapsed request time", "unit": "seconds",
             "value_format": "decimal", "calculation": {"description": "Caller monotonic elapsed time",
             "population": "one run", "aggregation": "none", "exclusions": "none", "missing_data": "unavailable"}})
    for index, value in enumerate(("10", "8", "7")):
        run = f"native-run-{index}"
        timestamp = f"2026-10-{22 + index}T12:00:00Z"
        records.append({"id": run, "title": run, "occurred_at": timestamp, "body": {"kind": "run",
                        "target_id": "native-target", "suite_id": "native-suite", "runner": "fixture",
                        "runner_version": "1", "state": "completed", "selection_rule": "all"}})
        records.append({"id": f"native-result-{index}", "title": f"Result {index}", "occurred_at": timestamp,
                        "body": {"kind": "measurement", "metric_id": "native-latency", "subject": {"scope": "run", "id": run},
                                 "value": {"storage": "inline", "text": value}, "explanation": "Caller result"}})
    return records


def exercise_evals(binary: str, config: Path, root: Path, environment: dict[str, str], terminal: tuple[int, subprocess.Popen[bytes]]) -> None:
    master, process = terminal
    occurred = "2026-10-22T12:00:00Z"
    records = [
        {"id": "native-eval-change", "title": "Native evaluation change", "occurred_at": occurred,
         "body": {"kind": "change", "motivation": {
             "state": "recorded", "reason": "observed_failure", "problem": "A router ignored a supplied tool.",
             "expected_behavior": "Select an appropriate supplied tool.",
             "rationale": "Distinct router regression reason", "provenance": "contemporaneous"},
             "evidence": [{"kind": "record", "id": "native-git-evidence"}]}},
        {"id": "native-git-evidence", "title": "Native Git evidence", "occurred_at": occurred,
         "body": {"kind": "artifact", "media_type": "image/png", "byte_size": 900000, "digest": None,
                  "backings": [{"kind": "git_file", "repository": "fixture-repository", "commit": "a" * 40,
                                "object_format": "sha1", "path": "images/router-fixture.png", "blob_oid": None,
                                "entry_mode": "100644"}]}},
    ]
    records.extend(eval_chart_records())
    fixture = root / "native eval records.json"
    fixture.write_text(json.dumps({"format_version": 1, "records": records}))
    recorded = subprocess.run(
        [binary, "--config", str(config), "eval", "record", str(fixture)],
        cwd=root, env=environment, capture_output=True, check=True, timeout=5,
    )
    assert json.loads(recorded.stdout)["ok"] is True
    os.write(master, b"\t")
    wait_for_text(process, master, (b"Evaluation sets", b"native-suite"))
    os.write(master, b"\r")
    wait_for_text(process, master, (b"Router latency", b"seconds", "●".encode()))
    os.write(master, b"b")
    wait_for_text(process, master, b"records loaded")
    os.write(master, b"t")
    wait_for_text(process, master, b"Native evaluation change")
    os.write(master, b"\r")
    wait_for_text(process, master, b"Distinct router regression reason")
    os.write(master, b"l")
    wait_for_text(process, master, b"native-git-evidence")
    os.write(master, b"\r")
    wait_for_text(process, master, (b"image/png", b"fixture-repository", b"a" * 40, b"images/router-fixture.png"))
    os.write(master, b"\x1b")
    wait_for_text(process, master, b"Distinct router regression reason")
    os.write(master, b"w")
    wait_for_text(process, master, b"Why / reported history")


def seed_work(binary: str, config: Path, root: Path, environment: dict[str, str]) -> None:
    observations: list[dict[str, object]] = []

    def observe(identifier: str, scope: str, subject: str, second: int, body: dict[str, object]) -> None:
        observations.append({"id": identifier, "subject": {"scope": scope, "id": subject},
                             "occurred_at": f"2026-10-24T12:00:{second:02d}Z",
                             "producer": {"name": "terminal-fixture", "version": "1", "instance_id": "native"},
                             "method": "caller_reported", "body": body})

    def declare(identifier: str, name: str, kind: str, parent: str | None, second: int) -> None:
        observe(f"{identifier}-declared", "activity", identifier, second,
                {"event": "activity_declared", "name": name, "kind": kind,
                 "work_session_id": "native-work", "parent_activity_id": parent, "context": {}})
        observe(f"{identifier}-started", "activity", identifier, second, {"event": "started"})

    observe("native-work-opened", "session", "native-work", 0,
            {"event": "session_opened", "title": "Native work fixture", "context": {}})
    observe("native-request", "session", "native-work", 0, {"event": "milestone", "name": "request_received", "context": {}})
    declare("native-agent", "Implementation agent", "agent.task", None, 1)
    declare("native-reviewer", "Delegated reviewer", "review.qa", "native-agent", 2)
    declare("native-test-1", "Test attempt one", "test", "native-agent", 3)
    observe("native-test-1-finished", "activity", "native-test-1", 4,
            {"event": "finished", "outcome": "failed", "elapsed": None})
    declare("native-test-2", "Test attempt two", "test", "native-agent", 5)
    observe("native-test-2-finished", "activity", "native-test-2", 6,
            {"event": "finished", "outcome": "succeeded", "elapsed": None})
    declare("native-grandchild", "Grandchild test", "test", "native-reviewer", 7)
    observe("native-grandchild-finished", "activity", "native-grandchild", 8,
            {"event": "finished", "outcome": "succeeded", "elapsed": {
                "nanoseconds": "250000000", "method": "monotonic", "clock_id": "fixture-process"}})
    observe("native-production", "session", "native-work", 9,
            {"event": "milestone", "name": "production_verified", "context": {"environment": "production"}})
    observe("native-work-finished", "session", "native-work", 9,
            {"event": "finished", "outcome": "succeeded", "elapsed": None})
    fixture = root / "native work observations.json"
    fixture.write_text(json.dumps({"format_version": 1, "observations": observations}))
    recorded = subprocess.run([binary, "--config", str(config), "work", "record", str(fixture)],
                              cwd=root, env=environment, capture_output=True, check=True, timeout=5)
    assert json.loads(recorded.stdout)["ok"] is True


def exercise_work(binary: str, config: Path, root: Path, environment: dict[str, str], terminal: tuple[int, subprocess.Popen[bytes]]) -> None:
    master, process = terminal
    seed_work(binary, config, root, environment)
    os.write(master, b"\t")
    wait_for_text(process, master, b"Native work fixture")
    os.write(master, b"\r")
    wait_for_text(process, master, (b"Session duration breakdown", b"Grandchild test", "Time ↓".encode(), b"Selected: Native work fixture"))
    os.write(master, b"\x1b[C")
    wait_for_text(process, master, (b"Selected: Implementation agent", b"Kind: Subagent (agent.task)"))
    os.write(master, b"\x1b[C")
    wait_for_text(process, master, (b"Selected: Delegated reviewer", b"Kind: Review (review.qa)"))
    os.write(master, b"\x1b[D\x1b[D")
    wait_for_text(process, master, b"Selected: Native work fixture")
    os.write(master, b"+\x1b[B")
    wait_for_text(process, master, b"/row")
    os.write(master, b"d")
    wait_for_text(process, master, b"work sessions loaded")
    os.write(master, b"\r")
    wait_for_text(process, master, b"Implementation agent")
    os.write(master, b"\x1b[C")
    wait_for_text(process, master, (b"Delegated reviewer", b"Test attempt one", b"Test attempt two"))
    os.write(master, b"\x1b[B\x1b[C")
    wait_for_text(process, master, b"Grandchild test")
    os.write(master, b"t")
    wait_for_text(process, master, (b"Relative timeline", b"Time axis"))
    os.write(master, b"\x1b[B\r")
    wait_for_text(process, master, (b"activity:native-grandchild", b"0.250000000s (monotonic)"))
    os.write(master, b"\x1b")
    wait_for_text(process, master, b"Relative timeline")
    os.write(master, b"\x1b")
    wait_for_text(process, master, b"work sessions loaded")
    os.write(master, b"ktest\r")
    wait_for_text(process, master, (b"Activity search", b"Test attempt two"))
    os.write(master, b"\r")
    wait_for_text(process, master, b"w related session hierarchy")
    os.write(master, b"w")
    wait_for_text(process, master, b"Related work sessions")
    os.write(master, b"\r")
    wait_for_text(process, master, (b"Activity hierarchy", b"Implementation agent"))


def seed_concerns(binary: str, config: Path, root: Path, environment: dict[str, str]) -> int:
    location = {"repository": "native-repository", "object_format": "sha1", "commit": "a" * 40,
                "path": "src/native-router.rs", "language": "rust", "qualified_name": "router::finish",
                "declaration_kind": "function", "line_start": 10, "line_end": 20, "dirty": True, "verification": "supplied"}
    area = {"id": None, "repository": "native-repository", "kind": "subsystem", "key": "router", "display_name": "Native router subsystem", "discriminator": None}
    context = {"host_session_id": "native-host", "work_session_id": "native-work", "activity_id": "native-test-1", "repository": "native-repository"}
    observations = [
        {"id": "native-concern-report", "concern_id": "native-concern", "occurred_at": "2026-10-27T12:00:00Z", "observed_at": "2026-10-27T12:00:01Z", "context": context,
         "body": {"event": "reported", "title": "Native router race", "description": "Two callbacks finalize one dispatch", "hypothesis": None, "impact": None,
                  "encounter": {"symptom": "Duplicate finalization", "source": {"source": "native-tests", "repository": "native-repository", "execution_attempt": "native-test-1", "failure_id": "duplicate"},
                                "areas": [{"area": area, "relation": "affected", "location": location, "memberships": []}], "evidence": []}}},
        {"id": "native-concern-remedy", "concern_id": "native-concern", "occurred_at": "2026-10-27T12:01:00Z", "observed_at": "2026-10-27T12:01:01Z", "context": context,
         "body": {"event": "remedy_recorded", "encounter_ids": ["native-concern-report"], "what": "Serialize native completion", "why": "Duplicate native callbacks", "outcome": "uncertain", "evidence": [{"kind": "git", "location": location}]}},
    ]
    fixture = root / "native code concerns.json"
    fixture.write_text(json.dumps({"format_version": 1, "observations": observations}))
    recorded = subprocess.run([binary, "--config", str(config), "concerns", "record", str(fixture)],
                              cwd=root, env=environment, capture_output=True, check=True, timeout=5)
    assert json.loads(recorded.stdout)["ok"] is True
    detail = subprocess.run([binary, "--config", str(config), "concerns", "get", "native-concern"],
                            cwd=root, env=environment, capture_output=True, check=True, timeout=5)
    history = json.loads(detail.stdout)["concern"]["history"]
    return next(index for index, item in enumerate(history) if item["input"]["id"] == "native-concern-remedy")


def exercise_concerns(binary: str, config: Path, root: Path, environment: dict[str, str], terminal: tuple[int, subprocess.Popen[bytes]]) -> None:
    master, process = terminal
    remedy_index = seed_concerns(binary, config, root, environment)
    os.write(master, b"\t")
    wait_for_text(process, master, (b"Problem areas", b"Native router subsystem", b"known sources"))
    os.write(master, b"\r")
    wait_for_text(process, master, (b"Distinct concerns", b"Native router race"))
    os.write(master, b"\r")
    wait_for_text(process, master, (b"Timeline", b"remedy_recorded"))
    os.write(master, b"j" * remedy_index + b"r")
    wait_for_text(process, master, (b"Serialize native completion", b"Duplicate native callbacks", b"src/native-router.rs", b"dirty true"))


def exercise_terminal_dashboard(binary: str, runtime_source: str, exit_mode: str, remote_config: Path | None = None) -> None:
    with tempfile.TemporaryDirectory(prefix="nexus-tui-") as directory:
        root, config, environment = isolated_project(Path(directory))
        if remote_config is not None:
            config = remote_config
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
            os.write(master, b"\t")
            wait_for_text(process, master, b"No current SOS issues")
            exercise_analytics(process, master)
            os.write(master, b"\t\t")
            wait_for_text(process, master, b"terminal fixture memory")
            exercise_memory_actions(process, master)
            exercise_papercuts(binary, config, root, environment, (master, process))
            exercise_evals(binary, config, root, environment, (master, process))
            exercise_work(binary, config, root, environment, (master, process))
            exercise_concerns(binary, config, root, environment, (master, process))
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
    exercise_terminal_dashboard(sys.argv[1], sys.argv[2], sys.argv[3], Path(sys.argv[4]) if len(sys.argv) > 4 else None)
