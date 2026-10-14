---
name: deliver
description: Workflow for every feature and every bugfix in QOOB3D with separate roles – specification, tests (test-author), implementation (implementer), check, critic. Use as soon as production code is to be written or changed.
---

# Deliver a feature or bugfix (ADR-0039, ADR-0041)

You are the **orchestrator**. You write neither tests nor production code yourself, but
delegate. The guard hook binds every agent context to one role: whoever has written tests once
may no longer write production code, and vice versa.

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
   `cargo xtask screenshot` and look at the PNGs. For UI changes also run the skill
   `usability-review` for the affected workflow.
5. **Critic:** skill `critic-review` over the diff. Confirmed `blocker`/`major` findings go,
   depending on their kind, to the `implementer` or the `test-author` and then back to step 4.
6. **Handover to the user:** what is implemented, which tests prove it, check result,
   critic result, test list for the user (CLAUDE.md rule 7).
   Commits only on instruction. Tests and production code go into **separate commits**
   (tests first). No AI attribution in commit messages (ADR-0038).
