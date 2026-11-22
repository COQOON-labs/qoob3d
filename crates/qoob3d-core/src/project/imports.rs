//! Module imports of a script (AG-PJ-04): `import "lib/x" as x;` names the
//! module `<root>/lib/x.rhai`. The modules feed the cache key, so the scan
//! must find every import the script engine would load; it therefore skips
//! comments and string literals instead of matching lines.
//!
//! Imports stay inside the project (same sandbox rule as AG-SC-02).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use super::{ProjectError, read_text};

/// Every module `file` imports, transitively, each once, in the order of
/// first discovery (depth first). Paths are relative to `root`.
///
/// # Errors
/// Unreadable file, missing module, or a module outside the project.
pub fn resolve_imports(root: &Path, file: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    let canonical_root =
        root.canonicalize().map_err(|source| ProjectError::Io { path: root.to_owned(), source })?;
    let mut seen = HashSet::from([file.to_owned()]);
    let mut modules = Vec::new();
    let mut stack = vec![file.to_owned()];
    while let Some(current) = stack.pop() {
        let text = read_text(&root.join(&current))?;
        let mut found = Vec::new();
        for spec in import_specs(&text) {
            let module = module_path(root, &canonical_root, &spec).map_err(|message| {
                ProjectError::Import { file: current.clone(), module: spec.clone(), message }
            })?;
            if seen.insert(module.clone()) {
                modules.push(module.clone());
                found.push(module);
            }
        }
        // Reversed, so the first import of `current` is scanned next.
        stack.extend(found.into_iter().rev());
    }
    Ok(modules)
}

/// `<spec>.rhai` relative to `root`, if it exists inside the project.
fn module_path(root: &Path, canonical_root: &Path, spec: &str) -> Result<PathBuf, String> {
    let module = PathBuf::from(format!("{spec}.rhai"));
    if !module.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err("must be a relative path inside the project".to_owned());
    }
    let canonical = root
        .join(&module)
        .canonicalize()
        .map_err(|e| format!("module {} not found: {e}", module.display()))?;
    // A symlink could still lead out of the project.
    if !canonical.starts_with(canonical_root) {
        return Err("leads outside the project".to_owned());
    }
    Ok(module)
}

/// The string after every `import` keyword outside comments and strings.
fn import_specs(text: &str) -> Vec<String> {
    let mut specs = Vec::new();
    let mut chars = text.chars().peekable();
    let mut word = String::new();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        let after_import = word == "import";
        word.clear();
        match c {
            '/' if chars.peek() == Some(&'/') => {
                chars.by_ref().find(|&c| c == '\n');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                chars.by_ref().find(|&c| std::mem::replace(&mut previous, c) == '*' && c == '/');
            }
            '\'' => {
                string_literal(&mut chars, '\'');
            }
            '"' | '`' => {
                let literal = string_literal(&mut chars, c);
                if after_import {
                    specs.push(literal);
                }
            }
            c if c.is_whitespace() && after_import => word.push_str("import"),
            _ => {}
        }
    }
    specs
}

/// Reads a string or character literal up to the closing `quote` (opening
/// one consumed). Backtick strings are raw in Rhai.
fn string_literal(chars: &mut impl Iterator<Item = char>, quote: char) -> String {
    let mut literal = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quote != '`' => literal.extend(chars.next()),
            c if c == quote => break,
            c => literal.push(c),
        }
    }
    literal
}
