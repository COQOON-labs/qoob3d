# Critic checklist (ADR-0041)

The critic checks a diff against these items. Every finding names **one** ID, a location
(`file:line`), the concrete consequence ("what breaks / what becomes more expensive") and the
smallest fix. Anything that is merely taste and not in the style register
(`docs/design/code-style.md`) is **not** a finding. At most it may appear as a proposal for a
new register entry (K-12, `minor`).

Severities:

- `blocker`: wrong behavior, data loss, security hole or a violation of a rule with real
  effect.
- `major`: clearly worse maintainability or a principle violation with nameable damage.
- `minor`: an improvement that blocks nobody.

| ID | Check item |
| --- | --- |
| K-01 | **Minimality:** Every line is needed. No duplicates of what exists (search first!), no speculative generality, no unused parameters, options or configuration. |
| K-02 | **Single responsibility:** Every function, type and module has exactly one reason to change. No mixing of UI, logic and IO. The name describes everything it does. |
| K-03 | **Open-closed in the Rust sense:** The compiler forces a new variant (feature, curve, command kind) into every relevant place: exhaustive `match` without `_ =>` over these enums. Extend via the intended points (`Request`/`Response`, `FeatureKind`) instead of special cases. |
| K-04 | **Abstraction only when needed (YAGNI beats OCP/DIP):** A trait, generic or indirection needs at least two real implementations or a crate boundary that enforces it (ST-07). |
| K-05 | **Liskov:** Every implementation of a trait keeps the documented contract (pre- and postconditions, errors, panics). If there are several implementations, shared contract tests run against all of them. |
| K-06 | **Interface segregation:** Interfaces only demand what they need. No large structs as parameters when two fields suffice, no traits with methods that implementers cannot fill meaningfully. |
| K-07 | **Dependency inversion and crate boundaries:** Dependencies run in the direction from ADR-0036. Core logic does not depend on IO, UI or OCCT. Testable logic is separated from side effects. |
| K-08 | **Clean code (readability):** Meaningful names, with the unit if not mm (ST-13). No magic numbers without a constant or reason. Comments explain the why. No commented-out code. |
| K-09 | **Error handling:** No swallowed errors (`let _ =`, `.ok()` without reason). OCCT calls go through `guarded`. Errors carry context. No panics on user input or file contents. |
| K-10 | **Security:** Files (`.qbd`, later import) are untrusted. Required are limits on sizes, recursion depth and numbers (NaN, ∞, negative), no panics and no unbounded memory driven by input. Writes are atomic, no shell with user data. New dependencies are justified. |
| K-11 | **Dead code:** Unused functions, variants, fields, parameters and translation keys, tests for removed code, leftovers of old approaches. |
| K-12 | **Consistency and style register:** The diff follows `docs/design/code-style.md`. No rework of unaffected code to personal taste. A new taste decision is proposed, not introduced silently. |
| K-13 | **Test quality:** Tests check behavior according to the specification, not implementation details. Expected values are analytical or come from the requirement, not from a run of the implementation. Tolerances are justified, edge cases covered, tests deterministic. No weakened assertion. |
| K-14 | **Accessibility (UI diff):** Every interactive element has an accessible name (AccessKit), is reachable by keyboard, the target size is ≥ 24 px, color is never the only carrier of information, contrasts as per the theme. |
| K-15 | **Usability (UI diff):** System status is visible. Escape and undo work everywhere. The controls match `docs/design/controls.md`. Errors are prevented, messages are understandable and translated. The shortest path "design → slicer → print" does not get longer. |
| K-16 | **Decisions and docs:** A new architecture or design decision gets an ADR. CLAUDE.md, `docs/architecture.md` and the requirements are updated. |
| K-17 | **Performance:** No unnecessary allocations or copies in the frame loop, no kernel work in the UI thread, no quadratic effort over meshes or features without reason. |

## Output format

One JSON object, no prose around it:

```json
{
  "findings": [
    {
      "rule": "K-09",
      "severity": "major",
      "file": "crates/qoob3d-core/src/document/mod.rs",
      "line": 120,
      "summary": "Load error is swallowed",
      "consequence": "A damaged file opens empty, the user saves over it",
      "fix": "Pass the error to the caller (`?`) and show it in the dialog"
    }
  ]
}
```
