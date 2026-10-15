---
id: ADR-0044
title: Every commit uses the project identity, and CODEOWNERS covers every harness path
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [git, ai, process, security]
supersedes: []
superseded_by: null
---

## Context

The maintainer works with several git identities on the same machine (a personal one for this
project, others for employers). Agents commit with whatever identity the local configuration
provides, so a wrong global identity or a tool default can end up in the history unnoticed.
ADR-0038 only rejects identities that name an AI tool.

ADR-0037 requires an owner review for harness changes on GitHub, but the repository had no
`.github/CODEOWNERS` yet, and nothing would notice a harness path without an owner.

## Decision

1. **Commit identity:** `harness/identity.txt` names the only allowed identity
   (`Name <mail>`). `cargo xtask commits` rejects every non-merge commit whose author or
   committer differs from it. The `commit-msg` hook checks the identity of the commit being
   created (`git var GIT_AUTHOR_IDENT`/`GIT_COMMITTER_IDENT`); `pre-push` and the CI job
   `policy` check every pushed commit.
2. **Code owners:** `.github/CODEOWNERS` assigns `* @Ben-Gee` and lists every harness path
   (`paths::HARNESS`) explicitly. `cargo xtask hygiene` fails if a harness path has no owner
   line.
3. The self-test commits as the project identity, so each sabotage case tests exactly one
   violation; a case can set another identity with `identity:`.

## Consequences

- A commit with a wrong identity is stopped by the `commit-msg` hook at the latest, and cannot
  be merged.
- **GitHub merges:** squash and rebase merges on GitHub create commits with the committer
  `GitHub <noreply@github.com>`; the `policy` run on `main` rejects them. Pull requests are
  merged with merge commits (skipped by `--no-merges`) or rebased locally and pushed.
- Another person or a bot account (ADR-0037) needs its own entry; `harness/identity.txt` then
  becomes a list. Not needed yet (YAGNI).
- Changing the identity is a harness change (owner review).

## Rejected alternatives

- **Only the e-mail address:** the name is part of what appears in the history; checking the
  full identity is just as simple.
- **Signed commits as proof of identity:** stronger, but needs key management on every machine
  and in CI; can come later.

## References

- Code: `xtask/src/commits.rs`, `xtask/src/hygiene.rs`, `harness/identity.txt`,
  `.github/CODEOWNERS`
- Sabotage cases: `harness/sabotage/commit-wrong-identity.case`,
  `codeowners-missing-harness-path.case`
- Related: ADR-0037, ADR-0038, ADR-0042
