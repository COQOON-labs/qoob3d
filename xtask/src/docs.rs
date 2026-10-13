//! Documentation lint (ADR-0001).
//!
//! - every ADR has a complete, valid front matter and the required sections
//! - the ADR id matches its file name, supersede links are mutual
//! - `docs/adr/README.md` contains the generated index (`--fix` rewrites it)
//! - every `ADR-NNNN` reference in code and docs points to an existing ADR
//! - every crate has its own CLAUDE.md

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const STATUSES: &[&str] = &["vorgeschlagen", "akzeptiert", "ersetzt", "verworfen"];
const REQUIRED_KEYS: &[&str] =
    &["id", "title", "status", "date", "crates", "tags", "supersedes", "superseded_by"];
const REQUIRED_SECTIONS: &[&str] = &["## Kontext", "## Entscheidung", "## Konsequenzen"];
const KNOWN_CRATES: &[&str] =
    &["qoob3d-core", "qoob3d-kernel", "qoob3d-render", "qoob3d-app", "xtask"];
const INDEX_START: &str = "<!-- adr-index:start";
const INDEX_END: &str = "<!-- adr-index:end -->";
/// File types scanned for `ADR-NNNN` references.
const SCANNED_EXTENSIONS: &[&str] = &["rs", "md", "wgsl", "toml", "json"];

struct Adr {
    number: u32,
    file: String,
    meta: BTreeMap<String, String>,
}

impl Adr {
    fn id(&self) -> String {
        format!("ADR-{:04}", self.number)
    }
    fn get(&self, key: &str) -> &str {
        self.meta.get(key).map_or("", String::as_str)
    }
}

