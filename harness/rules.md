# Rule register (ADR-0040)

Every quality rule of the project with the checks that enforce it. `cargo xtask rules`
checks that every rule has at least one check and that every named check exists.
If a check disappears, the check turns red instead of the rule silently becoming a wish.

Kinds: `xtask:` step of `cargo xtask` · `lint:` rustc/clippy lint in `Cargo.toml` ·
`ci:` job in `.github/workflows/` · `hook:` Claude Code hook · `githook:` Git hook ·
`agent:`/`skill:` Claude Code role or workflow · `critic:` item of the critic checklist
(`harness/critic-checklist.md`) · `test:` test function · `manual`.

Deterministic are `xtask`, `lint`, `ci`, `githook`, `hook` and `test`. `critic` is a
second line of defense: it only blocks through confirmed findings (ADR-0041).

| ID | Rule | Checks |
| --- | --- | --- |
| R-01 | Done means: `cargo xtask check` is green. | `hook:check-on-stop`, `githook:pre-push`, `ci:check-macos` |
| R-02 | As much code as necessary, as little as possible. | `lint:too_many_lines`, `lint:cognitive_complexity`, `ci:policy`, `critic:K-01` |
| R-03 | Clean code: readable, well named, commented on the why. | `lint:all`, `xtask:fmt`, `critic:K-08` |
| R-04 | Single responsibility. | `lint:too_many_lines`, `lint:cognitive_complexity`, `critic:K-02` |
| R-05 | Open-closed in the Rust sense (exhaustive enums, intended extension points). | `critic:K-03` |
| R-06 | Liskov: implementations keep the contract of their trait. | `critic:K-05` |
| R-07 | Interface segregation. | `critic:K-06` |
| R-08 | Dependency inversion and crate boundaries (ADR-0036). | `xtask:arch`, `critic:K-07` |
| R-09 | Abstraction only when needed, YAGNI before OCP (ST-07). | `critic:K-04` |
| R-10 | Security: no `unsafe`, no panics on input, no secrets, vetted supply chain. | `lint:unsafe_code`, `lint:unwrap_used`, `xtask:deny`, `ci:policy`, `critic:K-10` |
| R-11 | Accessibility of the UI. | `critic:K-14`, `skill:usability-review` |
| R-12 | Usability, measured by the path "design → slicer → print". | `critic:K-15`, `skill:usability-review` |
| R-13 | Dead code is detected and removed. | `lint:unreachable_pub`, `xtask:i18n`, `ci:policy`, `critic:K-11` |
| R-14 | Tests are written in a different agent context than the code they check. | `hook:guard`, `xtask:commits`, `xtask:hygiene`, `agent:test-author`, `agent:implementer` |
| R-15 | Tests really check something (mutations are caught). | `ci:mutants`, `critic:K-13` |
| R-16 | AI as critic: every pushed state with production code, harness or build-manifest changes is reviewed per the review plan (tier, lenses, model rotation, stopping rule), confirmed severe findings are fixed; the escape log is append-only. | `xtask:review`, `githook:pre-push`, `agent:critic`, `agent:verifier`, `skill:critic-review` |
| R-17 | No commit names an AI as author, co-author or generator. | `githook:commit-msg`, `githook:pre-push`, `xtask:commits`, `ci:policy` |
| R-18 | Decisions are recorded in ADRs, references point to valid ADRs. | `xtask:docs`, `critic:K-16` |
| R-19 | A bugfix starts with a failing test. | `agent:test-author`, `skill:deliver`, `critic:K-13` |
| R-20 | Lint suppressions only with a reason, `#[ignore]` only with a reason, their number is capped. | `lint:allow_attributes`, `lint:allow_attributes_without_reason`, `lint:ignore_without_reason`, `xtask:hygiene` |
| R-21 | Golden files, snapshots and expectation files only after asking the user. | `manual`, `critic:K-13` |
| R-22 | Format fixtures are never changed (ADR-0030). | `xtask:commits`, `hook:guard` |
| R-23 | No hard-coded UI texts, translations complete (ADR-0020). | `xtask:i18n` |
| R-24 | Taste is consistent: the style register applies, no rework to personal taste. | `xtask:fmt`, `lint:semicolon_if_nothing_returned`, `critic:K-12` |
| R-25 | The harness only changes with approval by the human. | `hook:guard`, `ci:policy`, `xtask:hygiene`, `manual` |
| R-26 | Gates are not bypassed (`--no-verify`, disabling hooks, admin merge). | `hook:guard`, `ci:policy` |
| R-27 | No leftovers in the code (`dbg!`, `todo!`, `unimplemented!`). | `lint:dbg_macro`, `lint:todo`, `lint:unimplemented` |
| R-28 | The harness is regularly checked against sabotage and audited. | `xtask:selftest`, `ci:selftest`, `skill:harness-audit`, `agent:harness-auditor` |
| R-29 | Feature coverage of new feature kinds (CLAUDE.md rule 9). | `xtask:models` |
| R-30 | Every commit is authored and committed by the project identity (`harness/identity.txt`, ADR-0044). | `xtask:commits`, `githook:commit-msg`, `githook:pre-push`, `ci:policy` |
| R-31 | The repository is English; German only in `locales/de/` and tests of German translations (ADR-0045). | `xtask:hygiene`, `xtask:docs` |
| R-32 | Agents cannot unlock the harness themselves; the unlock lifts only the harness-file protection (ADR-0043). | `hook:guard`, `xtask:selftest` |
