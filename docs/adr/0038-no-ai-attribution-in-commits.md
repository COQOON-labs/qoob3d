---
id: ADR-0038
title: No commit names an AI as author, co-author or generator
status: accepted
date: 2026-10-14
crates: [xtask]
tags: [git, ai, process]
supersedes: []
superseded_by: null
---

## Context

AI tools like to append lines such as `Co-Authored-By: Claude …` or "🤖 Generated with …"
to commit messages. In this project the history should contain no such attributions. The
rule has to be enforced against the tools' defaults.

## Decision

`commits::check_message` rejects a commit message if it contains any of the following:

- a `…-by:` trailer line (`Co-Authored-By`, `Signed-off-by`, `Assisted-by` …) that names an
  AI tool,
- a "Generated with/by" note referring to an AI tool,
- the character 🤖.

Authors and committers whose name or e-mail names an AI tool are rejected as well.
The list of markers is in `xtask/src/commits.rs` (`AI_MARKERS`).

This is checked in three places: in the `commit-msg` hook, in the `pre-push` hook (every pushed
commit) and in CI (`policy`).

## Consequences

- Agents write commit messages without AI attribution (agent memory, CLAUDE.md).
- A human whose first name is Claude cannot be entered as a co-author. We
  accept that.
- Existing commits with such lines must be rewritten before the first push.

## Rejected alternatives

- **Check only `Co-Authored-By`:** tools also use other trailers and footers.
- **Forbid every mention of an AI in the text:** would hit normal messages
  ("Claude Code hook", "cursor pivot").

## References

- Code: `xtask/src/commits.rs`, `.githooks/commit-msg`, `.githooks/pre-push`, CI job `policy`
- Sabotage cases: `harness/sabotage/ai-co-author.case`, `ai-generated.case`
- Related: ADR-0037
