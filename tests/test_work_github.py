# @feature work-history
# @spec docs/features/work-history.md
# @boundary synthetic-provider-snapshot-json
import importlib.util
import pathlib
import unittest
import subprocess
import tempfile
import os
import copy
import json
import sys

ADAPTER = pathlib.Path(__file__).resolve().parents[1] / "scripts/nexus-work-github.py"


class GitHubWorkCaptureTests(unittest.TestCase):
    def test_workflow_parent_attaches_the_same_execution_to_its_initiating_request(self) -> None:
        module = adapter()
        snapshot = fixture_snapshot()
        unlinked = module.convert(snapshot, "work")
        attached = module.convert(snapshot, "work", "request-17")
        declarations = lambda events: [item for item in events if item["body"]["event"] == "activity_declared"]
        self.assertEqual(declarations(unlinked), declarations(attached))
        parent = next(item for item in attached if item["body"]["event"] == "linked" and item["body"]["relation"] == "parent")
        self.assertEqual(parent["subject"]["id"], attached[0]["subject"]["id"])
        self.assertEqual(parent["body"]["target"], {"kind": "activity", "id": "request-17"})

    def test_named_shell_steps_preserve_the_child_result(self) -> None:
        helper = ADAPTER.parent / "work-capture.sh"
        with tempfile.TemporaryDirectory() as directory:
            binary = pathlib.Path(directory) / "nexus"
            marker = pathlib.Path(directory) / "used"
            binary.write_text('#!/bin/sh\nif [ "$1" = "config" ]; then exit 0; fi\nif [ "$2" = "--help" ]; then printf "%s" "--work-session"; exit 0; fi\nprintf used >"$FAKE_MARKER"\nshift\nwhile [ "$1" != "--" ]; do shift; done\nshift\nexec "$@"\n')
            binary.chmod(0o755)
            environment = dict(os.environ, NEXUS_WORK_BINARY=str(binary), FAKE_MARKER=str(marker))
            output = subprocess.run(["bash", "-c", 'source "$1"; nexus_work_exec test "Named test" sh -c \'printf child; exit 7\'', "test", str(helper)],
                                    env=environment, capture_output=True, check=False)
            self.assertEqual(output.returncode, 7, output.stderr)
            self.assertEqual(output.stdout, b"child")
            self.assertTrue(marker.exists())

    def test_one_execution_can_join_two_work_sessions_without_changing_its_declaration(self) -> None:
        module = adapter()
        snapshot = fixture_snapshot()
        first = module.convert(snapshot,"work-a")
        second = module.convert(snapshot,"work-b")
        declarations = lambda observations: [item["body"] for item in observations if item["body"]["event"] == "activity_declared"]
        self.assertEqual(declarations(first),declarations(second))
        memberships = lambda observations: {item["body"]["target"]["id"] for item in observations if item["body"]["event"] == "linked" and item["body"]["relation"] == "member_of"}
        self.assertEqual(memberships(first),{"work-a"})
        self.assertEqual(memberships(second),{"work-b"})

    def test_later_source_metadata_does_not_redeclare_or_invent_boundaries(self) -> None:
        module = adapter()
        snapshot = fixture_snapshot()
        job = snapshot["jobs"][0]
        job.pop("started_at")
        job.pop("completed_at")
        first = module.convert(snapshot,"work")
        snapshot["observed_at"] = "2026-10-24T12:20:00Z"
        job.update(started_at="2026-10-24T12:01:10Z",completed_at="2026-10-24T12:03:10Z",html_url="https://example.test/job/71")
        second = module.convert(snapshot,"work")
        declaration = lambda observations: [item["body"] for item in observations if item["body"]["event"] == "activity_declared"]
        self.assertEqual(declaration(first),declaration(second))
        observed = [item for item in first if item["body"]["event"] == "activity_declared" and item["subject"]["id"].endswith(":job:71")]
        self.assertEqual(observed[0]["occurred_at"],snapshot["observed_at"].replace("20:00","10:00"))
        self.assertEqual(observed[0]["method"],"imported_metadata_observation")

    def test_source_job_boundaries_are_exact_and_replays_are_stable(self) -> None:
        module = adapter()
        snapshot = fixture_snapshot()
        first = module.convert(snapshot, "work")
        self.assertEqual(first, module.convert(snapshot, "work"))
        finished = [event for event in first if event["body"]["event"] == "finished"]
        self.assertEqual(len(finished), 2)
        self.assertIsNone(finished[0]["body"]["elapsed"])
        self.assertEqual(finished[0]["occurred_at"],"2026-10-24T12:03:10Z")
        job_start = next(item for item in first if item["subject"]["id"].endswith(":job:71") and item["body"]["event"] == "started")
        self.assertEqual(job_start["occurred_at"],"2026-10-24T12:01:10Z")
        declarations = [event["body"] for event in first if event["body"]["event"] == "activity_declared"]
        self.assertEqual(declarations[1]["parent_activity_id"], first[0]["subject"]["id"])
        self.assertEqual(declarations[2]["parent_activity_id"], next(event["subject"]["id"] for event in first if event["body"] == declarations[1]))
        self.assertTrue(any(event["body"].get("name") == "Workflow conclusion observed" for event in first))
        self.assertNotIn("2026-10-24T12:09:00Z", str(finished))

    def test_completion_report_is_stable_when_source_start_arrives_later(self) -> None:
        module = adapter()
        snapshot = fixture_snapshot()
        job = snapshot["jobs"][0]
        job.pop("started_at")
        first = module.convert(snapshot,"work")
        job["started_at"] = "2026-10-24T12:01:10Z"
        second = module.convert(snapshot,"work")
        finished = lambda observations: next(item for item in observations if item["subject"]["id"].endswith(":job:71") and item["body"]["event"] == "finished")
        self.assertEqual(finished(first),finished(second))

    def test_accounting_metadata_qualifies_runner_identity_without_changing_legacy_declarations(self) -> None:
        module = adapter()
        snapshot = fixture_snapshot()
        first = module.convert(snapshot, "work")
        snapshot["jobs"][0]["runner_id"] = 8123
        snapshot["observed_at"] = "2026-10-24T12:21:00Z"
        second = module.convert(snapshot, "work")
        declarations = lambda values: [value["body"] for value in values if value["body"]["event"] == "activity_declared"]
        self.assertEqual(declarations(first), declarations(second))
        self.assertTrue(all(value["executor"] == "github-actions" for value in declarations(second)))
        self.assertTrue(all(value["context"]["labels"] == {"revision_basis": "workflow_run.head_sha"} for value in declarations(second)))
        roles = [value["body"]["context"]["labels"]["nexus.accounting.role"] for value in second if value["body"].get("name") == "Accounting role observed"]
        self.assertEqual(roles, ["container", "container", "execution"])
        executors = [value["body"]["context"]["labels"] for value in second if value["body"].get("name") == "Executor identity observed"]
        self.assertEqual([value.get("nexus.executor.id") for value in executors], [None, "github:example/repository:runner:8123", "github:example/repository:runner:8123"])
        self.assertTrue(all(value["nexus.executor.declaration"] == "unqualified" for value in executors))

    def test_runner_identity_remains_unknown_when_provider_has_not_assigned_a_positive_integer(self) -> None:
        module = adapter()
        for value in [None, 0, -1, True, "8123"]:
            with self.subTest(runner_id=value):
                snapshot = fixture_snapshot()
                snapshot["jobs"][0]["runner_id"] = value
                metadata = [item for item in module.convert(snapshot,"work") if item["body"].get("name") == "Executor identity observed"]
                self.assertEqual(len(metadata),3)
                self.assertTrue(all("nexus.executor.id" not in item["body"]["context"]["labels"] for item in metadata))

    def test_large_snapshots_write_complete_bounded_record_pages(self) -> None:
        snapshot = large_snapshot()
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "snapshot.json"
            source.write_text(json.dumps(snapshot))
            output = subprocess.run([sys.executable,str(ADAPTER),str(source),"--work-session","work","--output-directory",str(root / "pages")],capture_output=True,check=False)
            self.assertEqual(output.returncode,0,output.stderr)
            pages = sorted((root / "pages").glob("*.json"))
            self.assertGreater(len(pages),1)
            observations = []
            for path in pages:
                page = json.loads(path.read_text())
                self.assertLessEqual(len(page["observations"]),64)
                self.assertLessEqual(path.stat().st_size,768*1024)
                observations.extend(page["observations"])
            self.assertEqual(observations,adapter().convert(snapshot,"work"))

    def test_large_default_output_has_a_clear_bound_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "snapshot.json"
            source.write_text(json.dumps(large_snapshot()))
            output = subprocess.run([sys.executable,str(ADAPTER),str(source),"--work-session","work"],capture_output=True,check=False)
            self.assertNotEqual(output.returncode,0)
            self.assertEqual(output.stdout,b"")
            self.assertIn(b"--output-directory",output.stderr)


