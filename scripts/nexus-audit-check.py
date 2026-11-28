#!/usr/bin/env python3
# @feature audit-registry
# @spec docs/features/audit-registry.md
# @entrypoint reconcile a caller-owned task population against acknowledged Audit runs
# @boundary audit-check-json: decode caller plans and the public typed Nexus audit response only.
"""Read-only reconciliation. Exit 0 = matched evidence, 1 = gaps, 2 = invalid plan.

This checks a caller-declared population; it cannot discover omitted obligations,
replace an approval, or determine the meaning of a narrative review result.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import re
import subprocess
import sys


@dataclass(frozen=True)
class Expectation:
    name: str
    control: dict | None
    subject: dict | None
    run_id: str | None
    accepted_codes: tuple[str, ...]
    not_applicable: str | None


@dataclass(frozen=True)
class Task:
    project_id: str
    repository: str
    expectations: tuple[Expectation, ...]


def text(value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError('expected a nonempty string')
    return value


def subject(value: dict, repository: str) -> dict:
    kind = value['kind']
    if kind == 'git':
        revision = value['revision']
        size = {'sha1': 40, 'sha256': 64}[revision['object_format']]
        if revision['repository'] != repository or not re.fullmatch('[a-f0-9]{%d}' % size, revision['commit']):
            raise ValueError('Git subject must pin this repository and a full object ID')
    elif kind == 'staged_diff':
        if value['repository'] != repository or not re.fullmatch('[a-f0-9]{64}', value['sha256']):
            raise ValueError('staged subject must pin this repository and exact patch digest')
        if value.get('base'):
            subject({'kind': 'git', 'revision': value['base']}, repository)
    elif kind == 'external':
        text(value['source'])
        text(value['id'])
    else:
        raise ValueError('unsupported subject kind')
    return value


def expectation(entry: dict, repository: str) -> Expectation:
    name = text(entry['name'])
    if 'not_applicable' in entry:
        if set(entry) != {'name', 'not_applicable'}:
            raise ValueError('an exclusion cannot also assert a run')
        return Expectation(name, None, None, None, (), text(entry['not_applicable']))
    control = entry['control']
    text(control['key'])
    if type(control['version']) is not int or control['version'] < 1:
        raise ValueError('control version must be a positive integer')
    codes = tuple(text(code) for code in entry['accepted_codes'])
    if not codes:
        raise ValueError('declare acceptable categorical outcomes; reconcile narrative controls manually')
    run_id = text(entry['run_id']) if entry.get('run_id') is not None else None
    return Expectation(name, control, subject(entry['subject'], repository), run_id, codes, None)


def load_task(path: str) -> Task:
    raw = json.loads(Path(path).read_text())
    project, repository = text(raw['project_id']), text(raw['repository'])
    entries = tuple(expectation(entry, repository) for entry in raw['expectations'])
    if not entries or len({entry.name for entry in entries}) != len(entries):
        raise ValueError('declare a nonempty population with distinct expectation names')
    return Task(project, repository, entries)


def read_detail(binary: str, project: str, run_id: str) -> tuple[dict, list[dict]]:
    request = {'action': 'get', 'project_id': project, 'run_id': run_id}
    records, cutoff, definition = [], None, None
    # The registry pages bounded history; pin the first cutoff for every continuation.
    while True:
        completed = subprocess.run([binary, 'audit', 'read', '-'], input=json.dumps(request),
                                   text=True, capture_output=True, timeout=30, check=True)
        page = json.loads(completed.stdout)
        if page.get('ok') is not True or page.get('response') != 'detail' or page.get('project_id') != project:
            raise ValueError('project-bound detail not acknowledged')
        if cutoff is None:
            cutoff, definition = page['cutoff'], page['definition']
        if page['cutoff'] != cutoff or page['definition'] != definition:
            raise ValueError('history changed during pinned read')
        records.extend(page['records'])
        cursor = page.get('next_cursor')
        if cursor is None:
            return definition, records
        if type(cursor) is not int or cursor <= request.get('after', 0) or cursor > cutoff:
            raise ValueError('history cursor did not advance within cutoff')
        request.update(after=cursor, cutoff=cutoff)


def verified_result(entry: Expectation, definition: dict, records: list[dict]) -> tuple[int, dict]:
    original = next(record for record in records if record['input']['id'] == entry.run_id
                    and record['input']['event']['kind'] == 'run')
    event = original['input']['event']
    if (definition['input']['event']['control'] != entry.control or event['control'] != entry.control
            or event['run']['subject'] != entry.subject):
        raise ValueError('Control version or exact source differs from task population')
    if (type(original['ordinal']) is not int or original['ordinal'] < 1
            or not original['actor'].get('agent') or not original['actor'].get('session_id')):
        raise ValueError('receipt identity is incomplete')
    result = event['run']['result']
    for record in sorted(records, key=lambda value: value['ordinal']):
        correction = record['input']['event']
        if correction['kind'] == 'correct_run' and correction['run_id'] == entry.run_id:
            result = correction['result']
    return original['ordinal'], result


def reconcile(entry: Expectation, task: Task, binary: str) -> dict:
    item = {'name': entry.name}
    if entry.not_applicable:
        return dict(item, state='not_applicable', reason=entry.not_applicable)
    item.update(control=entry.control, subject=entry.subject, run_id=entry.run_id)
    if not entry.run_id:
        return dict(item, state='missing', reason='No run ID; queued input and Work history are not Audit receipts')
    try:
        definition, records = read_detail(binary, task.project_id, entry.run_id)
        ordinal, result = verified_result(entry, definition, records)
        output = result.get('output') or {}
        acceptable = (result['status'] == 'completed' and output.get('kind') == 'code'
                      and output.get('value') in entry.accepted_codes and not result.get('findings'))
        return dict(item, state='recorded' if acceptable else 'attention', ordinal=ordinal,
                    status=result['status'], output=output, findings=len(result.get('findings', [])),
                    reason='Matched receipt' if acceptable else 'Outcome or findings require explicit follow-up')
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError, StopIteration):
        # Do not print CLI stderr: a misconfigured service can include private data.
        return dict(item, state='unconfirmed', reason='Unable to verify complete matching project-bound run history')


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('plan', help='caller-owned task population JSON (store outside tracked files)')
    parser.add_argument('--nexus', default='nexus', help='Nexus CLI executable')
    args = parser.parse_args()
    try:
        task = load_task(args.plan)
    except (OSError, ValueError, KeyError, TypeError):
        print('Invalid task plan: pin project, repository, control versions, subjects and accepted codes.', file=sys.stderr)
        return 2
    items = [reconcile(entry, task, args.nexus) for entry in task.expectations]
    complete = all(item['state'] in ('recorded', 'not_applicable') for item in items)
    print(json.dumps({'project_id': task.project_id, 'repository': task.repository,
                      'complete': complete, 'items': items}, indent=2))
    return 0 if complete else 1


if __name__ == '__main__':
    sys.exit(main())
