# @feature installation
# @spec docs/features/installation.md
# @boundary isolated-compose-bootstrap: fake executables record startup and management handoff without Docker or user configuration.
import os
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class ComposeManagementTests(unittest.TestCase):
    def test_start_records_exact_compose_project_for_checkout_free_updates(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            tools = directory / "bin"
            tools.mkdir()
            log = directory / "commands"
            for name in ["docker", "nexus"]:
                tool = tools / name
                tool.write_text(f'#!/bin/sh\nprintf "%s\\n" "{name}" "$@" >> "{log}"\n')
                tool.chmod(0o755)
            environment = dict(os.environ, HOME=str(directory), PATH=f"{tools}:/usr/bin:/bin", NEXUS_PORT="8123")
            result = subprocess.run(["bash", str(ROOT / "scripts/start-docker-service.sh"), "--no-import"], env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            commands = log.read_text().splitlines()
            self.assertIn("nexus", commands)
            self.assertEqual(commands[-6:], ["nexus", "update", "--record-compose", str(ROOT / "docker-compose.yml"), "--service-endpoint", "http://127.0.0.1:8123"])

    def test_start_installs_private_browser_credentials_before_boot_without_logging_them(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            tools = directory / "bin"
            tools.mkdir()
            log = directory / "commands.jsonl"
            received = directory / "received.json"
            credential = directory / "viewer credentials.json"
            fixture = ROOT / "tests/fixtures/web-credentials.fixture.json"
            credential.write_bytes(fixture.read_bytes())
            credential.chmod(0o600)
            docker = tools / "docker"
            docker.write_text(
                "#!/usr/bin/env python3\nimport json,sys\nfrom pathlib import Path\n"
                f"with Path({str(log)!r}).open('a') as stream: stream.write(json.dumps(sys.argv[1:])+'\\n')\n"
                f"if 'run' in sys.argv: Path({str(received)!r}).write_bytes(sys.stdin.buffer.read())\n"
            )
            docker.chmod(0o755)
            environment = dict(os.environ, HOME=str(directory), PATH=f"{tools}:/usr/bin:/bin")
            for key in ["NEXUS_WEB_ORIGIN", "NEXUS_WEB_CREDENTIALS_FILE"]:
                environment.pop(key, None)
            result = subprocess.run(
                ["bash", str(ROOT / "scripts/start-docker-service.sh"), "--no-import", "--web-credentials", str(credential)],
                env=environment, capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertEqual(calls[0][-2:], ["build", "nexus"])
            self.assertIn("run", calls[1])
            self.assertIn("--no-deps", calls[1])
            self.assertIn("-T", calls[1])
            self.assertEqual(calls[1][calls[1].index("--entrypoint") + 1], "sh")
            self.assertEqual(calls[2][-4:], ["up", "--detach", "--force-recreate", "nexus"])
            self.assertEqual(received.read_bytes(), credential.read_bytes())
            self.assertNotIn("password_hash", result.stdout + result.stderr)
            self.assertNotIn(json.loads(credential.read_text())["password_hash"], result.stdout + result.stderr)

    def test_browser_credential_provisioning_rejects_unsafe_source_before_docker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            tools = directory / "bin"
            tools.mkdir()
            log = directory / "docker-called"
            docker = tools / "docker"
            docker.write_text(f'#!/bin/sh\nprintf called > "{log}"\n')
            docker.chmod(0o755)
            credential = directory / "viewer.json"
            credential.write_bytes((ROOT / "tests/fixtures/web-credentials.fixture.json").read_bytes())
            credential.chmod(0o644)
            environment = dict(os.environ, HOME=str(directory), PATH=f"{tools}:/usr/bin:/bin")
            result = subprocess.run(
                ["bash", str(ROOT / "scripts/start-docker-service.sh"), "--no-import", "--web-credentials", str(credential)],
                env=environment, capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("private", result.stderr)
            self.assertFalse(log.exists())

if __name__ == "__main__":
    unittest.main()
