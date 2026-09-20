---
name: tdd
description: Implement behavioral features and remediate bugs or review findings with observable test-first red-green-refactor evidence. Use whenever production behavior is added or corrected; do not use for documentation-only edits, formatting, generated artifacts, or pure deletion that changes no supported behavior.
---

# Test-Driven Development

Production code follows the test. A test written after an implementation is coverage, not TDD.

## Required sequence

1. Identify the smallest externally meaningful behavior that is absent or wrong. Prefer the public boundary where a caller observes it.
2. Write one focused test for that behavior without changing production code.
3. Run the narrowest command that executes the test and observe it fail for the intended behavioral reason. Record the command and the relevant failure. A compile error caused by the intentionally missing typed API may be a valid red; syntax errors, broken fixtures, unavailable infrastructure, and deliberately false assertions are not.
4. If the new test passes, it has not demonstrated the change. Strengthen or correct it and rerun until it fails for the intended reason. Do not proceed to production code on an unexplained pass.
5. Make the smallest production change that satisfies the test. Do not add speculative cases or unrelated cleanup during the green step.
6. Run the identical narrow test command and observe it pass.
7. Run the relevant broader suite. Refactor only while green, rerunning the focused test and affected suite after the refactor.

Do not weaken, delete, skip, or conditionally bypass the reproducing test to obtain green. Keep it as a regression test unless the supported behavior itself is later removed.

## Review findings and reported issues

Treat every accepted behavioral issue from human review, model review, static analysis, an incident, or a bug report as a request for a reproducer first.

- Verify that the finding describes reachable behavior in the current code.
- Add and run a test that fails by exhibiting that behavior before applying the remediation.
- If the behavior cannot be reproduced, do not implement a speculative fix. Report the evidence and reject or defer the finding.
- If a safe test genuinely cannot be automated, stop and explain why before changing production code; do not claim TDD evidence.

An already-existing failing test can supply the red phase only when it is run before remediation and fails for the issue being addressed.

## Recovering when implementation came first

Do not manufacture a red result. When safe and local, temporarily restore the pre-fix production state while retaining the new test, run it to capture the real failure, then reapply the implementation and prove green. If restoring the prior state would be destructive or would disturb unrelated user work, disclose that the change was not test-first and use the test as regression coverage; do not label the work TDD.

## Handoff evidence

Report the focused red command and expected failure, the same command passing after implementation, and the broader validation command. The observable red and green runs are part of the work product, not optional narration.
