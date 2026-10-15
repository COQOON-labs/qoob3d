use super::*;

#[test]
fn finds_adr_references() {
    assert_eq!(adr_refs("see ADR-0036 and ADR-0012."), ["ADR-0036", "ADR-0012"]);
    assert!(adr_refs("ADR-NNNN, ADR-12").is_empty());
}

#[test]
fn parses_front_matter_and_lists() {
    let meta = front_matter("---\nid: ADR-0001\ncrates: [a, b]\n---\nbody").unwrap();
    assert_eq!(meta["id"], "ADR-0001");
    assert_eq!(list(&meta["crates"]), ["a", "b"]);
    assert!(list("[]").is_empty());
    assert!(front_matter("no front matter").is_none());
}

#[test]
fn validates_dates() {
    assert!(is_date("2026-10-13"));
    assert!(!is_date("2026-1-13"));
    assert!(!is_date("YYYY-MM-DD"));
}

/// A complete, valid ADR record (front matter only) with the given status and
/// supersede links.
fn adr(number: u32, status: &str, supersedes: &str, superseded_by: &str) -> Adr {
    let id = format!("ADR-{number:04}");
    let meta: BTreeMap<String, String> = [
        ("id", id.as_str()),
        ("title", "A sufficiently long decision title"),
        ("status", status),
        ("date", "2026-10-14"),
        ("crates", "[xtask]"),
        ("tags", "[harness]"),
        ("supersedes", supersedes),
        ("superseded_by", superseded_by),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    Adr { number, file: format!("{number:04}-decision.md"), meta }
}

fn validate(adr: &Adr, all: &[&Adr]) -> Vec<String> {
    let by_id: BTreeMap<String, &Adr> = all.iter().map(|a| (a.id(), *a)).collect();
    let mut problems = Vec::new();
    validate_adr(adr, &by_id, &mut problems);
    problems
}

// ADR-0001 (spec section 5): the English statuses are valid.
#[test]
fn english_statuses_are_valid() {
    for status in ["proposed", "accepted", "rejected"] {
        let a = adr(1, status, "[]", "null");
        assert!(validate(&a, &[&a]).is_empty(), "{status}: {:?}", validate(&a, &[&a]));
    }
    let old = adr(1, "superseded", "[]", "ADR-0002");
    let new = adr(2, "accepted", "[ADR-0001]", "null");
    assert!(validate(&old, &[&old, &new]).is_empty(), "{:?}", validate(&old, &[&old, &new]));
    assert!(validate(&new, &[&old, &new]).is_empty(), "{:?}", validate(&new, &[&old, &new]));
}

// ADR-0001 (spec section 5): German statuses are invalid now.
#[test]
fn german_statuses_are_invalid() {
    for status in ["vorgeschlagen", "akzeptiert", "ersetzt", "verworfen"] {
        let a = adr(1, status, "[]", "null");
        let problems = validate(&a, &[&a]);
        let expected = format!("0001-decision.md: status `{status}` is invalid, allowed: ");
        assert!(problems.iter().any(|p| p.contains(&expected)), "{status}: {problems:?}");
    }
}

// ADR-0001 (spec section 5): the supersede logic uses `superseded`: a
// superseded ADR needs `superseded_by`, and `superseded_by` needs status
// `superseded`.
#[test]
fn supersede_logic_uses_english_status() {
    let lone = adr(1, "superseded", "[]", "null");
    assert!(!validate(&lone, &[&lone]).is_empty());
    let old = adr(1, "accepted", "[]", "ADR-0002");
    let new = adr(2, "accepted", "[ADR-0001]", "null");
    assert!(!validate(&old, &[&old, &new]).is_empty());
}

fn temp_root(test: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("qoob3d-docs-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("docs/adr")).unwrap();
    root
}

const FRONT: &str = "---\nid: ADR-0001\ntitle: A sufficiently long decision title\n\
                     status: accepted\ndate: 2026-10-14\ncrates: [xtask]\ntags: [harness]\n\
                     supersedes: []\nsuperseded_by: null\n---\n\n";

// ADR-0001 (spec section 5): `## Context`, `## Decision` and `## Consequences`
// are required; `## Rejected alternatives` and `## References` are optional.
#[test]
fn english_sections_are_required() {
    let root = temp_root("sections");
    let dir = root.join("docs/adr");
    let english =
        format!("{FRONT}# Title\n\n## Context\n\nx\n\n## Decision\n\ny\n\n## Consequences\n\nz\n");
    fs::write(dir.join("0001-decision.md"), &english).unwrap();
    let mut problems = Vec::new();
    let adrs = load_adrs(&dir, &mut problems);
    assert_eq!(adrs.len(), 1);
    assert!(problems.is_empty(), "{problems:?}");

    let german = format!(
        "{FRONT}# Title\n\n## Kontext\n\nx\n\n## Entscheidung\n\ny\n\n## Konsequenzen\n\nz\n"
    );
    fs::write(dir.join("0001-decision.md"), &german).unwrap();
    let mut problems = Vec::new();
    load_adrs(&dir, &mut problems);
    for section in ["## Context", "## Decision", "## Consequences"] {
        let expected = format!("0001-decision.md: section `{section}` is missing");
        assert!(problems.iter().any(|p| p.contains(&expected)), "{section}: {problems:?}");
    }
    assert_eq!(problems.len(), 3, "{problems:?}");
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0001 (spec section 5): the generated index has an English header.
#[test]
fn index_header_is_english() {
    let index = render_index(&[]);
    assert!(index.starts_with("| ADR | Title | Status | Date | Crates |\n"), "{index}");
    let one = render_index(&[adr(1, "accepted", "[]", "null")]);
    assert!(one.contains("| accepted |"), "{one}");
}

// ADR-0001 (spec section 5): an outdated index is reported in English.
#[test]
fn outdated_index_message_is_english() {
    let root = temp_root("index");
    let readme = root.join("docs/adr/README.md");
    fs::write(&readme, "# ADRs\n\n<!-- adr-index:start -->\n<!-- adr-index:end -->\n").unwrap();
    let mut problems = Vec::new();
    check_index(&readme, &[adr(1, "accepted", "[]", "null")], false, &mut problems);
    assert_eq!(
        problems,
        ["docs/adr/README.md: ADR index is outdated \u{2192} `cargo xtask docs --fix`"]
    );
    fs::remove_dir_all(&root).unwrap();
}

/// `ADR-NNNN` built at run time, so that this file contains no literal
/// reference to the fake ADRs below (`cargo xtask docs` scans it against the
/// real ADR set).
fn id(number: u32) -> String {
    format!("ADR-{number:04}")
}

// ADR-0001 (spec section 5): outside `docs/adr/` a reference to a superseded
// or rejected ADR is a problem naming the decision in force; inside
// `docs/adr/` it is fine.
#[test]
fn references_to_superseded_adrs_are_rejected() {
    let root = temp_root("references");
    let (one, two, three) = (id(1), id(2), id(3));
    fs::create_dir_all(root.join("crates/x/src")).unwrap();
    let code = format!("// ok\n// see {one}\n// {two}, {three}\n");
    fs::write(root.join("crates/x/src/lib.rs"), code).unwrap();
    fs::write(root.join("docs/adr/0002-new.md"), format!("Replaces {one}.\n")).unwrap();
    let old = adr(1, "superseded", "[]", &two);
    let new = adr(2, "accepted", &format!("[{one}]"), "null");
    let rejected = adr(3, "rejected", "[]", "null");
    let by_id: BTreeMap<String, &Adr> =
        [&old, &new, &rejected].iter().map(|a| (a.id(), *a)).collect();
    let mut problems = Vec::new();
    check_references(&root, &by_id, &mut problems);
    assert_eq!(problems.len(), 2, "{problems:?}");
    let superseded =
        format!("crates/x/src/lib.rs:2: refers to {one} (status superseded), in force: {two}");
    assert!(problems.iter().any(|p| p.contains(&superseded)), "{problems:?}");
    let rejected = format!("crates/x/src/lib.rs:3: refers to {three} (status rejected)");
    assert!(problems.iter().any(|p| p.contains(&rejected)), "{problems:?}");
    fs::remove_dir_all(&root).unwrap();
}
