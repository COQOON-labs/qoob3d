---
id: ADR-0041
title: AI as critic – checklist, independent verification and a review record as a push condition
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [quality, ai, review]
supersedes: []
superseded_by: null
---

## Context

Many things cannot be checked deterministically: SOLID, readability, duplicates, test quality,
usability. An AI critic can assess them, but is itself not deterministic, finds
spurious problems and tends towards judgments of taste.

## Decision

1. **Checklist** `harness/critic-checklist.md` (K-01 … K-17) with severities and a fixed JSON
   format. A finding needs a location, a consequence and the smallest correction. Taste without a
   style register entry is not a finding.
2. **Roles:** the subagent `critic` reads the diff with a fresh context, without the
   author's reasoning. A separate subagent `verifier` tries to refute every `blocker`/`major`.
   Neither has write permissions.
3. **Record:** the skill `critic-review` writes `target/critic/<tree>.json`. Only the
   human may accept a confirmed severe finding.
4. **Push condition:** `cargo xtask review` in the `pre-push` hook requires, for every state with
   production code, a record for exactly this tree without open, confirmed
   `blocker`/`major` findings. The human skips this with `QOOB3D_SKIP_REVIEW=1`; the guard
   forbids agents this variable.

## Consequences

- The critic only blocks via confirmed findings; spurious problems cost no work.
- The record is local and forgeable. It turns skipping into a deliberate action
  and provides metrics for the harness audit (false-alarm rate).
- A review applies to a committed state; after every correction a new review is due.
- Possible later: critic in CI (needs an API key and a cost budget).

## Rejected alternatives

- **Critic as a hard wall without verification:** too many false alarms, pressure to circumvent.
- **Critic only on request:** gets forgotten, precisely when things are urgent.

## References

- Code: `xtask/src/review.rs`, `.claude/agents/critic.md`, `.claude/agents/verifier.md`,
  `.claude/skills/critic-review/SKILL.md`, `harness/critic-checklist.md`
- Related: ADR-0037, ADR-0040
