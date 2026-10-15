---
name: critic-review
description: Run the AI critic over a revision range, verify findings independently and write the review record that the pre-push hook requires (ADR-0041). Use before every push with production code and when the user wants a review.
---

# Critic review (ADR-0041)

The pre-push hook (`cargo xtask review`) requires, for every pushed state with production code,
a record `target/critic/<tree>.json` in which every `blocker`/`major` finding is refuted, or
accepted by the user with an `accepted_reason`. Unverified and `fixed` findings still block (a fix
changes the tree and needs a new review).

1. **Determine the range.** `head` = `HEAD` (the review applies to the committed state; have
   uncommitted changes committed first, commits only on the user's instruction).
   `base` = `git merge-base origin/main HEAD`; without a remote, the last state the user
   already knows. `tree` = `git rev-parse HEAD^{tree}`.
2. **Critic:** subagent `critic` with the range `base..head`, nothing else. Do not pass on the
   author's reasoning; the critic is supposed to judge freshly.
3. **Verification:** for every finding with `blocker` or `major`, run one `verifier` subagent
   per finding in parallel, with exactly that finding. Take over its `verdict` and, if
   applicable, its lowered `severity`. `minor` findings get `"verdict": "unverified"`.
4. **Write the record** to `target/critic/<tree>.json`:
   ```json
   {
     "tree": "<tree>", "base": "<base>", "head": "<sha>",
     "findings": [
       {"rule": "K-09", "severity": "major", "verdict": "confirmed", "resolution": "open",
        "file": "…", "line": 1, "summary": "…", "consequence": "…", "fix": "…",
        "evidence": "<from the verifier>"}
     ]
   }
   ```
   `resolution`: `open` (initial state), `fixed` (fixed in a later commit, then a new review
   for the new state), `accepted` (only the **user** may accept a confirmed finding; reason in
   `"accepted_reason"`).
5. **Report to the user:** confirmed findings with location and fix, refuted ones briefly with
   the reason, `minor` as a list. Fixes go through the skill `deliver` (mind the roles).
