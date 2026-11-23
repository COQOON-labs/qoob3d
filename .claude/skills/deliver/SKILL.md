---
name: deliver
description: Workflow for a feature or bugfix in QOOB3D with separate roles – specification, tests (test-author), implementation (implementer), check, critic. Optional; use it when independent tests are worth the extra round, e.g. for geometry, file formats and bugfixes.
---

# Deliver a feature or bugfix with separate roles

Tests written in a different context than the code are an independent oracle. The workflow is
voluntary: for small or mechanical changes, write code and tests directly.

You are the **orchestrator** and delegate tests and production code.

1. **Clarify the specification.** Requirement IDs in `docs/requirements/`, affected ADRs,
   acceptance criteria as checkable sentences. Clarify open questions with the user now. For a
   bug: describe the reproduction (input → wrong result → expected result).
2. **Tests: subagent `test-author`.** Pass on only the specification from step 1, no
   implementation ideas. Result: tests that are red for the right reason.
3. **Implementation: subagent `implementer`.** Pass on the specification and the list of
   red tests. Result: tests green, `cargo xtask check` green.
   If the implementer reports a wrong test, the question goes back to the
   `test-author` (or to the user), never directly into the test.
4. **Check:** run `cargo xtask check` yourself. For UI changes run
   `cargo xtask screenshot` and look at the PNGs.
5. **Critic (optional):** skill `critic-review` over the diff.
6. **Handover to the user:** what is implemented, which tests prove it, check result, test
   list for the user. Commits only on instruction, without AI attribution.
