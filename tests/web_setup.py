# @feature installation
# @feature observability-ui
# @spec docs/features/installation.md
# @spec docs/features/observability-ui.md
# @boundary isolated-credential-setup: exercise the packaged CLI in private temporary homes, without exposing passwords in process arguments or touching a live service.
"""Black-box setup checks, called by the Rust integration test."""
import json
import os
import signal
import socket
import time
import urllib.request
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest

BINARY = Path(sys.argv.pop(1)).resolve() if __name__ == "__main__" else Path("target/debug/nexus").resolve()


class WebSetupTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.credentials = self.root / "private" / "web-credentials.json"
        self.config = self.root / "config.toml"
        self.config.write_text('schema_version = 1\n[client]\ntransport = "remote"\nendpoint = "http://127.0.0.1:7337"\nproject_id = "fixture"\n')
        self.environment = {key: value for key, value in os.environ.items() if not key.startswith("NEXUS_")}
        self.environment.update({"HOME": str(self.root), "CODEX_HOME": str(self.root / "codex"), "CLAUDE_CONFIG_DIR": str(self.root / "claude"), "PATH": ""})

    def setup(self, arguments: list[str], password: str | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run([str(BINARY), "--config", str(self.config), "setup", "--refresh", *arguments], cwd=self.root, env=self.environment, input=password, text=True, capture_output=True, timeout=30)

    def provision(self) -> subprocess.CompletedProcess[str]:
        return self.setup(["--web-user", "viewer", "--web-password-file", "-", "--web-credentials-file", str(self.credentials)], "a-private-viewer-password\n")

    def test_setup_provisions_private_hashed_credentials_without_disclosing_password(self) -> None:
        result = self.provision()
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["web"]["username"], "viewer")
        self.assertEqual(report["web"]["credentials_file"], str(self.credentials))
        stored = self.credentials.read_text()
        self.assertIn("$argon2id$", stored)
        self.assertNotIn("a-private-viewer-password", stored + result.stdout + result.stderr)
        self.assertNotIn("$argon2id$", result.stdout + result.stderr)
        self.assertEqual(stat.S_IMODE(self.credentials.stat().st_mode), 0o600)

    def test_resource_refresh_preserves_existing_viewer_credentials(self) -> None:
        result = self.provision()
        self.assertEqual(result.returncode, 0, result.stderr)
        original = self.credentials.read_bytes()
        refreshed = self.setup([])
        self.assertEqual(refreshed.returncode, 0, refreshed.stderr)
        self.assertEqual(self.credentials.read_bytes(), original)

    def test_password_file_requires_private_permissions(self) -> None:
        password = self.root / "password.txt"
        password.write_text("a-private-viewer-password\n")
        password.chmod(0o644)
        result = self.setup(["--web-user", "viewer", "--web-password-file", str(password), "--web-credentials-file", str(self.credentials)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("private", result.stderr.lower())
        self.assertNotIn("a-private-viewer-password", result.stdout + result.stderr)
        self.assertFalse(self.credentials.exists())

    def test_web_starts_configured_local_daemon_without_an_agent(self) -> None:
        self.config.write_text(f'schema_version = 1\n[storage]\ndatabase_path = "{self.root}/nexus.db"\n[runtime]\nsocket_path = "{self.root}/nexus.sock"\nlock_path = "{self.root}/nexus.lock"\n[memory]\nenabled = false\n')
        result = self.provision()
        self.assertEqual(result.returncode, 0, result.stderr)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        with subprocess.Popen([str(BINARY), "--config", str(self.config), "web", "--credentials-file", str(self.credentials), "--listen", f"127.0.0.1:{port}"], cwd=self.root, env=self.environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True) as viewer:
            try:
                origin = f"http://127.0.0.1:{port}"
                self.wait_for_viewer(origin)
                body = json.dumps({"username":"viewer", "password":"a-private-viewer-password"}).encode()
                login = urllib.request.Request(origin + "/ui/api/login", data=body, headers={"Content-Type":"application/json", "Origin":origin})
                with urllib.request.urlopen(login, timeout=5) as response:
                    cookie = response.headers["Set-Cookie"].split(";", 1)[0]
                read = urllib.request.Request(origin + "/ui/api/read", data=b'{"project_id":null,"query":{"kind":"projects"}}', headers={"Content-Type":"application/json", "Origin":origin, "Cookie":cookie})
                with urllib.request.urlopen(read, timeout=5) as response:
                    self.assertTrue(json.load(response)["ok"])
            finally:
                os.killpg(viewer.pid, signal.SIGTERM)
                viewer.wait(timeout=10)

    def wait_for_viewer(self, origin: str) -> None:
        for _ in range(100):
            try:
                with urllib.request.urlopen(origin + "/ui/api/session", timeout=1):
                    return
            except OSError:
                time.sleep(0.05)
        self.fail("viewer did not become ready")


if __name__ == "__main__":
    unittest.main()
