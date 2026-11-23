---
name: critic-review
description: Run the AI critic over a revision range, verify the severe findings independently and report them. Optional; use when the user wants a review or before a larger pull request.
---

# Critic review

A second opinion with a fresh context. It is voluntary: nothing blocks a push without it.

1. **Determine the range.** `head` = `HEAD` (the review applies to the committed state).
   `base` = `git merge-base origin/main HEAD`; without a remote, the last state the user
   already knows.
2. **Critic:** subagent `critic` with the range `base..head`, nothing else. Do not pass on the
   author's reasoning; the critic is supposed to judge freshly.
3. **Verification:** for every finding with `blocker` or `major`, run one `verifier` subagent
   per finding in parallel, with exactly that finding. Take over its `verdict` and, if
   applicable, its lowered `severity`. `minor` findings are not verified.
4. **Report to the user:** confirmed findings with location and fix, refuted ones briefly with
   the reason, `minor` as a list. The user decides what is fixed now, later or not at all; the
   pull request description names the result.
