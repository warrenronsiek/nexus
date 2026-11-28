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
    expectations: tuple[Expectation, ...]


def text(value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError('expected a nonempty string')
    return value


def expectation(entry: dict) -> Expectation:
    name = text(entry['name'])
    if 'not_applicable' in entry:
        if set(entry) != {'name', 'not_applicable'}:
            raise ValueError('an exclusion cannot also assert a run')
        return Expectation(name, None, None, None, (), text(entry['not_applicable']))
    control = entry['control']
    text(control['key'])
    if type(control['version']) is not int or control['version'] < 1 or not isinstance(entry['subject'], dict):
        raise ValueError('use a positive control version and typed Audit subject object')
    codes = tuple(text(code) for code in entry['accepted_codes'])
    if not codes:
        raise ValueError('declare acceptable categorical outcomes; reconcile narrative controls manually')
    run_id = text(entry['run_id']) if entry.get('run_id') is not None else None
    return Expectation(name, control, entry['subject'], run_id, codes, None)


def load_task(path: str) -> Task:
    raw = json.loads(Path(path).read_text())
    project = text(raw['project_id'])
    entries = tuple(expectation(entry) for entry in raw['expectations'])
    if not entries or len({entry.name for entry in entries}) != len(entries):
        raise ValueError('declare a nonempty population with distinct expectation names')
    return Task(project, entries)


def read_selection(binary: str, project: str, selection: dict) -> tuple[list[dict], int]:
    items = []
    while True:
        request = {'action': 'list', 'project_id': project, 'filter': selection}
        completed = subprocess.run([binary, 'audit', 'read', '-'], input=json.dumps(request),
                                   text=True, capture_output=True, timeout=30, check=True)
        page = json.loads(completed.stdout)
        if page.get('ok') is not True or page.get('response') != 'page' or page.get('project_id') != project:
            raise ValueError('project-bound summary page not acknowledged')
        cutoff = selection.get('cutoff')
        if cutoff is not None and page['cutoff'] != cutoff:
            raise ValueError('history changed during pinned read')
        items.extend(page['items'])
        cursor = page.get('next_cursor')
        if cursor is None:
            return items, page['cutoff']
        if type(cursor) is not int or cursor <= selection.get('after', 0) or cursor > page['cutoff']:
            raise ValueError('history cursor did not advance within cutoff')
        selection.update(after=cursor, cutoff=page['cutoff'])


def read_summaries(binary: str, task: Task) -> dict[str, dict]:
    identifiers = sorted({entry.run_id for entry in task.expectations if entry.run_id})
    summaries, cutoff = {}, None
    # The registry owns correction/category projection. Bound each selection to
    # its public 64-ID limit and pin one cutoff across chunks and page reads.
    for offset in range(0, len(identifiers), 64):
        items, cutoff = read_selection(binary, task.project_id,
                                       {'run_ids': identifiers[offset:offset + 64], 'cutoff': cutoff})
        summaries.update((item['id'], item) for item in items)
    return summaries


def reconcile(entry: Expectation, summaries: dict[str, dict] | None) -> dict:
    item = {'name': entry.name}
    if entry.not_applicable:
        return dict(item, state='not_applicable', reason=entry.not_applicable)
    item.update(control=entry.control, subject=entry.subject, run_id=entry.run_id)
    if not entry.run_id:
        return dict(item, state='missing', reason='No run ID; queued input and Work history are not Audit receipts')
    if summaries is None:
        return dict(item, state='unconfirmed', reason='Unable to read the complete project-bound summary selection')
    summary = summaries.get(entry.run_id)
    if summary is None:
        return dict(item, state='unacknowledged', reason='No run with this ID in the project at the pinned cutoff')
    matched = summary['control'] == entry.control and summary['subject'] == entry.subject
    acceptable = (summary['status'] == 'completed' and summary.get('result_code') in entry.accepted_codes
                  and summary['findings'] == 0)
    outcome = 'recorded' if acceptable else 'attention'
    state = outcome if matched else 'mismatch'
    reasons = {'recorded': 'Matched receipt', 'attention': 'Outcome or findings require explicit follow-up',
               'mismatch': 'Control version or exact source differs from task population'}
    return dict(item, state=state, ordinal=summary['ordinal'], status=summary['status'],
                result_code=summary.get('result_code'), findings=summary['findings'], reason=reasons[state])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('plan', help='caller-owned task population JSON (store outside tracked files)')
    parser.add_argument('--nexus', default='nexus', help='Nexus CLI executable')
    args = parser.parse_args()
    try:
        task = load_task(args.plan)
    except (OSError, ValueError, KeyError, TypeError):
        print('Invalid task plan: pin project, control versions, subjects and accepted codes.', file=sys.stderr)
        return 2
    try:
        summaries = read_summaries(args.nexus, task)
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        # Do not expose CLI stderr: a misconfigured service may include private data.
        summaries = None
    items = [reconcile(entry, summaries) for entry in task.expectations]
    complete = all(item['state'] in ('recorded', 'not_applicable') for item in items)
    print(json.dumps({'project_id': task.project_id, 'complete': complete, 'items': items}, indent=2))
    return 0 if complete else 1


if __name__ == '__main__':
    sys.exit(main())
