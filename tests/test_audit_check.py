# @feature audit-registry
# @spec docs/features/audit-registry.md
# @boundary synthetic-audit-json: exercise the CLI with public registry response fixtures.
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECK = Path(__file__).resolve().parents[1] / 'scripts/nexus-audit-check.py'
SUBJECT = {'kind': 'git', 'revision': {'repository': 'acme/service', 'object_format': 'sha1', 'commit': 'a' * 40}}
CONTROL = {'key': 'review', 'version': 1}


def plan() -> dict:
    return {'project_id': 'project', 'expectations': [
        {'name': 'Review', 'control': copy.deepcopy(CONTROL), 'subject': copy.deepcopy(SUBJECT), 'run_id': 'run-1', 'accepted_codes': ['pass']}]}


def detail() -> dict:
    # RunSummary is the server's correction-aware projection, not a local fold.
    return {'ok': True, 'response': 'page', 'project_id': 'project', 'cutoff': 2, 'next_cursor': None,
            'items': [{'id': 'run-1', 'ordinal': 2, 'control': copy.deepcopy(CONTROL),
                       'subject': copy.deepcopy(SUBJECT), 'status': 'completed', 'result_code': 'pass', 'findings': 0}]}


class AuditCompletionTests(unittest.TestCase):
    def invoke(self, expected: dict, responses: dict) -> tuple[subprocess.CompletedProcess, list[list[str]]]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'plan.json').write_text(json.dumps(expected))
            (root / 'responses.json').write_text(json.dumps(responses))
            binary = root / 'nexus'
            binary.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE'])
with (root / 'calls').open('a') as output:
    output.write(json.dumps(sys.argv[1:]) + '\\n')
responses = json.loads((root / 'responses.json').read_text())
request = json.loads(sys.stdin.read())
print(json.dumps(responses[str(request.get('filter', {}).get('after') or 0)]))
''')
            binary.chmod(0o755)
            result = subprocess.run([sys.executable, str(CHECK), str(root / 'plan.json'), '--nexus', str(binary)],
                                    env=dict(os.environ, FIXTURE=tmp), capture_output=True, text=True)
            calls = [json.loads(line) for line in (root / 'calls').read_text().splitlines()] if (root / 'calls').exists() else []
            return result, calls

    def test_missing_receipt_is_reported_without_claiming_completion(self) -> None:
        expected = plan()
        expected['expectations'][0].pop('run_id')
        result, calls = self.invoke(expected, {})
        self.assertEqual(result.returncode, 1, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['items'][0]['state'], 'missing')
        self.assertFalse(report['complete'])
        self.assertEqual(calls, [])

    def test_verified_exact_run_is_read_only_and_retains_receipt_identity(self) -> None:
        result, calls = self.invoke(plan(), {'0': detail()})
        self.assertEqual(result.returncode, 0, result.stderr)
        item = json.loads(result.stdout)['items'][0]
        self.assertEqual(item['state'], 'recorded')
        self.assertEqual(item['ordinal'], 2)
        self.assertEqual(calls, [['audit', 'read', '-']])

    def test_wrong_project_control_subject_and_unacknowledged_run_are_rejected(self) -> None:
        for change in ['project', 'control', 'subject', 'id']:
            response = detail()
            if change == 'project': response['project_id'] = 'other'
            elif change == 'control': response['items'][0]['control']['version'] = 2
            elif change == 'subject': response['items'][0]['subject']['revision']['commit'] = 'b' * 40
            elif change == 'id': response['items'][0]['id'] = 'other'
            with self.subTest(change=change):
                result, _ = self.invoke(plan(), {'0': response})
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertFalse(json.loads(result.stdout)['complete'])
                if change in ('project', 'id'):
                    expected_state = 'unconfirmed' if change == 'project' else 'unacknowledged'
                    self.assertEqual(json.loads(result.stdout)['items'][0]['state'], expected_state)

    def test_server_corrected_outcome_and_findings_prevent_false_success(self) -> None:
        for change in [{'status': 'failed'}, {'result_code': 'inconclusive'}, {'findings': 1}]:
            response = detail()
            response['items'][0].update(change)
            result, _ = self.invoke(plan(), {'0': response})
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertEqual(json.loads(result.stdout)['items'][0]['state'], 'attention')

    def test_bulk_selection_follows_pages_without_rereading_each_run(self) -> None:
        expected = plan()
        second = copy.deepcopy(expected['expectations'][0])
        second.update(name='Second review', run_id='run-2')
        expected['expectations'].append(second)
        first_page = detail()
        first_page.update(cutoff=3, next_cursor=2)
        second_page = copy.deepcopy(first_page)
        second_page['next_cursor'] = None
        second_page['items'][0].update(id='run-2', ordinal=3)
        result, calls = self.invoke(expected, {'0': first_page, '2': second_page})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(calls), 2)

    def test_unavailable_service_and_pagination_gaps_remain_visible(self) -> None:
        for response in [{'ok': False, 'error': 'offline'}, dict(detail(), next_cursor=0)]:
            result, _ = self.invoke(plan(), {'0': response})
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertEqual(json.loads(result.stdout)['items'][0]['state'], 'unconfirmed')

    def test_failed_attempt_with_unavailable_patch_digest_is_attention_not_invalid_plan(self) -> None:
        expected = plan()
        unavailable = {'kind': 'staged_diff', 'repository': 'acme/service', 'base': None, 'sha256': None}
        expected['expectations'][0]['subject'] = unavailable
        response = detail()
        run = response['items'][0]
        run['subject'] = unavailable
        run['status'] = 'failed'
        run['result_code'] = 'inconclusive'
        result, _ = self.invoke(expected, {'0': response})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)['items'][0]['state'], 'attention')

    def test_not_applicable_requires_a_reason_and_empty_plan_cannot_claim_coverage(self) -> None:
        expected = plan()
        expected['expectations'] = [{'name': 'Browser QA', 'not_applicable': 'Documentation-only change'}]
        result, calls = self.invoke(expected, {})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)['items'][0]['state'], 'not_applicable')
        self.assertEqual(calls, [])
        for entries in [[], [{'name': 'QA', 'not_applicable': ''}]]:
            expected['expectations'] = entries
            result, _ = self.invoke(expected, {})
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
