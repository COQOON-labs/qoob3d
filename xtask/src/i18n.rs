//! Translation lint (ADR-0020):
//! - every language pack has exactly the keys of the English base file,
//! - every key used in the app exists: a key is any kebab-case string
//!   literal in UI code (`tr("nav-fit")`, but also `planned(ui, "file-open")`
//!   or `match … => "viewcube-top"`), except `qoob3d-…` widget ids,
//! - no key is defined but unused,
//! - no obviously user-visible string literal is passed to an egui widget.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const LOCALES: &str = "crates/qoob3d-app/locales";
const APP_SRC: &str = "crates/qoob3d-app/src";
/// Other crates that name translation keys (e.g. tool field labels).
const KEY_ONLY_SRC: &[&str] = &["crates/qoob3d-sketch/src"];
/// Calls that take a translation key as first argument.
const KEY_CALLS: &[&str] = &["tr(\"", "tr_args(\""];
/// Widget calls whose first string literal is user-visible.
const WIDGET_CALLS: &[&str] = &[
    ".label(\"",
    ".button(\"",
    "Button::new(\"",
    "on_hover_text(\"",
    "on_disabled_hover_text(\"",
    ".checkbox(",
    "selected_text(\"",
    "CollapsingHeader::new(\"",
    "RichText::new(\"",
];
/// Literals that are not language-specific.
const ALLOWED_LITERALS: &[&str] = &["QOOB3D", "🏠", "⏮", "⏴", "⏵", "⏭"];
/// Files that are developer-facing (CLI/test output), not UI.
const NON_UI_FILES: &[&str] = &["cli.rs", "smoke.rs", "headless_render.rs", "main.rs", "i18n.rs"];

pub(crate) fn check(root: &Path) -> Result<(), Vec<String>> {
    let mut problems = Vec::new();
    let locales = root.join(LOCALES);
    let base = keys(&locales.join("en/main.ftl"), &mut problems);

    let mut langs: Vec<PathBuf> =
        fs::read_dir(&locales).into_iter().flatten().flatten().map(|e| e.path()).collect();
    langs.sort();
    for dir in langs.iter().filter(|d| d.is_dir()) {
        let code = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let other = keys(&dir.join("main.ftl"), &mut problems);
        for missing in base.difference(&other) {
            problems.push(format!("{code}/main.ftl: key `{missing}` is missing"));
        }
        for extra in other.difference(&base) {
            problems.push(format!("{code}/main.ftl: key `{extra}` does not exist in en"));
        }
    }

    let mut used = BTreeSet::new();
    let mut files = Vec::new();
    collect_rs(&root.join(APP_SRC), &mut files);
    let app_files = files.len();
    for dir in KEY_ONLY_SRC {
        collect_rs(&root.join(dir), &mut files);
    }
    for (index, file) in files.into_iter().enumerate() {
        let Ok(text) = fs::read_to_string(&file) else { continue };
        let rel = file.strip_prefix(root).unwrap_or(&file).display().to_string();
        let key_only = index >= app_files;
        let ui_file = !key_only && !NON_UI_FILES.iter().any(|f| rel.ends_with(f));
        for (no, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut keys_here = literals_after(line, KEY_CALLS);
            if ui_file || key_only {
                keys_here.extend(all_literals(line).into_iter().filter(|l| looks_like_key(l)));
            }
            for key in keys_here {
                if !base.contains(&key) {
                    problems.push(format!("{rel}:{}: key `{key}` is not in en/main.ftl", no + 1));
                }
                used.insert(key);
            }
            if ui_file && !line.trim_start().starts_with("//") {
                for lit in literals_after(line, WIDGET_CALLS) {
                    if lit.chars().any(char::is_alphabetic)
                        && !ALLOWED_LITERALS.contains(&lit.as_str())
                    {
                        problems.push(format!(
                            "{rel}:{}: hard-coded UI text {lit:?} – translate via tr(\"…\")",
                            no + 1
                        ));
                    }
                }
            }
        }
    }
    for unused in base.difference(&used) {
        problems.push(format!("en/main.ftl: key `{unused}` is not used anywhere"));
    }

    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Top-level Fluent message ids (`key = …`) of a file.
fn keys(path: &Path, problems: &mut Vec<String>) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(path) else {
        problems.push(format!("{} is missing", path.display()));
        return BTreeSet::new();
    };
    text.lines()
        .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_string()))
        .collect()
}

/// All simple string literals in a line (no escape handling needed for keys).
fn all_literals(line: &str) -> Vec<String> {
    line.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

/// kebab-case with at least one dash, not a `qoob3d-…` widget id.
fn looks_like_key(s: &str) -> bool {
    s.contains('-')
        && !s.starts_with("qoob3d-")
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// String literals directly following one of `markers` in `line`.
fn literals_after(line: &str, markers: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for marker in markers {
        let mut rest = line;
        while let Some(pos) = rest.find(marker) {
            // `tr("` must not match the tail of `push_str("`.
            let glued =
                rest[..pos].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
            rest = &rest[pos + marker.len()..];
            if glued && marker.starts_with(|c: char| c.is_alphabetic()) {
                continue;
            }
            // Markers without the quote (e.g. `.checkbox(`) take the next literal on the line.
            let body = if marker.ends_with('"') {
                rest
            } else {
                match rest.find('"') {
                    // Only a literal that is itself an argument, not one inside tr(…).
                    Some(q) if rest[..q].trim_end().ends_with(',') => &rest[q + 1..],
                    _ => continue,
                }
            };
            if let Some(end) = body.find('"') {
                out.push(body[..end].to_string());
            }
        }
    }
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("tests.rs") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests;
