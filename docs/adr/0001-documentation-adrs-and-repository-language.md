---
id: ADR-0001
title: Decisions live in about 15 topic ADRs that are updated in place, the repository is English, and xtask checks the documentation
status: accepted
date: 2026-11-23
crates: [xtask, qoob3d-app]
tags: [documentation, process, ai, i18n]
supersedes: []
superseded_by: null
---

## Context

QOOB3D is developed mostly with an AI agent across many sessions. The agent has no memory
between sessions except what is in the repository; decisions made only in chat get lost or
are later silently reverted. A single long documentation file becomes hard to navigate and
quietly goes stale.

The first approach was one immutable ADR per decision, superseded by a new one on every
change. After six weeks there were 56 of them; many described a path that no longer exists,
and finding the current state of a topic meant reading a chain of superseding files. The
maintainer chats in German but wants the repository in English; a mixed repository forced
agents to switch languages within one task.

This topic absorbs the former decision 0045.

## Decision

### Topic ADRs

- Decisions are grouped by topic into about 15 files `docs/adr/NNNN-slug.md`. Each topic
  keeps the number of its lead decision. A topic is a living document: when a decision in it
  changes, the file is rewritten to describe the state as it is now. The history is in Git,
  not in the file.
- A new topic, or a new section in an existing one, is written only for decisions that are
  costly to reverse (architecture, file formats, crate boundaries, dependencies, harness).
  It belongs in the same commit as the code.
- Each file has a YAML front matter (`id, title, status, date, crates, tags, supersedes,
  superseded_by`) and the sections Context, Decision, Consequences, then Rejected
  alternatives and References. The rationale and the measurements that still matter stay;
  text that only concerned removed code goes. Template: `docs/adr/0000-template.md`.
  Statuses: `proposed`, `accepted`, `superseded`, `rejected`.
- `docs/adr/README.md` holds the generated index and a table that maps the former numbers to
  their topics.
- Code references a decision with `ADR-NNNN` in a comment where it implements it.

### Tiered entry points

- `CLAUDE.md` (root): purpose, commands, working rules, pitfalls, links.
- `crates/<crate>/CLAUDE.md`: purpose, rules, files and relevant ADRs of that crate. Claude
  Code loads them as soon as work happens in that directory.
- `docs/architecture.md`: the big picture (data flow, threads, crate boundaries).
- `docs/design/`: descriptive specifications (theme, controls, code style), not decisions.
- `docs/requirements/`: requirements with IDs per area.

### Repository language

- Everything in the repository is English: code, comments, tool output, documentation, ADRs,
  CLAUDE.md files, harness, agent and skill definitions, CI step names, commit messages, file
  names. The chat with the user may be German.
- Exceptions: the German language pack `crates/qoob3d-app/locales/de/` and tests that assert
  German translations (the UI language decision is ADR-0020).
- `cargo xtask hygiene` rejects German umlauts and the sharp s in every versioned text file,
  except the paths and proper names in `harness/language-allowlist.txt`.

### Mechanical checks

`cargo xtask docs` (part of `cargo xtask check`) checks: front matter complete and valid, the
id matches the file name, the index lists every ADR with its current title and status, every
`ADR-NNNN` reference in code and docs exists (and outside `docs/adr/` never points at a
superseded or rejected one), supersede links are mutual, every crate has a CLAUDE.md.
`--fix` regenerates the index.

## Consequences

- The current state of a topic is in one file; agents find it via the index or by searching
  for `ADR-`.
- Rewriting a topic loses the old wording from the working tree; `git log -p` on the file
  shows how a decision developed.
- Removing a topic or renumbering means remapping every `ADR-NNNN` reference; the lint finds
  the ones that were missed.
- The umlaut check catches most German text but not German written without umlauts; reviews
  cover the rest. Tests that assert German UI texts write umlauts as escapes (`\u{e4}`).

## Rejected alternatives

- **Immutable ADRs, superseded on every change** – the first approach; 56 files after six
  weeks, the current state was spread over chains of files.
- **Everything in one CLAUDE.md** – grows without control, fills the context window with
  irrelevant content.
- **Wiki or external tool** – not reachable for the agent, not versioned with the code.
- **Bilingual documentation** – doubles the maintenance, and the copies drift apart.
- **A full language detector** – heavy dependency and false positives for an effect the
  umlaut check already has in most cases.

## References

- `CLAUDE.md`, `crates/*/CLAUDE.md`, `docs/architecture.md`, `docs/adr/README.md`,
  `docs/adr/0000-template.md`
- Code: `xtask/src/docs.rs`, `xtask/src/hygiene.rs`, `harness/language-allowlist.txt`
- Sabotage cases: `harness/sabotage/hygiene-german-text.case`,
  `harness/sabotage/adr-german-status.case`, `harness/sabotage/superseded-adr.case`
- Related: ADR-0020, ADR-0037
