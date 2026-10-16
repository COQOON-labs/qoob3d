---
name: critic-review
description: Run the AI critics the review plan asks for over a revision range, verify findings independently and write the review record that the pre-push hook requires (ADR-0041, ADR-0052). Use before every push with production code and when the user wants a review.
---

# Critic review (ADR-0041, ADR-0052)

The pre-push hook (`cargo xtask review`) requires, for every pushed state with production code,
a record `target/critic/<tree>.json` (version 2) that covers the review plan of the pushed range,
rotates the general model after a fix, satisfies the stopping rule, and in which every
`blocker`/`major` finding is refuted, or accepted by the user with an `accepted_reason`.
Unverified and `fixed` findings still block (a fix changes the tree and needs a new review).

1. **Determine the range.** `head` = `HEAD` (the review applies to the committed state; have
   uncommitted changes committed first, commits only on the user's instruction).
   `base` = `git merge-base origin/main HEAD`; without a remote, the last state the user
   already knows. `tree` = `git rev-parse HEAD^{tree}`.
2. **Plan.** `cargo xtask review --plan HEAD <base>..HEAD` prints the tier, its reasons and the
   required reviews. Tier `none` needs no record.
3. **Models and round.** Models are the Agent tool's `model` values: `opus`, `fable`, `sonnet`.
   If this review follows a fix of findings from an earlier record of this branch, set
   `previous_tree` to that record's tree, use `round` = its highest round + 1 for every
   review, and give a whole-diff general review a model outside the previous record's
   whole-diff pair (its first two general reviews on different models). Otherwise
   `previous_tree` is `null` and `round` is 1.
4. **Critics, all in one message so they run in parallel.** Subagent `critic` per required
   review, with the range `base..head`, the lens and the plan's reasons, nothing else. Do not
   pass on the author's reasoning; the critic is supposed to judge freshly.
   - `standard`: one `general` critic.
   - `high`: two `general` critics on two different models, plus one critic per required
     lens (`tests`; `security` for harness changes; `robustness` at the OCCT boundary or the
     file format). A diff over 800 changed lines additionally gets one `general` critic per
     file group; record those with `"scope": "files"`. They do not count as whole-diff
     reviews for the two models, the stopping rule or the rotation.
5. **Merge the findings.** Give every review an id (`r1`, `r2`, …). Findings that name the same
   defect become one finding whose `found_by` lists every review that reported it.
6. **Verification:** for every finding with `blocker` or `major`, run one `verifier` subagent
   per finding in parallel, with exactly that finding. Take over its `verdict` and, if
   applicable, its lowered `severity`. `minor` findings get `"verdict": "unverified"`.
7. **Stopping rule (tier `high`).** With the two general reviews a and b: n₁, n₂ = non-refuted
   `blocker`/`major` findings found by a, by b; m = found by both.
   R̂ = (n₁+1)(n₂+1)/(m+1) − 1 − (n₁+n₂−m). If R̂ ≥ 1, run one more `general` critic on the
   third model now, merge and verify its findings (steps 5–6).
8. **Write the record** to `target/critic/<tree>.json`:
   ```json
   {
     "version": 2,
     "tree": "<tree>", "base": "<base>", "head": "<sha>",
     "tier": "high", "previous_tree": null,
     "reviews": [
       {"id": "r1", "model": "opus", "lens": "general", "round": 1},
       {"id": "r2", "model": "fable", "lens": "general", "round": 1},
       {"id": "r3", "model": "sonnet", "lens": "tests", "round": 1}
     ],
     "findings": [
       {"rule": "K-09", "severity": "major", "verdict": "confirmed", "resolution": "open",
        "found_by": ["r1", "r2"], "file": "…", "line": 1, "summary": "…",
        "consequence": "…", "fix": "…", "evidence": "<from the verifier>"}
     ]
   }
   ```
   `resolution`: `open` (initial state), `fixed` (fixed in a later commit, then a new review
   for the new state), `accepted` (only the **user** may accept a confirmed finding; reason in
   `"accepted_reason"`). Then `cargo xtask review HEAD <base>..HEAD` must print `review: ok`.
9. **Escapes.** A confirmed finding in code that an earlier round already reviewed unchanged,
   or a finding from Copilot, CI or the user that our critics missed, gets a row in
   `docs/escapes.md` (ADR-0052).
10. **Report to the user:** tier and reasons, confirmed findings with location and fix,
    refuted ones briefly with the reason, `minor` as a list, R̂ for tier `high`. The PR
    description carries the same summary. Fixes go through the skill `deliver` (mind the
    roles).
