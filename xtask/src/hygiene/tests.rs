use std::process::Command;

use super::*;

const REL: &str = "crates/qoob3d-core/src/expr.rs";

// ADR-0039: `#[cfg(test)]` followed by `mod tests;` is fine, also indented
// and with blank lines in between.
#[test]
fn accepts_out_of_line_test_module() {
    for text in [
        "fn a() {}\n\n#[cfg(test)]\nmod tests;\n",
        "#[cfg(test)]\n\n\nmod tests;",
        "mod inner {\n    #[cfg(test)]\n    mod tests;\n}\n",
        "fn a() {}\n",
        "",
        "#[cfg(not(test))]\nfn real() {}\n",
        "// #[cfg(test)]\nfn a() {}\n",
    ] {
        assert!(check_source(REL, text).is_empty(), "{text:?}");
    }
}

// ADR-0039: an inline test module is rejected with `rel:<line of the cfg>`;
// since addendum F4 its `#[test]` line (5) is a problem of its own, too
// ("every line whose trimmed start is `#[test]` … is a problem").
#[test]
fn rejects_inline_test_module() {
    let text = "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n";
    let problems = check_source(REL, text);
    assert_eq!(problems.len(), 2, "{problems:?}");
    for line in [3, 5] {
        let expected = format!(
            "crates/qoob3d-core/src/expr.rs:{line}: test code belongs in its own file: \
             `#[cfg(test)] mod tests;` + tests.rs (ADR-0039)"
        );
        assert!(problems.iter().any(|p| p.contains(&expected)), "{line}: {problems:?}");
    }
}

/// The check_source message for one line (spec section 4.3).
fn test_code_message(line: usize) -> String {
    format!(
        "{REL}:{line}: test code belongs in its own file: `#[cfg(test)] mod tests;` + tests.rs \
         (ADR-0039)"
    )
}

// ADR-0039 (addendum F4): a bare test attribute in production code is a
// problem even without any `#[cfg(test)]`: `#[test]`, `#[tokio::test…`,
// `#[should_panic…`, indented or not, with `rel:<line of the attribute>`.
#[test]
fn f4_bare_test_attributes_are_flagged() {
    for (text, line) in [
        ("fn a() {}\n#[test]\nfn t() {}\n", 2),
        ("mod inline {\n    #[test]\n    fn t() {}\n}\n", 2),
        ("#[test] fn t() {}\n", 1),
        ("fn a() {}\n\t#[test]\n\tfn t() {}\n", 2),
        ("#[tokio::test]\nasync fn t() {}\n", 1),
        ("fn a() {}\n\n  #[tokio::test(flavor = \"multi_thread\")]\n  async fn t() {}\n", 3),
        ("#[should_panic]\nfn t() {}\n", 1),
        ("fn a() {}\n#[should_panic(expected = \"boom\")]\nfn t() {}\n", 2),
        // An out-of-line test module does not excuse a stray test elsewhere.
        ("#[cfg(test)]\nmod tests;\n\n#[test]\nfn t() {}\n", 4),
    ] {
        let problems = check_source(REL, text);
        assert_eq!(problems, vec![test_code_message(line)], "{text:?}");
    }
}

// ADR-0039 (addendum F4): each attribute line is its own problem.
#[test]
fn f4_every_test_attribute_line_is_reported() {
    let text =
        "#[test]\n#[should_panic]\nfn a() {}\n\nfn b() {}\n#[tokio::test]\nasync fn c() {}\n";
    let problems = check_source(REL, text);
    assert_eq!(
        problems,
        vec![test_code_message(1), test_code_message(2), test_code_message(6)],
        "{problems:?}"
    );
}

// ADR-0039 (addendum F4): only lines whose trimmed start is one of the three
// attributes count; comments, strings and similar-looking attributes pass.
#[test]
fn f4_lookalikes_pass() {
    for text in [
        "// #[test]\nfn a() {}\n",
        "/// #[should_panic]\nfn a() {}\n",
        "let s = \"#[test]\";\n",
        "#[test_case(1)]\nfn a() {}\n",
        "#[testing]\nfn a() {}\n",
        "#[tokio::main]\nasync fn main() {}\n",
        "#[cfg(test)]\nmod tests;\n",
    ] {
        assert!(check_source(REL, text).is_empty(), "{text:?}");
    }
}