pub fn check(root: &Path, fix: bool) -> Result<(), Vec<String>> {
    let adr_dir = root.join("docs/adr");
    let mut problems = Vec::new();
    let adrs = load_adrs(&adr_dir, &mut problems);
    let by_id: BTreeMap<String, &Adr> = adrs.iter().map(|a| (a.id(), a)).collect();

    for adr in &adrs {
        validate_adr(adr, &by_id, &mut problems);
    }
    check_index(&adr_dir.join("README.md"), &adrs, fix, &mut problems);
    check_references(root, &by_id, &mut problems);
    check_crate_docs(root, &mut problems);

    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

fn load_adrs(dir: &Path, problems: &mut Vec<String>) -> Vec<Adr> {
    let mut adrs = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        problems.push(format!("{} fehlt", dir.display()));
        return adrs;
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if !name.ends_with(".md") || name == "README.md" {
            continue;
        }
        let Some(number) = name.get(..4).and_then(|n| n.parse::<u32>().ok()) else {
            problems.push(format!("{name}: Dateiname muss mit NNNN- beginnen"));
            continue;
        };
        if number == 0 {
            continue; // template
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        let Some(meta) = front_matter(&text) else {
            problems.push(format!("{name}: YAML-Kopfblock (--- … ---) fehlt"));
            continue;
        };
        for section in REQUIRED_SECTIONS {
            if !text.lines().any(|l| l.trim_end() == *section) {
                problems.push(format!("{name}: Abschnitt `{section}` fehlt"));
            }
        }
        adrs.push(Adr { number, file: name, meta });
    }
    adrs
}

/// Minimal parser for our flat `key: value` front matter.
fn front_matter(text: &str) -> Option<BTreeMap<String, String>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut meta = BTreeMap::new();
    for line in lines {
        if line.trim() == "---" {
            return Some(meta);
        }
        if let Some((k, v)) = line.split_once(':') {
            meta.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    None
}

fn list(value: &str) -> Vec<String> {
    value
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn validate_adr(adr: &Adr, by_id: &BTreeMap<String, &Adr>, problems: &mut Vec<String>) {
    let f = &adr.file;
    for key in REQUIRED_KEYS {
        if !adr.meta.contains_key(*key) {
            problems.push(format!("{f}: Feld `{key}` fehlt im Kopfblock"));
        }
    }
    if adr.get("id") != adr.id() {
        problems.push(format!(
            "{f}: id `{}` passt nicht zum Dateinamen ({})",
            adr.get("id"),
            adr.id()
        ));
    }
    if adr.get("title").len() < 10 {
        problems.push(format!("{f}: title fehlt oder ist zu kurz"));
    }
    let status = adr.get("status");
    if !STATUSES.contains(&status) {
        problems.push(format!("{f}: status `{status}` ungültig, erlaubt: {STATUSES:?}"));
    }
    if !is_date(adr.get("date")) {
        problems.push(format!("{f}: date `{}` ist kein YYYY-MM-DD", adr.get("date")));
    }
    for krate in list(adr.get("crates")) {
        if !KNOWN_CRATES.contains(&krate.as_str()) {
            problems.push(format!("{f}: unbekanntes Crate `{krate}`"));
        }
    }

    // Supersede links must be mutual.
    let superseded_by = adr.get("superseded_by");
    match (status, superseded_by) {
        ("ersetzt", "null" | "") => {
            problems.push(format!("{f}: status ersetzt, aber superseded_by ist leer"));
        }
        (_, "null" | "") => {}
        (_, other) => match by_id.get(other) {
            None => problems.push(format!("{f}: superseded_by verweist auf unbekannte {other}")),
            Some(newer) if !list(newer.get("supersedes")).contains(&adr.id()) => {
                problems.push(format!("{f}: {other} führt {} nicht in `supersedes`", adr.id()))
            }
            Some(_) if status != "ersetzt" => {
                problems.push(format!("{f}: hat superseded_by, status muss `ersetzt` sein"));
            }
            Some(_) => {}
        },
    }
    for old in list(adr.get("supersedes")) {
        match by_id.get(&old) {
            None => problems.push(format!("{f}: supersedes verweist auf unbekannte {old}")),
            Some(o) if o.get("superseded_by") != adr.id() => {
                problems.push(format!("{f}: {old} hat nicht superseded_by: {}", adr.id()));
            }
            Some(_) => {}
        }
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn render_index(adrs: &[Adr]) -> String {
    let mut out = String::from(
        "| ADR | Titel | Status | Datum | Crates |\n| --- | --- | --- | --- | --- |\n",
    );
    for a in adrs {
        let crates = list(a.get("crates")).join(", ");
        out.push_str(&format!(
            "| [{}]({}) | {} | {} | {} | {} |\n",
            a.id(),
            a.file,
            a.get("title"),
            a.get("status"),
            a.get("date"),
            if crates.is_empty() { "–".into() } else { crates },
        ));
    }
    out
}

fn check_index(readme: &Path, adrs: &[Adr], fix: bool, problems: &mut Vec<String>) {
    let Ok(text) = fs::read_to_string(readme) else {
        problems.push("docs/adr/README.md fehlt".into());
        return;
    };
    let (Some(start), Some(end)) = (text.find(INDEX_START), text.find(INDEX_END)) else {
        problems.push("docs/adr/README.md: Index-Marker fehlen".into());
        return;
    };
    let start_line_end = start + text[start..].find('\n').unwrap_or(0) + 1;
    let current = &text[start_line_end..end];
    let expected = render_index(adrs);
    if current == expected {
        return;
    }
    if fix {
        let updated = format!("{}{}{}", &text[..start_line_end], expected, &text[end..]);
        match fs::write(readme, updated) {
            Ok(()) => println!("docs: ADR-Index neu erzeugt"),
            Err(e) => problems.push(format!("docs/adr/README.md schreiben: {e}")),
        }
    } else {
        problems.push("docs/adr/README.md: ADR-Index veraltet → `cargo xtask docs --fix`".into());
    }
}

fn check_references(root: &Path, by_id: &BTreeMap<String, &Adr>, problems: &mut Vec<String>) {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    for path in files {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        if rel.starts_with("docs/adr/0000-") {
            continue;
        }
        for (line_no, line) in text.lines().enumerate() {
            for id in adr_refs(line) {
                if id != "ADR-0000" && !by_id.contains_key(&id) {
                    problems.push(format!("{rel}:{}: Verweis auf unbekannte {id}", line_no + 1));
                }
            }
        }
    }
}

/// All `ADR-dddd` tokens in a line.
fn adr_refs(line: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = line;
    while let Some(pos) = rest.find("ADR-") {
        let digits = &rest[pos + 4..];
        if digits.len() >= 4 && digits.as_bytes()[..4].iter().all(u8::is_ascii_digit) {
            refs.push(format!("ADR-{}", &digits[..4]));
        }
        rest = &rest[pos + 4..];
    }
    refs
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | ".git" | "node_modules") {
                collect_files(&path, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| SCANNED_EXTENSIONS.contains(&e.to_string_lossy().as_ref()))
        {
            out.push(path);
        }
    }
}

fn check_crate_docs(root: &Path, problems: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root.join("crates")) else { return };
    for entry in entries.flatten() {
        if entry.path().is_dir() && !entry.path().join("CLAUDE.md").exists() {
            problems.push(format!(
                "crates/{}/CLAUDE.md fehlt (ADR-0001)",
                entry.file_name().to_string_lossy()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_adr_references() {
        assert_eq!(adr_refs("see ADR-0003 and ADR-0012."), ["ADR-0003", "ADR-0012"]);
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
}