def adapter() -> object:
    spec = importlib.util.spec_from_file_location("nexus_work_github",ADAPTER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fixture_snapshot() -> dict[str, object]:
    return {
        "observed_at": "2026-10-24T12:10:00Z",
        "repository": "example/repository",
        "workflow_run": {"id": 41, "run_attempt": 2, "name": "CI", "head_sha": "a" * 40,
                         "created_at": "2026-10-24T12:00:00Z", "run_started_at": "2026-10-24T12:01:00Z",
                         "status": "completed", "conclusion": "failure", "updated_at": "2026-10-24T12:09:00Z"},
        "jobs": [{"id": 71, "name": "Linux", "started_at": "2026-10-24T12:01:10Z",
                  "completed_at": "2026-10-24T12:03:10Z", "conclusion": "failure",
                  "steps": [{"number": 1, "name": "Tests", "started_at": "2026-10-24T12:01:20Z",
                             "completed_at": "2026-10-24T12:03:00Z", "conclusion": "failure"}]}],
    }


def large_snapshot() -> dict[str, object]:
    snapshot = fixture_snapshot()
    template = snapshot["jobs"][0]
    snapshot["jobs"] = [dict(copy.deepcopy(template),id=1000+index) for index in range(30)]
    return snapshot


if __name__ == "__main__":
    unittest.main()
