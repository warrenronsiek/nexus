#!/usr/bin/env python3
# @feature observability-ui
# @feature runtime
# @feature architecture-tooling
# @spec docs/features/observability-ui.md
# @spec docs/features/runtime.md
# @spec docs/features/architecture-tooling.md
# @boundary isolated-browser-smoke: inspects an explicitly supplied test origin using the public test-only viewer fixture and an ephemeral agent credential.
"""Exercise browser assets, login, closed reads, and authentication separation."""
from __future__ import annotations

from html.parser import HTMLParser
from http.client import HTTPMessage
import json
import os
import sys
from urllib.error import HTTPError
from urllib.parse import urljoin, urlsplit
from urllib.request import Request, urlopen

# Public test-only fixture; no production credential is configured by this script.
VIEWER_PASSWORD = "nexus-browser-test-only-password"


class Assets(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.paths: list[str] = []

    def handle_starttag(self, tag: str, attributes: list[tuple[str, str | None]]) -> None:
        values = dict(attributes)
        if tag == "script" and values.get("src"):
            self.paths.append(str(values["src"]))
        if tag == "link" and values.get("rel") == "stylesheet" and values.get("href"):
            self.paths.append(str(values["href"]))


class BrowserProbe:
    def __init__(self, origin: str, agent_secret: str) -> None:
        self.origin = origin
        self.agent_secret = agent_secret
        self.cookie: str | None = None

    def request(self, path: str, payload: object | None = None,
                headers: dict[str, str] | None = None) -> tuple[int, bytes, HTTPMessage]:
        headers = dict(headers or {})
        body = None
        if payload is not None:
            body = json.dumps(payload).encode()
            headers["Content-Type"] = "application/json"
            headers.setdefault("Origin", self.origin)
        try:
            response = urlopen(Request(urljoin(self.origin + "/", path), data=body, headers=headers), timeout=20)
        except HTTPError as error:
            response = error
        with response:
            return response.status, response.read(), response.headers

    def read(self, kind: str) -> tuple[int, bytes, HTTPMessage]:
        return self.request("/ui/api/read", {"project_id": "docker-smoke", "query": {"kind": kind}}, {"Cookie": self.cookie or ""})


def inspect_assets(probe: BrowserProbe) -> None:
    status, html, _ = probe.request("/")
    assert status == 200 and b"Nexus" in html, "packaged browser HTML is unavailable"
    assets = Assets()
    assets.feed(html.decode())
    assert len(assets.paths) >= 2, "browser HTML omits its script or stylesheet"
    for path in assets.paths:
        assert urlsplit(urljoin(probe.origin, path)).netloc == urlsplit(probe.origin).netloc, "asset escaped the service origin"
        status, content, _ = probe.request(path)
        assert status == 200 and content, f"packaged browser asset is unavailable: {path}"
        assert probe.agent_secret.encode() not in content, "browser asset contains the agent credential"


def inspect_login(probe: BrowserProbe) -> None:
    read = {"project_id": "docker-smoke", "query": {"kind": "coordination"}}
    assert probe.request("/ui/api/read", read)[0] == 401, "unauthenticated browser read was admitted"
    assert probe.request("/ui/api/read", read, headers={"Authorization": f"Bearer {probe.agent_secret}"})[0] == 401, "agent bearer authorized a browser read"
    assert probe.request("/ui/api/login", {"username": "viewer", "password": "incorrect test-only password"})[0] == 401, "incorrect viewer password was accepted"
    status, body, headers = probe.request("/ui/api/login", {"username": "viewer", "password": VIEWER_PASSWORD})
    assert status == 200 and json.loads(body).get("authenticated") is True, "viewer login failed"
    set_cookie = headers.get("Set-Cookie", "")
    assert all(flag in set_cookie for flag in ["HttpOnly", "SameSite=Strict", "Path=/ui/api", "Max-Age="]), "browser cookie omitted its protection or expiry"
    probe.cookie = set_cookie.split(";", 1)[0]
    assert probe.agent_secret not in probe.cookie, "browser session reused the agent bearer"


def inspect_reads(probe: BrowserProbe) -> None:
    for kind in ["coordination", "usage", "analytics_summary", "analytics_catalog", "analytics_opportunities",
                 "memory_status", "memory_context", "papercuts", "sos_list", "eval_dashboard", "work_dashboard", "concern_dashboard"]:
        status, body, _ = probe.read(kind)
        assert status == 200 and json.loads(body).get("ok") is True, f"scoped browser read failed: {kind}"


def inspect_read_only(probe: BrowserProbe) -> None:
    for kind in ["memory_add", "memory_invalidate", "memory_consolidate", "papercut_report", "sos_report",
                 "eval_record", "work_record", "concern_report", "analytics_rename", "coordination_resolve"]:
        assert probe.read(kind)[0] == 400, f"browser read API admitted a mutation: {kind}"
    query = {"project_id": "docker-smoke", "query": {"kind": "coordination"}, "method": "memory.add"}
    assert probe.request("/ui/api/read", query, {"Cookie": probe.cookie or ""})[0] == 400, "browser accepted an arbitrary service method"


def inspect_separate_auth(probe: BrowserProbe) -> None:
    query = {"project_id": "docker-smoke", "query": {"kind": "coordination"}}
    assert probe.request("/ui/api/read", query, {"Cookie": probe.cookie or "", "Origin": "https://other.example"})[0] == 403, "cross-origin browser read was admitted"
    # Supply the cookie explicitly, beyond its normal /ui/api Path, to prove agent admission is independent.
    for path, payload in [("/api/v1/request", {"method": "status", "params": {}, "project_id": "docker-smoke"}),
                          ("/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/list"})]:
        assert probe.request(path, payload, {"Cookie": probe.cookie or ""})[0] == 401, "browser cookie authorized an agent API request"


def inspect_logout(probe: BrowserProbe) -> None:
    assert probe.request("/ui/api/logout", {}, {"Cookie": probe.cookie or ""})[0] == 200, "browser logout failed"
    assert probe.read("coordination")[0] == 401, "logged-out browser session remained authorized"
    assert json.loads(probe.request("/ui/api/session", headers={"Cookie": probe.cookie or ""})[1]).get("authenticated") is False, "logged-out session remained active"


if __name__ == "__main__":
    probe = BrowserProbe(sys.argv[1].rstrip("/"), os.environ["NEXUS_TEST_SERVICE_SECRET"])
    inspect_assets(probe)
    inspect_login(probe)
    inspect_reads(probe)
    inspect_read_only(probe)
    inspect_separate_auth(probe)
    inspect_logout(probe)
    print("Browser smoke passed: packaged assets, login, scoped reads, write rejection, and credential separation")
