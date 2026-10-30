# @feature observability-ui
# @feature runtime
# @spec docs/features/observability-ui.md
# @spec docs/features/runtime.md
# @boundary isolated-http-proxy: delay reads to a disposable real Nexus service.
"""Observe startup and recovery in a real terminal across the MCP deadline."""
from __future__ import annotations

import http.server
import os
import select
import time
from pathlib import Path
import sys
import tempfile
import threading
import urllib.request
from tui_terminal import isolated_project, wait_for_text, close_dashboard, terminal_dashboard, terminal_text, _screens


class DelayedReads(http.server.BaseHTTPRequestHandler):
    ready = threading.Event()
    endpoint: str

    def log_message(self, *_arguments: object) -> None:
        pass

    def do_POST(self) -> None:
        body = self.rfile.read(int(self.headers["Content-Length"]))
        if not self.ready.wait(15):
            self.send_error(503)
            return
        request = urllib.request.Request(self.endpoint + "/api/v1/request", data=body, headers={
            "Authorization": self.headers["Authorization"], "Content-Type": "application/json",
        })
        with urllib.request.urlopen(request, timeout=5) as response:
            wire = response.read()
        self.send_response(200)
        self.send_header("Content-Length", str(len(wire)))
        self.end_headers()
        try:
            self.wfile.write(wire)
        except BrokenPipeError:
            pass


def exercise_startup(binary: str, config: str, endpoint: str) -> None:
    DelayedReads.endpoint = endpoint
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), DelayedReads)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="nexus-tui-startup-") as directory:
            root, _config, environment = isolated_project(Path(directory))
            environment["NEXUS_ENDPOINT"] = f"http://127.0.0.1:{server.server_port}"
            with terminal_dashboard(binary, Path(config), root, environment) as (process, master):
                wait_for_text(process, master, b"Connecting to Nexus", 3)
                assert "recent sessions" not in terminal_text(process)
                deadline = time.monotonic() + 6
                while time.monotonic() < deadline:
                    readable, _, _ = select.select([master], [], [], 0.1)
                    if readable:
                        _screens[process].feed(os.read(master, 65536))
                assert "Connecting to Nexus" in terminal_text(process)
                assert "Unavailable" not in terminal_text(process)
                assert process.poll() is None, "A queued read must not close the terminal"
                DelayedReads.ready.set()
                wait_for_text(process, master, b"Coordination", 8)
                close_dashboard(process, master, "quit")
    finally:
        DelayedReads.ready.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    exercise_startup(sys.argv[1], sys.argv[2], sys.argv[3])