// ADR-0039 (addendum F4): `check_source` itself flags test attributes in any
// text; it is the caller (`check`) that only passes Code files, and a
// `tests.rs` or `tests/` file is not Code.
#[test]
fn f4_check_source_flags_regardless_of_path_and_caller_filters() {
    let text = "#[test]\nfn t() {}\n";
    for rel in ["crates/qoob3d-core/src/expr/tests.rs", "crates/qoob3d-core/tests/expr.rs"] {
        assert_eq!(check_source(rel, text).len(), 1, "{rel}");
        assert_eq!(paths::kind(rel), Kind::Test, "{rel}");
    }
    assert_eq!(paths::kind(REL), Kind::Code);
}

// ADR-0039: anything other than exactly `mod tests;` after the cfg line is a
// problem, including the end of the file.
#[test]
fn rejects_other_test_only_items() {
    for text in [
        "#[cfg(test)]\nfn helper() {}\n",
        "#[cfg(test)]\nmod test_utils;\n",
        "#[cfg(test)]\npub(crate) mod tests;\n",
        "fn a() {}\n#[cfg(test)]\n",
        "fn a() {}\n#[cfg(test)]\n\n   \n",
    ] {
        assert_eq!(check_source(REL, text).len(), 1, "{text:?}");
    }
}

// ADR-0039 (extended): a line starting with `#[cfg(test)]` is fine only if the
// rest of that line is exactly `mod tests;`; any other rest is one problem
// with `rel:<line>`, regardless of what follows on the next line.
#[test]
fn single_line_cfg_test_must_declare_mod_tests() {
    for text in [
        "#[cfg(test)] mod tests;\n",
        "fn a() {}\n    #[cfg(test)]   mod tests;  \n",
        "#[cfg(test)]mod tests;",
    ] {
        assert!(check_source(REL, text).is_empty(), "{text:?}");
    }
    for (text, line) in [
        ("fn a() {}\n#[cfg(test)] mod tests { #[test] fn t() {} }\n", 2),
        ("#[cfg(test)] fn helper() {}\n", 1),
        ("fn a() {}\n\n    #[cfg(test)] fn helper() {}\nmod tests;\n", 3),
        ("#[cfg(test)] pub(crate) mod tests;\n", 1),
        ("#[cfg(test)] mod tests; fn extra() {}\n", 1),
    ] {
        let problems = check_source(REL, text);
        assert_eq!(problems.len(), 1, "{text:?}: {problems:?}");
        assert!(problems[0].contains(&format!("{REL}:{line}")), "{text:?}: {problems:?}");
    }
}

// ADR-0039: one problem per offending cfg line, each with its own line number.
#[test]
fn reports_every_offending_line() {
    let text =
        "#[cfg(test)]\nfn a() {}\n#[cfg(test)]\nmod tests;\n    #[cfg(test)]\n    use std::fs;\n";
    let problems = check_source(REL, text);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains(&format!("{REL}:1"))), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains(&format!("{REL}:5"))), "{problems:?}");
}

// ADR-0040: a crate manifest must inherit the workspace lints.
#[test]
fn manifest_inherits_workspace_lints() {
    let ok = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n\n[dependencies]\nserde_json = \"1\"\n";
    assert!(check_manifest("crates/x/Cargo.toml", ok).is_empty());
    let last = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true";
    assert!(check_manifest("crates/x/Cargo.toml", last).is_empty());
}

// ADR-0040: missing `[lints]`, `workspace = false`, `workspace = true` in
// another section, or only `[workspace.lints]` → exactly one problem.
#[test]
fn manifest_without_inherited_lints() {
    for text in [
        "[package]\nname = \"x\"\n",
        "[package]\nname = \"x\"\n\n[lints]\nworkspace = false\n",
        "[lints]\n\n[dependencies]\nserde = { workspace = true }\n",
        "[package]\nversion.workspace = true\n",
        "[workspace.lints]\nworkspace = true\n",
        "[workspace.lints.rust]\nunsafe_code = \"deny\"\n",
        "",
    ] {
        assert_eq!(check_manifest("crates/x/Cargo.toml", text).len(), 1, "{text:?}");
    }
}

