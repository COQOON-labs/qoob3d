# Style register

Binding taste decisions for code (ADR-0040). The register ensures that all
agents and humans write the same style and nobody reworks code according to personal taste.

**Rules for the register**

1. Only what is written here applies. Anything not regulated follows the surrounding code.
2. New entries are **proposed by an agent, the human decides**. Only then do they apply.
3. Convert existing code after a new entry only in a separate commit, never in passing.
4. Where possible, a tool checks the entry (column "Check"). The others are checked by the
   critic (K-12).

| ID | Decision | Check |
| --- | --- | --- |
| ST-01 | Formatting is done by `rustfmt` (`max_width = 100`, `use_small_heuristics = "Max"`). No hand formatting that `rustfmt` changes again. | `cargo fmt --check` |
| ST-02 | Everything in the repository in English: code, comments, docs, commit messages, messages from `xtask` (ADR-0045). German only in `crates/qoob3d-app/locales/de/` and in tests that assert German translations. Visible UI texts only via `tr("key")` (ADR-0020). | `xtask hygiene`, `xtask i18n`, critic |
| ST-03 | Visibility as narrow as possible. `pub` only for the crate's API, otherwise `pub(crate)` or private. That way `dead_code` detects unused parts. | `unreachable_pub` |
| ST-04 | Error types with `thiserror` in libraries, `anyhow` only in `qoob3d-app`. Propagate errors with `?`, adding context where it is missing. | critic (K-09) |
| ST-05 | No `unwrap()` in production code. `expect("why this holds here")` only for real invariants, otherwise `?` or handling. `unwrap()` is allowed in tests. | `clippy::unwrap_used` |
| ST-06 | Lint exceptions only as `#[expect(lint, reason = "…")]`, never `#[allow]`. Their number is capped. | `clippy::allow_attributes`, `xtask hygiene` |
| ST-07 | Variants as `enum` with exhaustive `match`, no speculative traits. A trait only with the second real implementation or when a crate boundary requires it ("rule of two", YAGNI before OCP). | critic (K-03, K-04) |
| ST-08 | Functions at most 100 lines and cognitive complexity at most 25. Older exceptions are marked and get split the next time they are touched. | `too_many_lines`, `cognitive_complexity` |
| ST-09 | Unit tests in a separate file: `#[cfg(test)] mod tests;` and `<module>/tests.rs`. Integration tests in `tests/`. Expectations analytic or from the requirement. | `xtask hygiene`, critic (K-13) |
| ST-10 | Every file starts with `//!` docs: what it is for, with an ADR reference for decisions. Comments explain the why, not the what. | critic (K-08) |
| ST-11 | Early return with `let … else`. No negated condition with an `else` branch. `if let … else` instead of `match` with one arm and `_`. | `if_not_else`, `single_match_else` |
| ST-12 | `use` stands at the beginning of its scope, not between statements. | `items_after_statements` |
| ST-13 | Names in English and descriptive. Lengths are mm. Other units are in the name (`_px`, `_deg`, `_rad`, `_s`). | critic (K-08) |
| ST-14 | Method references instead of trivial closures (`.map(String::as_str)`), `map_or` instead of `map(…).unwrap_or(…)`, statements end with `;`. | `redundant_closure_for_method_calls`, `map_unwrap_or`, `semicolon_if_nothing_returned` |
