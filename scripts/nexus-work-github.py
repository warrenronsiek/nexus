#!/usr/bin/env python3
# @feature work-history
# @spec docs/features/work-history.md
# @entrypoint caller-owned GitHub history import
# @boundary github-snapshot-json: decode caller-supplied provider metadata and emit the closed Nexus observation contract, without fetching or executing workloads.
"""Convert a local GitHub run/jobs/optional PR snapshot to Nexus recording JSON."""
import argparse
import datetime
import hashlib
import json
import pathlib
from typing import cast


def object_value(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError("expected provider object")
    return cast(dict[str, object], value)


def text(value: object) -> str:
    if not isinstance(value, (str, int)):
        raise ValueError("expected provider identity or text")
    return str(value)


def timestamp(value: object) -> str:
    result = text(value)
    parsed = datetime.datetime.fromisoformat(result.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("source timestamp requires timezone")
    return result


class GitHubCapture:
    def __init__(self, snapshot: dict[str, object], work_session_id: str) -> None:
        self.snapshot = snapshot
        self.work_session_id = work_session_id
        self.repository = text(snapshot["repository"])
        self.observed_at = timestamp(snapshot["observed_at"])
        self.run = object_value(snapshot["workflow_run"])
        attempt = text(self.run.get("run_attempt", 1))
        self.run_id = f"github:{self.repository}:run:{text(self.run['id'])}:attempt:{attempt}"
        self.revision = text(self.run["head_sha"])
        if len(self.revision) not in (40, 64) or any(ch not in "0123456789abcdefABCDEF" for ch in self.revision):
            raise ValueError("head_sha must be a full Git object ID")
        self.observations: list[dict[str, object]] = []

    def context(self, kind: str, identity: str, url: object = None) -> dict[str, object]:
        revision = {"repository": self.repository,
                    "object_format": "sha1" if len(self.revision) == 40 else "sha256", "commit": self.revision}
        external = {"provider": "github", "kind": kind, "id": identity,
                    "url": text(url) if url is not None else None}
        return {"repository": self.repository, "revision": revision, "external": [external],
                "labels": {"revision_basis": "workflow_run.head_sha"}}

    def emit(self, identity: str, occurrence: object, body: dict[str, object], method: str = "imported_provider_boundary") -> None:
        if not identity or len(identity.encode()) > 512:
            raise ValueError("provider activity identity exceeds Nexus's 512-byte bound")
        observation = {"subject": {"scope": "activity", "id": identity}, "occurred_at": timestamp(occurrence),
                       "producer": {"name": "github-snapshot", "version": "1", "instance_id": self.run_id},
                       "method": method, "body": body, "correction": None}
        encoded = json.dumps(observation, sort_keys=True, separators=(",", ":"))
        self.observations.append(dict(observation,id=hashlib.sha256(encoded.encode()).hexdigest()))

    def activity(self, source: dict[str, object], identity: str, parent: str | None, kind: str) -> None:
        boundary = source.get("created_at") or source.get("started_at")
        method = "imported_provider_boundary" if boundary else "imported_metadata_observation"
        self.emit(identity,boundary or self.observed_at,{"event":"activity_declared",
                  "name": f"GitHub {kind.split('.')[-1]} {identity.rsplit(':',1)[-1]}", "kind":kind,
                  "work_session_id":None,"parent_activity_id":parent,"initiator":None,
                  "executor":"github-actions","context":self.context(kind,identity)},method)
        self.emit(identity,self.observed_at,{"event":"linked","relation":"member_of",
                  "target":{"kind":"session","id":self.work_session_id}},"imported_membership_observation")
        self.metadata(source,identity,kind)
        for field, event in (("created_at", "queued"), ("started_at", "started")):
            if source.get(field) is not None:
                self.emit(identity, source[field], {"event": event})
        if source.get("completed_at") is not None:
            self.finish(source, identity)

    def metadata(self, source: dict[str, object], identity: str, kind: str) -> None:
        context = self.context(kind,identity,source.get("html_url"))
        labels = {field:text(source[field]) for field in ("name","status","conclusion") if source.get(field) is not None}
        if any(len(value.encode()) > 1024 for value in labels.values()):
            raise ValueError("provider metadata exceeds Nexus's 1024-byte label bound")
        labels["revision_basis"] = "workflow_run.head_sha"
        context["labels"] = labels
        self.emit(identity,self.observed_at,{"event":"milestone","name":"Provider metadata observed",
                  "context":context},"imported_metadata_observation")

    def finish(self, source: dict[str, object], identity: str) -> None:
        conclusions = {"success": "succeeded", "failure": "failed", "cancelled": "canceled",
                       "timed_out": "timed_out", "skipped": "skipped", "startup_failure": "infrastructure_error"}
        self.emit(identity, source["completed_at"], {"event": "finished",
                  "outcome": conclusions.get(text(source.get("conclusion") or "unknown"), "unknown"),
                  "elapsed": None, "exit_code": None, "signal": None, "assessment": None})

    def workflow(self) -> None:
        source = dict(self.run)
        source["started_at"] = self.run.get("run_started_at")
        self.activity(source, self.run_id, None, "ci.workflow")
        if self.run.get("conclusion") is not None:
            context = self.context("workflow_run", self.run_id, self.run.get("html_url"))
            context["labels"] = {"conclusion": text(self.run["conclusion"]), "timing": "completion boundary unavailable"}
            self.emit(self.run_id, self.observed_at, {"event": "milestone", "name": "Workflow conclusion observed",
                      "context": context}, "imported_status_observation")

    def jobs(self) -> None:
        for raw_job in source_array(self.snapshot, "jobs"):
            job = object_value(raw_job)
            job_id = f"{self.run_id}:job:{text(job['id'])}"
            self.activity(job, job_id, self.run_id, "ci.job")
            for raw_step in source_array(job, "steps"):
                step = object_value(raw_step)
                self.activity(step, f"{job_id}:step:{text(step['number'])}", job_id, "ci.step")

    def pull_request(self) -> None:
        if self.snapshot.get("pull_request") is None:
            return
        pr = object_value(self.snapshot["pull_request"])
        identity = f"github:{self.repository}:pr:{text(pr['number'])}"
        for field, name in (("created_at", "PR opened"), ("merged_at", "PR merged"), ("closed_at", "PR closed")):
            if pr.get(field) is not None:
                self.emit(self.run_id, pr[field], {"event": "milestone", "name": name,
                          "context": self.context("pull_request", identity, pr.get("html_url"))})


def source_array(source: dict[str, object], field: str) -> list[object]:
    value = source.get(field, [])
    if not isinstance(value, list):
        raise ValueError(f"{field} must be an array")
    return value


def convert(snapshot: dict[str, object], work_session_id: str) -> list[dict[str, object]]:
    capture = GitHubCapture(snapshot, work_session_id)
    capture.workflow()
    capture.jobs()
    capture.pull_request()
    return capture.observations


MAX_OBSERVATIONS = 64
MAX_PAGE_BYTES = 768 * 1024


def encode_page(observations: list[dict[str, object]]) -> str:
    return json.dumps({"format_version":1,"observations":observations},ensure_ascii=False,indent=2)+"\n"


def record_pages(observations: list[dict[str, object]]) -> list[str]:
    pages: list[str] = []
    page: list[dict[str, object]] = []
    for observation in observations:
        candidate = page + [observation]
        if len(candidate) > MAX_OBSERVATIONS or len(encode_page(candidate).encode()) > MAX_PAGE_BYTES:
            if not page:
                raise ValueError("one observation exceeds the record-file byte bound")
            pages.append(encode_page(page))
            candidate = [observation]
        if len(encode_page(candidate).encode()) > MAX_PAGE_BYTES:
            raise ValueError("one observation exceeds the record-file byte bound")
        page = candidate
    if page:
        pages.append(encode_page(page))
    return pages


def write_pages(directory: pathlib.Path, pages: list[str]) -> None:
    directory.mkdir(parents=True,exist_ok=True)
    if any(directory.iterdir()):
        raise ValueError("--output-directory must be empty; choose a directory for this snapshot")
    for index,page in enumerate(pages,1):
        (directory / f"observations-{index:06d}.json").write_text(page,encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=pathlib.Path)
    parser.add_argument("--work-session", required=True)
    parser.add_argument("--output-directory",type=pathlib.Path,help="Write bounded record files to an empty directory")
    arguments = parser.parse_args()
    try:
        snapshot = object_value(json.loads(arguments.snapshot.read_text(encoding="utf-8")))
        pages = record_pages(convert(snapshot,arguments.work_session))
        if arguments.output_directory is not None:
            write_pages(arguments.output_directory,pages)
        elif len(pages) > 1:
            raise ValueError("snapshot exceeds one record file; use --output-directory DIRECTORY, then record each JSON file")
        elif pages:
            print(pages[0],end="")
    except (ValueError,KeyError,OSError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