// ADR-0040: suppressions are `#[expect(` / `#![expect(` at the start of a line
// (after whitespace); `allow`, comments and mid-line mentions do not count.
#[test]
fn counts_expect_attributes() {
    // Built with concat! so that no line of this file starts with the attribute
    // (the real repository ratchet counts this file, too).
    let text = concat!(
        "#[expect(clippy::too_many_lines, reason = \"x\")]\nfn a() {}\n",
        "    #![expect(dead_code, reason = \"y\")]\n\t#[expect(unused)]\n",
    );
    assert_eq!(count_suppressions(text), 3);
    for text in [
        "",
        "#[allow(dead_code)]\n",
        "#![allow(dead_code)]\n",
        "// #[expect(dead_code)]\n",
        "    // #![expect(dead_code)]\n",
        "let s = \"#[expect(\";\n",
        "fn a() {} #[expect(dead_code)]\n",
    ] {
        assert_eq!(count_suppressions(text), 0, "{text:?}");
    }
}

// ADR-0040 (spec section 4.3): exceeding the ratchet is one problem with the
// count, the limit and the file.
#[test]
fn ratchet_message_is_english() {
    let root = std::env::temp_dir().join(format!("qoob3d-hygiene-ratchet-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("harness")).unwrap();
    fs::write(root.join("harness/ratchet.txt"), "suppressions = 2\n").unwrap();
    let problem = check_ratchet(&root, 3).expect("3 > 2 must be a problem");
    assert!(
        problem.contains(
            "3 lint suppressions (#[expect]), 2 allowed (harness/ratchet.txt). \
             Fix the cause instead of suppressing it (ADR-0040)."
        ),
        "{problem}"
    );
    assert_eq!(check_ratchet(&root, 2), None);
    fs::remove_dir_all(&root).unwrap();
}

const O_UMLAUT: char = '\u{f6}';

/// The full message for a forbidden character (spec section 4.1).
fn language_message(rel: &str, line: usize, c: char) -> String {
    format!(
        "{rel}:{line}: `{c}` is not English \u{2013} the repository is English, German only in \
         locales/de/ (ADR-0045, harness/language-allowlist.txt)"
    )
}

// ADR-0045 (spec section 4.1): `path:` and `word:` entries, trimmed; comments
// and blank lines are ignored.
#[test]
fn parses_language_allowlist() {
    let text = "# The repository is English.\n\n\
                path: crates/qoob3d-app/locales/de/\n\
                # A single file.\n\
                path:   harness/sabotage/hygiene-german-text.case  \n\
                \n\
                word: M\u{f6}ller\n\
                word:  Gr\u{f6}ner \n";
    let expected = Allowlist {
        paths: vec![
            "crates/qoob3d-app/locales/de/".to_owned(),
            "harness/sabotage/hygiene-german-text.case".to_owned(),
        ],
        words: vec!["M\u{f6}ller".to_owned(), "Gr\u{f6}ner".to_owned()],
    };
    assert_eq!(parse_allowlist(text), expected);
    assert_eq!(parse_allowlist(""), Allowlist::default());
    assert_eq!(parse_allowlist("# only comments\n\n"), Allowlist::default());
}

// ADR-0045 (spec section 4.1): every forbidden character is flagged with
// `path:line` and the character.
#[test]
fn german_characters_are_flagged() {
    let none = Allowlist::default();
    for c in ['\u{e4}', '\u{f6}', '\u{fc}', '\u{c4}', '\u{d6}', '\u{dc}', '\u{df}', '\u{1e9e}'] {
        let text = format!("first line\nword with {c} inside\n");
        let problems = check_language("docs/a.md", &text, &none);
        assert_eq!(problems, [language_message("docs/a.md", 2, c)], "{c}");
    }
}

// ADR-0045 (spec section 4.1): one problem per offending line, naming the
// first forbidden character of that line.
#[test]
fn one_problem_per_line_with_first_character() {
    let text = "Gr\u{fc}\u{df}e und T\u{e4}ler\nclean\n\u{c4}rger\n";
    let problems = check_language("src/x.rs", text, &Allowlist::default());
    assert_eq!(
        problems,
        [language_message("src/x.rs", 1, '\u{fc}'), language_message("src/x.rs", 3, '\u{c4}')]
    );
}

// ADR-0045 (spec section 4.1): ASCII and other non-ASCII characters
// (typography, math, units, accents) are fine.
#[test]
fn other_characters_pass() {
    let text = "a \u{2013} b, \u{d8}20, \u{3c0}\u{b7}r\u{b2}, mm\u{b3}, 40\u{d7}25, caf\u{e9}, \u{2192}, \u{2714}, \u{2026}\n\
                plain ASCII line\n";
    assert!(check_language("docs/a.md", text, &Allowlist::default()).is_empty());
    assert!(check_language("docs/a.md", "", &Allowlist::default()).is_empty());
}

// ADR-0045 (spec section 4.1): a path entry ending in `/` allowlists every
// file below it, any other entry exactly that file.
#[test]
fn allowlisted_paths_pass() {
    let allow = Allowlist {
        paths: vec![
            "crates/qoob3d-app/locales/de/".to_owned(),
            "harness/sabotage/hygiene-german-text.case".to_owned(),
        ],
        words: vec![],
    };
    let text = "Sch\u{f6}n\n";
    for rel in [
        "crates/qoob3d-app/locales/de/main.ftl",
        "crates/qoob3d-app/locales/de/sub/extra.ftl",
        "harness/sabotage/hygiene-german-text.case",
    ] {
        assert!(check_language(rel, text, &allow).is_empty(), "{rel}");
    }
    for rel in [
        "crates/qoob3d-app/locales/en/main.ftl",
        "crates/qoob3d-app/locales/de-old/main.ftl",
        "harness/sabotage/hygiene-german-text.case.bak",
        "harness/sabotage/other.case",
        "docs/crates/qoob3d-app/locales/de/main.ftl",
    ] {
        assert_eq!(
            check_language(rel, text, &allow),
            [language_message(rel, 1, O_UMLAUT)],
            "{rel}"
        );
    }
}

// ADR-0045 (spec section 4.1): allowlisted words are removed from the line
// before the check; other forbidden characters on the same line still count.
#[test]
fn allowlisted_words_pass() {
    let allow = Allowlist {
        paths: vec![],
        words: vec!["Gr\u{f6}ner".to_owned(), "M\u{f6}ller".to_owned()],
    };
    for text in [
        "Benjamin Gr\u{f6}ner <bgroener@coqoon.com>\n",
        "M\u{f6}ller and Gr\u{f6}ner\n",
        "Gr\u{f6}nerGr\u{f6}ner\n",
    ] {
        assert!(check_language("harness/identity.txt", text, &allow).is_empty(), "{text:?}");
    }
    let text = "Gr\u{f6}ner says gr\u{fc}n\n";
    assert_eq!(
        check_language("docs/a.md", text, &allow),
        [language_message("docs/a.md", 1, '\u{fc}')]
    );
    // The word is case-sensitive as written.
    assert_eq!(
        check_language("docs/a.md", "GR\u{d6}NER\n", &allow),
        [language_message("docs/a.md", 1, '\u{d6}')]
    );
}

/// A CODEOWNERS text with an owner line for every required harness entry.
fn complete_codeowners() -> String {
    let mut text = String::from("# Code owners\n* @owner\n\n");
    for entry in paths::HARNESS.iter().filter(|e| !e.starts_with("target/")) {
        text.push_str(&format!("/{entry} @owner\n"));
    }
    text
}

fn codeowners_message(entry: &str) -> String {
    format!(".github/CODEOWNERS has no owner line for harness path `{entry}` (ADR-0044)")
}

// ADR-0044 (spec section 4.2): a file with an owner line for every harness
// entry passes; `target/` entries are not required.
#[test]
fn complete_codeowners_passes() {
    assert!(check_codeowners(&complete_codeowners()).is_empty());
    let literal = "* @Ben-Gee\n/.claude/ @Ben-Gee\n/.githooks/ @Ben-Gee\n/.github/ @Ben-Gee\n\
                   /.cargo/ @Ben-Gee\n/xtask/ @Ben-Gee @other\n/harness/ @Ben-Gee\n\
                   /Cargo.toml @Ben-Gee\n/clippy.toml @Ben-Gee\n/deny.toml @Ben-Gee\n\
                   /rustfmt.toml @Ben-Gee\n/rust-toolchain.toml @Ben-Gee\n";
    assert!(check_codeowners(literal).is_empty(), "{:?}", check_codeowners(literal));
    assert!(paths::HARNESS.contains(&"target/debug/xtask"));
}

// ADR-0044 (spec section 4.2): a missing line, a commented-out line, a line
// without owner or a different first token do not cover the entry.
#[test]
fn incomplete_codeowners_is_flagged() {
    let complete = complete_codeowners();
    for replacement in [
        "",
        "# /xtask/ @owner\n",
        "/xtask/\n",
        "/xtask/   \n",
        "xtask/ @owner\n",
        "/xtask/src/ @owner\n",
        "/xtask @owner\n",
    ] {
        let text = complete.replace("/xtask/ @owner\n", replacement);
        assert_eq!(check_codeowners(&text), [codeowners_message("xtask/")], "{replacement:?}");
    }
    let required = paths::HARNESS.iter().filter(|e| !e.starts_with("target/")).count();
    assert_eq!(check_codeowners("").len(), required);
    assert_eq!(check_codeowners("* @owner\n").len(), required);
}

/// Runs git in `root` without global/system config.
fn git_in(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn put(root: &Path, rel: &str, content: &[u8]) {
    let path: PathBuf = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

// ADR-0045 (spec section 4.1): `check` scans tracked and untracked files that
// are not ignored, skips files that are not UTF-8 and honours the allowlist.
#[test]
fn check_scans_versioned_text_files() {
    let root = std::env::temp_dir().join(format!("qoob3d-hygiene-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git_in(&root, &["init", "-q"]);
    put(&root, "harness/ratchet.txt", b"suppressions = 0\n");
    put(&root, "harness/language-allowlist.txt", b"path: crates/qoob3d-app/locales/de/\n");
    put(&root, ".github/CODEOWNERS", complete_codeowners().as_bytes());
    put(&root, ".gitignore", b"ignored/\n");
    put(&root, "docs/tracked.md", "Gr\u{fc}n\n".as_bytes());
    put(&root, "docs/untracked.md", "fine\nsch\u{f6}n\n".as_bytes());
    put(&root, "ignored/x.md", "\u{e4}\n".as_bytes());
    put(&root, "crates/qoob3d-app/locales/de/main.ftl", "nav-grid = \u{dc}bersicht\n".as_bytes());
    // Latin-1 0xE4 (a with diaeresis) is not valid UTF-8.
    put(&root, "docs/latin1.txt", &[b'a', 0xe4, b'\n']);
    git_in(&root, &["add", "docs/tracked.md", ".gitignore"]);

    let problems = check(&root).unwrap_err();
    let has = |needle: &str| problems.iter().any(|p| p.contains(needle));
    assert!(has(&language_message("docs/tracked.md", 1, '\u{fc}')), "{problems:?}");
    assert!(has(&language_message("docs/untracked.md", 2, O_UMLAUT)), "{problems:?}");
    for absent in ["ignored/x.md", "latin1", "locales/de/main.ftl", "CODEOWNERS"] {
        assert!(!has(absent), "{absent}: {problems:?}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0044 (spec section 4.2): `check` reports a missing CODEOWNERS file.
#[test]
fn check_reports_missing_codeowners() {
    let root =
        std::env::temp_dir().join(format!("qoob3d-hygiene-codeowners-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git_in(&root, &["init", "-q"]);
    put(&root, "harness/ratchet.txt", b"suppressions = 0\n");
    put(&root, "harness/language-allowlist.txt", b"");
    let problems = check(&root).unwrap_err();
    assert!(
        problems.iter().any(|p| p.contains(".github/CODEOWNERS is missing (ADR-0044)")),
        "{problems:?}"
    );
    fs::remove_dir_all(&root).unwrap();
}
