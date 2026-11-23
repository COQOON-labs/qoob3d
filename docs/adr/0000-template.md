---
id: ADR-0000
title: Template – short sentence that names the decision
status: template
date: YYYY-MM-DD
crates: []
tags: []
supersedes: []
superseded_by: null
---

<!-- Copy as NNNN-kebab-title.md, next free number, then `cargo xtask docs --fix`.
     Required fields in the front matter: id, title, status, date, crates, tags, supersedes,
     superseded_by.
     status: proposed | accepted | superseded | rejected
     ADRs are topic documents: a changed decision updates the ADR of its topic, the history
     is in Git. A new file only for a new topic. -->

## Context

Which problem, which constraints, which forces are at work? Facts, not opinions.

## Decision

What we do – phrased in the active voice, verifiable ("The kernel …", "We …").

## Consequences

What becomes easier, what becomes harder, which rules follow from it.

## Rejected alternatives

- **Alternative** – why not.

## References

- Code: `crates/…/file.rs`
- Related: ADR-NNNN
