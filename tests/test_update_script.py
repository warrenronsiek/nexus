# @feature installation
# @spec docs/features/installation.md
# @boundary update-wrapper-fixtures: fake Nexus commands preserve argv and status without updating the machine.
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class UpdateScriptTests(unittest.TestCase):
    def test_update_from_another_directory_preserves_arguments_and_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            command = directory / "nexus executable"
            log = directory / "arguments.json"
            command.write_text("#!/usr/bin/env python3\nimport json,sys\nfrom pathlib import Path\n"
                               f"Path({str(log)!r}).write_text(json.dumps(sys.argv[1:]))\nsys.exit(7)\n")
            command.chmod(0o755)
            arguments = ["--config", str(directory / "custom config.toml"), "--service-updater", str(directory / "service updater")]
            result = subprocess.run(["bash", str(ROOT / "scripts/update.sh"), *arguments], cwd=directory,
                                    env=dict(os.environ, NEXUS_BIN=str(command)), capture_output=True, text=True)
            self.assertEqual(result.returncode, 7, result.stderr)
            self.assertEqual(json.loads(log.read_text()), ["update", *arguments])
