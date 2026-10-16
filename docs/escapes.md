# Escape log (ADR-0052)

Defects that a check or review should have caught earlier than it was caught. One row per
escape; rows are never deleted, only the status changes. The harness audit (ADR-0042) reads
this log: a class with two escapes gets a deterministic check, or else a checklist item or a
critic lens. Every row also becomes a critic calibration seed.

Status: `open` (class not covered yet), `covered` (a check for the class exists, named in
the row), `accepted` (gap accepted by the user, with the decision).

| Date | Found by | What escaped | Where | Missed by | Class | Check for the class | Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-15 | CI Linux | A double click on the timeline chip failed when egui counted it as a triple click (third click within 0.6 s) | `crates/qoob3d-app/src/panels.rs` | local check (macOS timing), critic | UI tests that depend on egui click counting | `navigation::double_clicked_by`; proposed: a source rule against `Response::double_clicked` in app code | open |
| 2026-10-15 | user | Git hooks failed under Sourcetree: Gatekeeper refused the freshly linked, quarantined `target/debug/xtask` | `.githooks/` | every check (no GUI client in tests or CI) | hooks in macOS GUI git clients | PR #4: hooks run the built binary, pre-push stops under a quarantining client; stub tests for `lib.sh` | covered |
| 2026-10-16 | orchestrator | The guard denied harmless commands: a heredoc with PR text (reported as a fixture) and `gh pr create` with "xtask" in the title | `xtask/src/guard.rs` | – (false positive) | textual guard on data text | the OS sandbox replaces the textual guard (open item in CLAUDE.md) | accepted (ADR-0049, user decision B) |
