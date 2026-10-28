# @feature installation
# @spec docs/features/installation.md
# @boundary isolated-compose-bootstrap: fake executables record startup and management handoff without Docker or user configuration.
import os
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

if __name__ == "__main__":
    unittest.main()
