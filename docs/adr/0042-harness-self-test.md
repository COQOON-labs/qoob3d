---
id: ADR-0042
title: The harness checks itself – sabotage corpus, weekly CI run and audit
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [quality, ai, tests]
supersedes: []
superseded_by: null
---

## Context

A check that never fires looks exactly like a broken check. If the
code changes (new directories, renamed files), rules can silently stop matching anything. Our
own ZIP reader missed a writer bug (ADR-0033): verifiers need to be verified
too.

## Decision

1. **Sabotage corpus** `harness/sabotage/*.case`: every case is a deliberate violation
   (patch, commit message or hook input), the command that must reject it, and the
   expected text in the output. That way a failure for the wrong reason does not count.
2. `cargo xtask selftest` plays through every case in a copy of the working tree (a separate
   Git repo under `target/selftest/`). The real repo stays untouched.
3. CI runs the self-test on every PR and every Monday (drift detection), plus the
   full mutation tests of the pure crates as a report.
4. **Audit:** the skill `harness-audit` (subagent `harness-auditor`) checks rule coverage,
   gaps, false alarms, run times and fitness for purpose. It delivers proposals and changes
   nothing. At least once per milestone.
5. **Every new rule and every gap found gets a sabotage case.**

## Consequences

- Broken or circumvented checks surface within a week at the latest.
- Patches in the corpus must be maintained when their target files change. The self-test
  reports this as "patch no longer applies".
- The first run builds `qoob3d-core` and `xtask` in a separate target directory.

## Rejected alternatives

- **Unit tests of the check functions alone:** do not show that the check works in the overall
  flow (hook → command → exit code).

## References

- Code: `xtask/src/selftest.rs`, `harness/sabotage/`, `.claude/skills/harness-audit/SKILL.md`
- Related: ADR-0033, ADR-0037, ADR-0040
