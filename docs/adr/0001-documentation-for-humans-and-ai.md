---
id: ADR-0001
title: Decisions as ADRs, tiered CLAUDE.md files and machine-checked documentation
status: accepted
date: 2026-10-13
crates: [xtask]
tags: [documentation, process, ai]
supersedes: []
superseded_by: null
---

## Context

QOOB3D is developed mostly with an AI agent (Claude Code), across many sessions. The agent
has no memory between sessions except what is in the repo. Decisions made only in chat get
lost or are later silently reverted. A single long documentation file becomes hard to
navigate and quietly goes stale.

## Decision

1. **Every architecture or design decision is a separate file** `docs/adr/NNNN-title.md`
   with a YAML front matter (`id, title, status, date, crates, tags, supersedes, superseded_by`)
   and the sections Context / Decision / Consequences / Rejected alternatives /
   References. Template: `docs/adr/0000-template.md`. Index: `docs/adr/README.md`.
2. **Accepted ADRs are immutable.** A change of the decision is a new ADR that supersedes
   the old one. This keeps the *why* traceable.
3. **Tiered entry points for the agent:**
   - `CLAUDE.md` (root): short – purpose, commands, working rules, pitfalls, links.
   - `crates/<crate>/CLAUDE.md`: purpose, rules and relevant ADRs of this crate. Claude Code
     loads them automatically as soon as work happens in that directory.
   - `docs/architecture.md`: overall picture (data flow, threads, crate boundaries).
   - `docs/design/`: design specifications (theme, layout, controls) – descriptive,
     not decisions.
4. **Code references decisions** with `ADR-NNNN` in comments at the places that implement a
   decision.
5. **`cargo xtask docs` checks mechanically** (and is part of `cargo xtask check`):
   front matter complete and valid, ID matches the file name, index lists every ADR with
   current title and status, every `ADR-NNNN` reference in code and docs exists,
   `supersedes`/`superseded_by` are mutually consistent.

## Consequences

- The agent finds decisions via the index or by searching for `ADR-` and can read
  their status mechanically.
- Changing a decision costs a new file – intentionally, because it forces reflection.
- If a new decision is made while working, the ADR belongs in the same commit.

## Rejected alternatives

- **Everything in one CLAUDE.md** – grows uncontrolled, the context window fills with
  irrelevant content, history gets lost.
- **Wiki / external tool** – not reachable for the agent, not versioned with the code.

## References

- `CLAUDE.md`, `docs/adr/README.md`, `xtask/src/docs.rs`
