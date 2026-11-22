//! Target values `values/*.toml` (AG-PJ-03): every entry has a value, a
//! source and a status, so each number in a model is traceable; `unit` is
//! optional documentation (AG-MC-07).
//!
//! An entry is a table holding at least one non-table key; tables holding
//! only tables are namespaces. `[crane.column]` therefore is the entry
//! `crane.column` inside the namespace `crane`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{ProjectError, files_with_extension, read_text};

/// How trustworthy a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueStatus {
    Measured,
    OneSource,
    Assumption,
}

/// One target value.
#[derive(Debug, Clone, PartialEq)]
pub struct Value {
    /// Dotted table path, e.g. `crane.column`.
    pub id: String,
    pub value: serde_json::Value,
    pub source: String,
    pub status: ValueStatus,
    /// Documentation of the unit, e.g. `"mm"`; scripts get the bare value.
    pub unit: Option<String>,
    /// File that defines the value, relative to the project root.
    pub file: PathBuf,
}

/// All values of a project, by id.
#[derive(Debug, Clone, Default)]
pub struct Values {
    by_id: BTreeMap<String, Value>,
}

impl Values {
    /// Reads every `values/*.toml`; a project without `values/` has no values.
    ///
    /// # Errors
    /// Unreadable or malformed file, an entry without value, source or a
    /// known status, or an id defined in two files.
    pub fn load(root: &Path) -> Result<Self, ProjectError> {
        let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
        for file in files_with_extension(root, "values", "toml")? {
            let text = read_text(&root.join(&file))?;
            let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
                ProjectError::Toml { file: file.clone(), message: e.to_string() }
            })?;
            let mut found = Vec::new();
            collect(&file, "", &table, &mut found)?;
            for value in found {
                if let Some(first) = by_id.get(&value.id) {
                    return Err(ProjectError::DuplicateValue {
                        id: value.id,
                        first: first.file.clone(),
                        second: file,
                    });
                }
                by_id.insert(value.id.clone(), value);
            }
        }
        Ok(Self { by_id })
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.by_id.get(id)
    }

    /// All values, sorted by id.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.by_id.values()
    }

    /// The values with status assumption, sorted by id (the assumption report).
    #[must_use]
    pub fn assumptions(&self) -> Vec<&Value> {
        self.by_id.values().filter(|v| v.status == ValueStatus::Assumption).collect()
    }
}

/// Collects the entries in `table` (at table path `prefix`) into `out`.
fn collect(
    file: &Path,
    prefix: &str,
    table: &toml::Table,
    out: &mut Vec<Value>,
) -> Result<(), ProjectError> {
    for (key, item) in table {
        let id = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        let error = |message: String| ProjectError::Value {
            file: file.to_owned(),
            id: id.clone(),
            message,
        };
        let Some(inner) = item.as_table() else {
            return Err(error("expected a table with value, source and status".to_owned()));
        };
        if inner.values().all(toml::Value::is_table) {
            collect(file, &id, inner, out)?;
        } else {
            out.push(entry(file, id.clone(), inner).map_err(error)?);
        }
    }
    Ok(())
}

/// Reads one entry; the error is the message without file and id.
fn entry(file: &Path, id: String, table: &toml::Table) -> Result<Value, String> {
    if let Some(key) =
        table.keys().find(|k| !matches!(k.as_str(), "value" | "source" | "status" | "unit"))
    {
        return Err(format!("unknown key {key:?}"));
    }
    let value = table.get("value").ok_or("missing value")?;
    let value = serde_json::to_value(value).map_err(|e| format!("unsupported value: {e}"))?;
    let source = match table.get("source").map(toml::Value::as_str) {
        Some(Some(s)) if !s.trim().is_empty() => s.to_owned(),
        _ => return Err("missing source (a non-empty string)".to_owned()),
    };
    let status = match table.get("status").map(toml::Value::as_str) {
        None => return Err("missing status".to_owned()),
        Some(Some("measured")) => ValueStatus::Measured,
        Some(Some("one_source")) => ValueStatus::OneSource,
        Some(Some("assumption")) => ValueStatus::Assumption,
        Some(other) => {
            return Err(format!(
                "status {:?} is not measured, one_source or assumption",
                other.unwrap_or("<not a string>")
            ));
        }
    };
    let unit = match table.get("unit").map(toml::Value::as_str) {
        None => None,
        Some(Some(unit)) => Some(unit.to_owned()),
        Some(None) => return Err("unit must be a string".to_owned()),
    };
    Ok(Value { id, value, source, status, unit, file: file.to_owned() })
}
