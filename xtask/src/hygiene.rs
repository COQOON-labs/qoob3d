//! Source rules that neither rustc nor clippy can express:
//! - unit tests live in their own `tests.rs` file, so that test code and
//!   production code are separate files (ADR-0039),
//! - every crate inherits the workspace lints (ADR-0040),
//! - the number of suppressed lints (each lint in an `#[expect(…)]`, also
//!   inside `cfg_attr`) never grows beyond `harness/ratchet.txt` (ADR-0040),
//! - the repository is English: no German umlauts or sharp s outside
//!   `harness/language-allowlist.txt` (ADR-0045),
//! - `.github/CODEOWNERS` names an owner for every harness path (ADR-0044).

use std::fs;
use std::path::{Path, PathBuf};

use crate::git;
use crate::paths::{self, Kind};

pub(crate) const LANGUAGE_ALLOWLIST: &str = "harness/language-allowlist.txt";
const CODEOWNERS: &str = ".github/CODEOWNERS";
/// Attributes that only test code carries.
const TEST_ATTRIBUTES: &[&str] = &["#[test]", "#[tokio::test", "#[should_panic"];
/// German letters that English text never contains.
const GERMAN_CHARS: &[char] =
    &['\u{e4}', '\u{f6}', '\u{fc}', '\u{c4}', '\u{d6}', '\u{dc}', '\u{df}', '\u{1e9e}'];

/// Exceptions to the language rule.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Allowlist {
    /// Files (exact) or directories (ending in `/`), repository-relative.
    pub(crate) paths: Vec<String>,
    /// Words such as proper names.
    pub(crate) words: Vec<String>,
}

pub(crate) fn check(root: &Path) -> Result<(), Vec<String>> {
    let mut problems = Vec::new();
    let mut files = Vec::new();
    for dir in ["crates", "xtask"] {
        collect(&root.join(dir), &mut files);
    }
    let mut suppressions = 0;
    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        let Ok(text) = fs::read_to_string(&file) else { continue };
        if rel.ends_with("Cargo.toml") {
            problems.extend(check_manifest(&rel, &text));
        } else if rel.ends_with(".rs") {
            suppressions += count_suppressions(&text);
            if paths::kind(&rel) == Kind::Code {
                problems.extend(check_source(&rel, &text));
            }
        }
    }
    problems.extend(check_ratchet(root, suppressions));
    problems.extend(check_versioned_files(root));
    match fs::read_to_string(root.join(CODEOWNERS)) {
        Ok(text) => problems.extend(check_codeowners(&text)),
        Err(_) => problems.push(format!("{CODEOWNERS} is missing (ADR-0044)")),
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// `#[cfg(test)]` may only declare the out-of-line `mod tests;`, on the same
/// line or the next non-empty one; test attributes never appear (ADR-0039).
pub(crate) fn check_source(rel: &str, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut problems = Vec::new();
    for (no, line) in lines.iter().enumerate() {
        let line = line.trim();
        let misplaced = match line.strip_prefix("#[cfg(test)]") {
            Some(rest) => {
                let item = match rest.trim() {
                    "" => lines[no + 1..].iter().map(|l| l.trim()).find(|l| !l.is_empty()),
                    same_line => Some(same_line),
                };
                item != Some("mod tests;")
            }
            None => TEST_ATTRIBUTES.iter().any(|a| line.starts_with(a)),
        };
        if misplaced {
            problems.push(format!(
                "{rel}:{}: test code belongs in its own file: `#[cfg(test)] mod tests;` \
                 + tests.rs (ADR-0039)",
                no + 1
            ));
        }
    }
    problems
}

/// Suppressed lints: every lint of an `expect` attribute, also inside
/// (nested) `cfg_attr`, wherever the attribute stands and however it is
/// spread over lines; plain `allow` is already rejected by clippy (ADR-0040).
pub(crate) fn count_suppressions(text: &str) -> usize {
    let tokens = tokens(text);
    let attribute = |i: usize| {
        let rest = &tokens[i + 1..];
        let rest = rest.strip_prefix(&[Token::Punct(b'!')]).unwrap_or(rest);
        if rest.first() == Some(&Token::Punct(b'[')) {
            group_parts(rest).iter().map(|meta| meta_lints(meta)).sum()
        } else {
            0
        }
    };
    (0..tokens.len()).filter(|&i| tokens[i] == Token::Punct(b'#')).map(attribute).sum()
}

/// The lints an attribute's content suppresses: the arguments of `expect`
/// except `reason = …`, or those of the attributes in a `cfg_attr`.
fn meta_lints(meta: &[Token<'_>]) -> usize {
    match meta {
        [Token::Ident("expect"), args @ ..] => group_parts(args)
            .iter()
            .filter(|lint| {
                !lint.is_empty()
                    && !matches!(lint, [Token::Ident("reason"), Token::Punct(b'='), ..])
            })
            .count(),
        // The first part is the predicate.
        [Token::Ident("cfg_attr"), args @ ..] => {
            group_parts(args).iter().skip(1).map(|attribute| meta_lints(attribute)).sum()
        }
        _ => 0,
    }
}

/// The comma-separated parts of the bracket group `tokens` starts with
/// (`(a, b(c, d))` → `a`, `b(c, d)`), none if it starts with no bracket.
fn group_parts<'t, 'a>(tokens: &'t [Token<'a>]) -> Vec<&'t [Token<'a>]> {
    let mut parts = Vec::new();
    if !matches!(tokens.first(), Some(Token::Punct(b'(' | b'[' | b'{'))) {
        return parts;
    }
    let (mut depth, mut start) = (0, 1);
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Punct(b'(' | b'[' | b'{') => depth += 1,
            Token::Punct(b')' | b']' | b'}') => depth -= 1,
            Token::Punct(b',') if depth == 1 => {
                parts.push(&tokens[start..i]);
                start = i + 1;
                continue;
            }
            _ => continue,
        }
        if depth == 0 {
            parts.push(&tokens[start..i]);
            break;
        }
    }
    parts
}

