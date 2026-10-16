---
id: ADR-0052
title: Review depth follows the risk of a diff, reviewers differ, and escaped findings are logged
status: accepted
date: 2026-10-16
crates: [xtask]
tags: [quality, ai, review]
supersedes: []
superseded_by: null
---

## Context

ADR-0041 has one critic review each pushed state. A model reviewer is a sample: it finds a
part of the defects and misses others systematically. Iterating until the same reviewer is
quiet fits the code to that reviewer's blind spots. A second model, a changed context or
GitHub's Copilot review then still finds important defects. "No findings left" is therefore
no stopping criterion.

Escaped defects so far reached us through CI on another OS, through the user and through
later review rounds. Nothing records them, so the same class can escape again.

Review costs time and tokens. Most diffs are small and low-risk; a few touch the harness, the
OCCT boundary or the file format, where a miss is expensive.

## Decision

1. **Review plan from the diff.** `cargo xtask review --plan <tip> <range>` derives a tier
   deterministically and prints the reasons:
   - `none`: neither production code nor a harness path changed. (ADR-0041 skipped every
     range without production code; a change to hooks, CI or the checklist now needs a
     review too.)
   - `standard`: production code without a high-risk trigger. One general critic, one
     verifier per `blocker`/`major` finding.
   - `high`: any of: a harness path (ADR-0037); the OCCT boundary (C++ sources, cxx bridges,
     `build.rs`, `third_party/`); the project format, its migrations or the exporters
     (ADR-0030, ADR-0034); more than 400 changed lines of production code; a file with an
     escaped finding in the last 180 days (decision 5). Two general critics on different
     models, plus lens critics: always tests (K-13), and per trigger harness bypasses and
     security (K-10) or robustness at the boundary (K-09, K-10).
   The path lists live in xtask next to `paths::HARNESS`. The pre-push hook plans the whole
   branch against the remote's `main`, not only the newly pushed commits, so a large change
   cannot be split into small pushes. Escape rows are read at the tip and at the range's
   boundary commits, so a range cannot drop a row that applies to it.
2. **Diverse reviewers.** Critics run with fresh context, in parallel, on models from the
   available set (Opus, Fable, Sonnet). Every round after a fix uses, for a general review
   of the whole diff, a model outside the previous round's whole-diff pair (the two samples
   of decision 4; a third stopping-rule critic and file-group critics do not count).
   Rotation is not opt-in: the gate finds the earlier records of the branch itself
   (version-2 records of another tree whose `head` is in the planned range).
3. **Record, version 2.** `target/critic/<tree>.json` additionally names the `tier` (at
   least the plan's), the `previous_tree`, the reviews (`id`, `model`, `lens`,
   `round`, and `scope: files` for a file-group critic) and for every finding `found_by`
   (review ids, duplicates merged). If the branch has an earlier record, `previous_tree`
   must name one with the highest round; it is `null` only for the first record of a
   branch. `cargo xtask review` checks that the record covers the
   plan of the pushed range, the model rotation and the stopping rule. The record stays local
   and forgeable (ADR-0041); every PR description carries its summary.
4. **Stopping rule (capture–recapture).** In tier `high`, the two general critics are two
   samples of the same defect population. With n₁ and n₂ non-refuted `blocker`/`major`
   findings and m found by both, the Chapman estimate of all such defects is
   N̂ = (n₁+1)(n₂+1)/(m+1) − 1, the estimate of undiscovered ones R̂ = N̂ − (n₁+n₂−m). A state
   with R̂ ≥ 1 needs one more general critic on a third model before the push. Correlated
   misses make R̂ an underestimate; the threshold is calibrated from the records
   (decision 6).
5. **Escape log** `docs/escapes.md`. Every defect that a check or review should have caught
   earlier gets a row: date, source (CI, Copilot, user, later round), what escaped, file,
   which layer missed it, its class and the deterministic check proposed for the class.
   Rows are never deleted; only the status changes. The harness audit (ADR-0042) reads the
   log: a class with two escapes gets a deterministic check, or else a checklist item or a
   lens. Every escape also becomes a calibration seed.
6. **Calibration by seeded defects.** `harness/critic-seeds/` holds known defects as patches
   on real past states, each with its expected finding. The skill `critic-calibrate` runs
   the critic configurations of the plan over the seeds and reports recall per model and
   lens. It runs after every change to the checklist, the agents or the model set, and at
   least monthly. The results choose models and lenses in the plan. (Follow-up; until it
   exists the thresholds are uncalibrated starting values, open item in CLAUDE.md.)
7. **An outside reviewer.** Copilot code review runs on every PR (GitHub ruleset; open item
   until the ruleset has it). Its
   `blocker`/`major`-grade comments go to a verifier like critic findings; the ones our
   critics missed go into the escape log.
8. **Deterministic hints first.** The critic only runs after `cargo xtask check` is green. It
   receives the plan's reasons and, once available, the uncovered changed lines and the
   surviving mutants of the diff. A diff over 800 changed lines is also reviewed per file
   group in parallel, beside one whole-diff review for cross-cutting defects.

## Consequences

- Standard diffs cost what they cost today. High-risk diffs run about four to five agents in
  parallel, so wall-clock time stays near one review while tokens grow accordingly.
- A model reviewer can no longer be "satisfied" by iteration alone: the rotation and the
  estimate force a fresh sample where defects are likely left.
- With small counts the estimate is coarse. It is a signal for one more look, not a proof.
- The escape log and calibration make the critic's recall measurable; checks, lenses and
  models change based on data instead of impressions.
- Follow-ups with their own ADRs: diff coverage in CI, a mutation score threshold and more
  crates under mutation testing, property-based and metamorphic geometry tests, fuzzing of
  the file formats. A local cross-vendor reviewer (Copilot CLI) is evaluated separately.

## Rejected alternatives

- **Iterate one critic until it is quiet:** fits the code to that critic's blind spots.
- **The full ensemble on every diff:** multiplies cost and wait for small, low-risk changes.
- **Critic in CI now:** needs an API key and a cost budget (ADR-0041); the outside reviewer
  in CI is Copilot for now.

## References

- Code: `xtask/src/review.rs`, `.claude/skills/critic-review/SKILL.md`,
  `.claude/agents/critic.md`, `harness/critic-checklist.md`, `docs/escapes.md`
- Related: ADR-0037, ADR-0041, ADR-0042
