//! Project parts and target values in scripts (docs/requirements/agent.md
//! AG-PJ-02/03, §2.1, ADR-0053): `part("Name", #{ depends: [..] }, |deps| …)`
//! and `value("id")`.
//!
//! [`PartMode`] decides what `part` does: build every part in order (plain
//! script runs), only record the declarations (service discovery, without
//! OCCT), or build exactly one part from the BREP files of its dependencies
//! (worker process).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use rhai::{Array, Dynamic, Engine, FnPtr, Map, NativeCallContext};

use super::api::{Rr, runtime};
use super::{RunOptions, RunOutput, Shape, read_brep};

/// What `part(name, options, closure)` does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PartMode {
    /// Calls every closure in declaration order with the parts built before
    /// it as `deps` and shows the result under the part's name.
    #[default]
    All,
    /// Records name and dependencies only. No closure runs and the CAD API
    /// is not registered, so declaring never calls OCCT.
    Declare,
    /// Builds only the part `name`; `deps` maps each of its dependencies to
    /// a BREP file. Other declarations are skipped.
    One { name: String, deps: BTreeMap<String, PathBuf> },
}

/// A part as declared by `part(...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredPart {
    pub name: String,
    pub depends: Vec<String>,
}

pub(super) fn register(engine: &mut Engine, output: &Rc<RefCell<RunOutput>>, options: &RunOptions) {
    let values = options.values.clone();
    let read = Rc::clone(output);
    engine.register_fn("value", move |id: &str| -> Rr<Dynamic> {
        let Some(v) = values.get(id) else {
            return Err(runtime(format!("value: unknown value id {id:?} (values/*.toml)")));
        };
        read.borrow_mut().values_read.insert(id.to_owned(), v.clone());
        Ok(dynamic(v))
    });

    let mode = options.parts.clone();
    let declared = Rc::clone(output);
    engine.register_fn(
        "part",
        move |ctx: NativeCallContext, name: &str, opts: Map, build: FnPtr| -> Rr<()> {
            let depends = depends(name, &opts)?;
            declared
                .borrow_mut()
                .parts
                .push(DeclaredPart { name: name.to_owned(), depends: depends.clone() });
            let deps = match &mode {
                PartMode::Declare => return Ok(()),
                PartMode::One { name: wanted, .. } if wanted != name => return Ok(()),
                PartMode::One { deps: files, .. } => from_files(name, &depends, files)?,
                PartMode::All => built_before(name, &depends, &declared.borrow())?,
            };
            let result: Dynamic = build.call_within_context(&ctx, (deps,))?;
            let shape = result.try_cast::<Shape>().ok_or_else(|| {
                runtime(format!("part {name:?}: the closure must return a shape"))
            })?;
            declared.borrow_mut().shapes.push((name.to_owned(), shape));
            Ok(())
        },
    );
}

/// `options.depends` as part names (missing = none).
fn depends(name: &str, options: &Map) -> Rr<Vec<String>> {
    let Some(list) = options.get("depends") else { return Ok(Vec::new()) };
    let invalid = || runtime(format!("part {name:?}: depends must be an array of part names"));
    let list = list.read_lock::<Array>().ok_or_else(invalid)?;
    list.iter().map(|d| d.clone().into_string().map_err(|_| invalid())).collect()
}

/// `deps` from the BREP files the caller passed.
fn from_files(name: &str, depends: &[String], files: &BTreeMap<String, PathBuf>) -> Rr<Map> {
    let mut deps = Map::new();
    for d in depends {
        let Some(file) = files.get(d) else {
            return Err(runtime(format!("part {name:?}: dependency {d:?} was not built")));
        };
        let shape = read_brep(file).map_err(|e| runtime(format!("part {name:?}: {d:?}: {e}")))?;
        deps.insert(d.into(), Dynamic::from(shape));
    }
    Ok(deps)
}

/// `deps` from the parts this script built before `name`.
fn built_before(name: &str, depends: &[String], output: &RunOutput) -> Rr<Map> {
    let mut deps = Map::new();
    for d in depends {
        let Some((_, shape)) = output.shapes.iter().rev().find(|(n, _)| n == d) else {
            return Err(runtime(format!(
                "part {name:?}: dependency {d:?} must be declared before it"
            )));
        };
        deps.insert(d.into(), Dynamic::from(shape.clone()));
    }
    Ok(deps)
}

/// A TOML value (as JSON) as a script value.
fn dynamic(v: &serde_json::Value) -> Dynamic {
    match v {
        serde_json::Value::Null => Dynamic::UNIT,
        serde_json::Value::Bool(b) => Dynamic::from_bool(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Dynamic::from_float(n.as_f64().unwrap_or(f64::NAN)), Dynamic::from_int),
        serde_json::Value::String(s) => Dynamic::from(s.clone()),
        serde_json::Value::Array(a) => Dynamic::from_array(a.iter().map(dynamic).collect()),
        serde_json::Value::Object(m) => {
            Dynamic::from_map(m.iter().map(|(k, v)| (k.into(), dynamic(v))).collect())
        }
    }
}
