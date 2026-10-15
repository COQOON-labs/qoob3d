---
id: ADR-0045
title: The repository is English; German exists only as the UI language pack
status: accepted
date: 2026-10-14
crates: [xtask, qoob3d-app]
tags: [documentation, process, i18n]
supersedes: []
superseded_by: null
---

## Context

Until now code, comments and commit messages were English, while documentation (`docs/`,
CLAUDE.md files), the harness texts, agent and skill definitions and many tool messages were
German (ADR-0020, consequences). The mix made the repository harder to share, forced agents to
switch languages within one task, and left German file names and identifiers in paths. The
maintainer chats in German, but wants the repository itself in English.

## Decision

1. **Everything in the repository is English:** code, comments, tool output (xtask, hooks,
   smoke test, CLI), documentation, ADRs, CLAUDE.md files, the harness (rules, critic
   checklist, sabotage cases), agent and skill definitions, CI step names, commit messages,
   file names. The chat with the user may be German.
2. **Exceptions:** the German language pack `crates/qoob3d-app/locales/de/` and tests that
   assert German translations.
3. **ADR format in English:** statuses `proposed`, `accepted`, `superseded`, `rejected`;
   sections `Context`, `Decision`, `Consequences`, `Rejected alternatives`, `References`.
   `cargo xtask docs` enforces statuses and required sections.
4. **Deterministic check:** `cargo xtask hygiene` rejects German umlauts and the sharp s in
   every versioned text file. Exceptions are listed in `harness/language-allowlist.txt` (paths and
   proper names such as Möller–Trumbore or the committer's name) and stay few.
5. The existing German documents and ADRs were translated once without changing their content.

## Consequences

- Replaces the consequence "documentation stays German" of ADR-0020; the UI language decision
  of ADR-0020 is unchanged.
- The umlaut check catches most German text but not German written without umlauts; the
  critic and reviews cover the rest.
- Tests that assert German UI texts with umlauts write them as escapes (`\u{e4}`) or are added to
  the allowlist with a reason.

## Rejected alternatives

- **Bilingual documentation:** doubles the maintenance, and the copies drift apart.
- **A full language detector:** heavy dependency and false positives for an effect the
  umlaut check already achieves for most cases.

## References

- Code: `xtask/src/hygiene.rs`, `xtask/src/docs.rs`, `harness/language-allowlist.txt`
- Sabotage cases: `harness/sabotage/hygiene-german-text.case`, `adr-german-status.case`
- Related: ADR-0001, ADR-0020, ADR-0040