/// What the suppression count sees of Rust source: comments are gone,
/// string and char literals are opaque.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    Ident(&'a str),
    Punct(u8),
    Literal,
}

fn tokens(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let rest = &bytes[i..];
        if byte.is_ascii_whitespace() {
            i += 1;
        } else if rest.starts_with(b"//") {
            i += rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
        } else if rest.starts_with(b"/*") {
            i += block_comment_len(rest);
        } else if byte == b'"' {
            i += quoted_len(rest);
            tokens.push(Token::Literal);
        } else if byte == b'\'' {
            // A lifetime or label: its name follows as an identifier.
            let len = char_literal_len(&text[i..]);
            i += len.unwrap_or(1);
            tokens.extend(len.map(|_| Token::Literal));
        } else if byte.is_ascii_alphanumeric() || byte == b'_' {
            let len = rest
                .iter()
                .position(|&b| !(b.is_ascii_alphanumeric() || b == b'_'))
                .unwrap_or(rest.len());
            let word = &text[i..i + len];
            let after = &rest[len..];
            let literal = match (word, after.first()) {
                ("r" | "br" | "cr", Some(b'"' | b'#')) => raw_string_len(after),
                ("b" | "c", Some(b'"')) | ("b", Some(b'\'')) => Some(quoted_len(after)),
                _ => None,
            };
            i += len + literal.unwrap_or(0);
            tokens.push(if literal.is_some() { Token::Literal } else { Token::Ident(word) });
        } else {
            i += 1;
            tokens.push(Token::Punct(byte));
        }
    }
    tokens
}

