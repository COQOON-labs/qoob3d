---
name: harness-audit
description: Regular audit of the quality harness (ADR-0042) – sabotage self-test, rule coverage, false alarms, critic accuracy, run times, gaps. Use when the user wants the harness checked, and at least once per milestone.
---

# Harness audit (ADR-0042)

Question: does the wall still hold, and is it still the right one? The result is a report with
proposals. Harness files are changed only by a session the human has unlocked.

1. **Self-test:** `cargo xtask selftest`. Every undetected sabotage is a critical
   finding.
2. **Rule coverage:** `cargo xtask rules`. Rules checked only by the critic or by nobody
   are candidates for a deterministic check. List rules from CLAUDE.md, the crate CLAUDE.md
   files and the ADRs that are missing from the register.
3. **Search for gaps:** for every rule ask: how could an agent violate it without any check
   firing? Every gap found becomes a new case in `harness/sabotage/` (proposal with finished
   content) and, if possible, a new check.
4. **Metrics:**
   - Run time of `cargo xtask check` per step (summary at the end).
   - Lint suppressions against `harness/ratchet.txt` (can the bound go down?).
   - `git log --grep "Mixed-Change:"`: how often and why was the test separation bypassed with
     a reason?
   - `target/critic/*.json`: share of `refuted` among `blocker`/`major` (false alarms of the
     critic), most frequent rules, how many `accepted`; per model and lens, which findings
     only that review found (ADR-0052).
   - `docs/escapes.md` (ADR-0052): escapes per class and per layer that missed them. A class
     with two escapes needs a deterministic check, or else a checklist item or a lens; propose
     it with finished content. Rows with status `open` older than one milestone are a finding.
   - Results of the last CI runs (`gh run list`), especially `mutants` and
     `check (Linux, experimental)`.
5. **Fitness for purpose:** which check has never fired since the last audit (dead or
   superfluous?), which one often fires falsely (friction, pressure to bypass)? Does the
   harness still fit the product goal (ADR-0002)?
6. **Report to the user:** status (green/yellow/red per area), metrics, findings,
   proposals with effort and benefit. Implement none of it yourself.
