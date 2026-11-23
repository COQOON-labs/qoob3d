---
name: harness-audit
description: Audit of the quality harness (ADR-0037) – sabotage self-test, false alarms, run times, gaps. Use when the user wants the harness checked.
---

# Harness audit

Question: does the wall still hold, and is it still the right one? The result is a report with
proposals. Harness files are changed only by a session the human has unlocked.

1. **Self-test:** `cargo xtask selftest`. Every undetected sabotage is a critical
   finding.
2. **Rule coverage:** compare the rule table in `docs/harness.md` with CLAUDE.md and the
   crate CLAUDE.md files. A rule that was actually broken without a check firing is a
   finding; a merely conceivable bypass of a local brake is at most a note.
3. **Real escapes:** look through recent pull requests and CI runs for defects that reached
   `main`. Propose a check or a sabotage case only for what actually happened.
4. **Metrics:**
   - Run time of `cargo xtask check` per step (summary at the end).
   - Lint suppressions against `harness/ratchet.txt` (can the bound go down?).
   - Results of the last CI runs (`gh run list`), especially the weekly `mutants` report and
     `check (Linux, experimental)`.
5. **Fitness for purpose:** which check has never fired since the last audit (dead or
   superfluous?), which one often fires falsely (friction, pressure to bypass)? Does the
   harness still fit the product goal (ADR-0002)?
6. **Report to the user:** status (green/yellow/red per area), metrics, findings,
   proposals with effort and benefit. Implement none of it yourself.