/// Length of the (nested) block comment `rest` starts with.
fn block_comment_len(rest: &[u8]) -> usize {
    let (mut depth, mut i) = (0, 0);
    while i < rest.len() {
        if rest[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if rest[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    rest.len()
}

/// Length of the literal `rest` starts with, up to the unescaped quote that
/// matches its first byte (`"…"`, `'…'`).
fn quoted_len(rest: &[u8]) -> usize {
    let mut i = 1;
    while let Some(&byte) = rest.get(i) {
        if byte == b'\\' {
            i += 2;
        } else if byte == rest[0] {
            return i + 1;
        } else {
            i += 1;
        }
    }
    rest.len()
}

/// Length of a raw string after its `r`/`br`/`cr` (`#"…"#`), `None` for a
/// raw identifier (`r#type`).
fn raw_string_len(after: &[u8]) -> Option<usize> {
    let hashes = after.iter().take_while(|&&b| b == b'#').count();
    if after.get(hashes) != Some(&b'"') {
        return None;
    }
    let body = &after[hashes + 1..];
    let close = body
        .windows(hashes + 1)
        .position(|w| w[0] == b'"' && w[1..].iter().all(|&b| b == b'#'))
        .map_or(body.len(), |at| at + hashes + 1);
    Some(hashes + 1 + close)
}

/// Length of the char literal `rest` starts with (`'a'`, `'"'`, `'\''`),
/// `None` for a lifetime or label (`'a`, `'static`).
fn char_literal_len(rest: &str) -> Option<usize> {
    let mut chars = rest[1..].chars();
    match chars.next()? {
        '\\' => Some(quoted_len(rest.as_bytes())),
        c => (chars.next() == Some('\'')).then_some(2 + c.len_utf8()),
    }
}

const RATCHET: &str = "harness/ratchet.txt";

fn check_ratchet(root: &Path, suppressions: usize) -> Option<String> {
    let text = fs::read_to_string(root.join(RATCHET)).unwrap_or_default();
    let limit = text
        .lines()
        .find_map(|l| l.strip_prefix("suppressions =").map(|v| v.trim().parse::<usize>()));
    match limit {
        Some(Ok(limit)) if suppressions > limit => Some(format!(
            "{suppressions} lint suppressions (#[expect]), {limit} allowed ({RATCHET}). \
             Fix the cause instead of suppressing it (ADR-0040)."
        )),
        Some(Ok(limit)) => {
            if suppressions < limit {
                println!(
                    "hygiene: only {suppressions} suppressions left – lower {RATCHET} to {suppressions}"
                );
            }
            None
        }
        _ => Some(format!("{RATCHET}: line `suppressions = <number>` is missing")),
    }
}

pub(crate) fn check_manifest(rel: &str, text: &str) -> Vec<String> {
    let inherits = text
        .split("\n[")
        .any(|section| section.starts_with("lints]") && section.contains("workspace = true"));
    if inherits {
        Vec::new()
    } else {
        vec![format!("{rel}: `[lints] workspace = true` is missing (ADR-0040)")]
    }
}

/// The language rule over every versioned text file (tracked and untracked,
/// not ignored).
fn check_versioned_files(root: &Path) -> Vec<String> {
    let listing = match git(root, &["ls-files", "-co", "--exclude-standard", "-z"]) {
        Ok(listing) => listing,
        Err(e) => return vec![e],
    };
    let allow =
        parse_allowlist(&fs::read_to_string(root.join(LANGUAGE_ALLOWLIST)).unwrap_or_default());
    listing
        .split('\0')
        .filter(|rel| !rel.is_empty())
        .filter_map(|rel| Some((rel, fs::read_to_string(root.join(rel)).ok()?)))
        .flat_map(|(rel, text)| check_language(rel, &text, &allow))
        .collect()
}

/// `path: <prefix>` and `word: <word>` lines; `#` comments and blank lines are ignored.
pub(crate) fn parse_allowlist(text: &str) -> Allowlist {
    let mut allow = Allowlist::default();
    for line in text.lines().map(str::trim) {
        if let Some(path) = line.strip_prefix("path:") {
            allow.paths.push(path.trim().to_string());
        } else if let Some(word) = line.strip_prefix("word:") {
            allow.words.push(word.trim().to_string());
        }
    }
    allow
}

/// Lines of one file that contain German umlauts or sharp s outside allowlisted words.
pub(crate) fn check_language(rel: &str, text: &str, allow: &Allowlist) -> Vec<String> {
    let allowed_path =
        allow.paths.iter().any(|p| if p.ends_with('/') { rel.starts_with(p) } else { rel == p });
    if allowed_path {
        return Vec::new();
    }
    let mut problems = Vec::new();
    for (no, line) in text.lines().enumerate() {
        let line = allow.words.iter().fold(line.to_string(), |l, w| l.replace(w.as_str(), ""));
        if let Some(c) = line.chars().find(|c| GERMAN_CHARS.contains(c)) {
            problems.push(format!(
                "{rel}:{}: `{c}` is not English – the repository is English, German only in \
                 locales/de/ (ADR-0045, {LANGUAGE_ALLOWLIST})",
                no + 1
            ));
        }
    }
    problems
}

/// Every harness path outside `target/` needs an owner line `/<path> @owner`.
pub(crate) fn check_codeowners(text: &str) -> Vec<String> {
    let owned: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let mut tokens = l.split_whitespace();
            let path = tokens.next()?;
            tokens.next().is_some().then_some(path)
        })
        .collect();
    paths::HARNESS
        .iter()
        .filter(|entry| !entry.starts_with("target/"))
        .filter(|entry| !owned.iter().any(|o| o.strip_prefix('/') == Some(**entry)))
        .map(|entry| {
            format!("{CODEOWNERS} has no owner line for harness path `{entry}` (ADR-0044)")
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() != "target" {
                collect(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests;
