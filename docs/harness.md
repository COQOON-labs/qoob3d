# Quality harness for AI agents

How QOOB3D makes sure AI agents can write a lot of code without the quality dropping.
Decisions: ADR-0037 … ADR-0045. Which rule is checked how: `harness/rules.md`.

## Principles

1. **Deterministic wherever possible.** A tool checks, not the agent's diligence.
2. **Brake locally, block in CI.** Locally an agent has the human's permissions. The actual
   wall is CI and the GitHub rules (ADR-0037).
3. **The harness belongs to the human.** Agents cannot change it, only propose changes.
   Unlock: start Claude Code with `QOOB3D_HARNESS_UNLOCK=1`. The unlock lifts only the
   harness-file protection; fixture, role and bypass rules stay active, and no agent may edit
   settings, shell startup files or `launchctl` state that could set the variable (ADR-0043).
4. **Independent oracles.** Tests are written in a different context than the code (ADR-0039),
   the critic judges with a fresh context, a verifier tries to refute it (ADR-0041).
5. **The harness checks itself** (ADR-0042).

## Layers

```mermaid
flowchart LR
    A[Agent wants to change a file] --> G{Guard<br/>PreToolUse}
    G -- harness / fixture /<br/>role switch / --no-verify /<br/>self-unlock --> X[blocked]
    G --> E[Edit]
    E --> S{Stop hook<br/>check --fast}
    S --> C[commit-msg<br/>identity · AI attribution]
    C --> P{pre-push<br/>policy · gitleaks ·<br/>critic record · check}
    P --> CI{CI<br/>policy · selftest ·<br/>check macOS · mutants}
    CI --> M[(main<br/>branch protection)]
```

| Check | Local brake | Wall in CI |
| --- | --- | --- |
| fmt, clippy (strict lints), tests, reference models | stop hook, pre-push | `check (macOS)` |
| Smoke test, supply chain (cargo-deny) | pre-push (`check` without `--fast`) | `check (macOS)`, `policy` |
| Hygiene (tests in their own files, lint suppressions capped, English only, CODEOWNERS covers the harness) | stop hook, pre-push | `policy`, `check` |
| Rule register complete | stop hook, pre-push | `check` |
| Commit policy (identity, no AI attribution, tests/code separate, fixtures) | commit-msg, pre-push | `policy` |
| Secrets (gitleaks), supply chain (cargo-deny), unused dependencies | pre-push (gitleaks, deny) | `policy` |
| Test quality (mutation tests) | – | `mutants` |
| Harness works (sabotage corpus) | `cargo xtask selftest` | `selftest`, Mondays |
| Harness files, roles, bypasses, self-unlock | guard | CODEOWNERS review |
| Critic review | pre-push (`xtask review`) | – (record stays local) |

## Roles (Claude Code subagents)

| Agent | May | Purpose |
| --- | --- | --- |
| `test-author` | write tests only | tests from the specification, before the implementation |
| `implementer` | write production code only | make tests green, with as little code as possible |
| `critic` | read only | diff against `harness/critic-checklist.md` |
| `verifier` | read only | refute or confirm a single finding |
| `harness-auditor` | read only | check the harness for fitness for purpose |

Workflows as skills: `/deliver` (feature or bugfix), `/critic-review`,
`/usability-review`, `/harness-audit`.

## For humans

```sh
cargo xtask setup                 # once: activate Git hooks
QOOB3D_HARNESS_UNLOCK=1 claude    # session in which agents may change the harness
QOOB3D_SKIP_REVIEW=1 git push     # your own push without a critic record
cargo xtask selftest              # check the harness against all sabotage cases
```

**Once on GitHub** (ADR-0037):

1. Ruleset for `main`: pull request required, required checks `policy`,
   `selftest (harness vs. sabotage)`, `check (macOS)`, `mutants (changed code)`, no
   force push, no deletion, **no bypass list**. Allow only merge commits: squash and rebase
   merges on GitHub get the committer `GitHub` and fail the identity rule (ADR-0044).
2. "Require review from Code Owners" (`.github/CODEOWNERS` names the owner of every harness
   path; `cargo xtask hygiene` keeps it complete).
3. Recommended: a separate GitHub identity for the agent (bot account or app) with
   write access to branches, without merge and admin rights. Only then is the wall
   complete.

## Limits (honestly)

- A determined agent can get around local brakes: the guard detects shell writes
  heuristically, the role ledger and the critic record live in directories the agent
  can reach. CI catches this as long as the agent cannot merge with admin rights.
- The critic is not deterministic. It only blocks through verified findings.
- Usability and accessibility are so far checked by the critic (`/usability-review`). Still
  missing as deterministic checks: AccessKit names of all controls, contrast of the theme
  colors, target sizes, interaction budgets for core workflows.
- Fuzzing of the file formats (`.qbd`) is still missing.
