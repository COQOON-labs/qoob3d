---
id: ADR-0040
title: Every rule has a check – rule register, style register and a stricter lint policy
status: accepted
date: 2026-10-14
crates: [xtask, qoob3d-core, qoob3d-sketch, qoob3d-kernel, qoob3d-render, qoob3d-app]
tags: [quality, ai, style]
supersedes: []
superseded_by: null
---

## Context

Rules that only live in CLAUDE.md turn into wishes over time. Otherwise agents decide matters of
taste (naming, control flow, level of abstraction) anew every time, and
agent B rebuilds what agent A wrote. Until now, `#[allow]` with a comment was permitted;
nobody checked whether an exception was still needed.

## Decision

1. **Rule register** `harness/rules.md`: every rule names its checks
   (`xtask:`, `lint:`, `ci:`, `hook:`, `githook:`, `critic:`, `agent:`, `skill:`, `test:`,
   `manual`). `cargo xtask rules` (part of `check`) verifies that every named check
   exists.
2. **Style register** `docs/design/code-style.md` (ST-NN): only what is written there applies. New
   entries are proposed by an agent, the human decides. No rebuilding of code according to one's
   own taste; conversions after new entries go in separate commits. The critic checks this
   (K-12).
3. **Lint policy** (`[workspace.lints]`, `clippy.toml`):
   - Exceptions only as `#[expect(lint, reason = "…")]` (`allow_attributes`,
     `allow_attributes_without_reason`). `expect` speaks up as soon as the exception is
     unnecessary. During the conversion, an outdated exception surfaced immediately this way.
   - The number of exceptions is capped (`harness/ratchet.txt`, `xtask hygiene`) and may
     only go down.
   - `unreachable_pub`: `pub` only for the crate's API, so that `dead_code` finds unused parts
     (rule "dead code").
   - `unwrap_used` outside of tests, `dbg_macro`, `todo`, `unimplemented`,
     `ignore_without_reason`.
   - Limits: at most 100 lines per function, cognitive complexity at most 25. Older
     violations are marked as `expect` and are split the next time they are touched.
   - A few low-noise style lints from `pedantic`. Lints that only produce rework without benefit
     (`match_same_arms`, `float_cmp`, `needless_pass_by_value`, `must_use_candidate`, casts),
     stay off.
4. Every crate inherits the workspace lints (`[lints] workspace = true`, `xtask hygiene`).

## Consequences

- Replaces the rule "no `#[allow(…)]` without a justifying comment" of ADR-0013
  (decision 5): suppressions are `#[expect(…, reason)]` only, and capped.
- A rule without a check stands out. Whatever only the critic checks is visible and a
  candidate for automation (harness audit, ADR-0042).
- Taste is decided in one place and is the same for everyone.
- Public items that merely go unused by other workspace crates are still not found by the
  compiler. The critic (K-11) remains responsible for that.

## Rejected alternatives

- **All of `clippy::pedantic`:** over 700 warnings, mostly rework without benefit
  (rule 1: as little code as possible, also for changes).
- **Raise the thresholds in `clippy.toml` to today's maximum:** would allow every function
  177 lines. Individual marked exceptions are more honest.

## References

- Code: `Cargo.toml`, `clippy.toml`, `xtask/src/rules.rs`, `xtask/src/hygiene.rs`,
  `harness/rules.md`, `harness/ratchet.txt`, `docs/design/code-style.md`
- Related: ADR-0013, ADR-0037, ADR-0041
