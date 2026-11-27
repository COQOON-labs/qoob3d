//! Project parts and target values in scripts (docs/requirements/agent.md
//! AG-PJ-02/03, §2.1, ADR-0053): `part("Name", #{ depends: [..] }, |deps| …)`
//! and `value("id")`.
//!
//! [`PartMode`] decides what `part` does: build every part in order (plain
//! script runs), only record the declarations (discovery in a worker,
//! without OCCT, ADR-0055), or build exactly one part from the BREP files of its dependencies
//! (worker process).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use rhai::{Array, Dynamic, Engine, FnPtr, Map, NativeCallContext};

use super::api::{Rr, runtime};
use super::{RunOptions, RunOutput, Shape, read_brep};
use qoob3d_core::project::PrintPose;

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
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredPart {
    pub name: String,
    pub depends: Vec<String>,
    /// How it lies on the bed for printing (`pose` option, AG-PJ-06).
    pub print: Option<PrintPose>,
    /// Its colour as `#RRGGBB` (`color` option, AG-PJ-07): shown in pictures,
    /// a filament in a slicer project.
    pub color: Option<String>,
    /// The part it prints with as one multi-colour object (`print_with`
    /// option, AG-PJ-07).
    pub print_with: Option<String>,
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
            let print = print_pose(name, &opts)?;
            let (color, print_with) = (color(name, &opts)?, print_with(name, &opts)?);
            declared.borrow_mut().parts.push(DeclaredPart {
                name: name.to_owned(),
                depends: depends.clone(),
                print,
                color,
                print_with,
            });
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

/// `options.pose`: `#{ rotate: ["x", 180], count: 2 }`, where `rotate` is
/// one `[axis, degrees]` or an array of them (applied in order) and both
/// keys are optional (AG-PJ-06).
fn print_pose(name: &str, options: &Map) -> Rr<Option<PrintPose>> {
    let Some(value) = options.get("pose") else { return Ok(None) };
    let error = |what: &str| runtime(format!("part {name:?}: pose {what}"));
    let Some(map) = value.read_lock::<Map>() else {
        return Err(error("must be a map like #{ rotate: [\"x\", 180], count: 2 }"));
    };
    if let Some(key) = map.keys().find(|k| !["rotate", "count"].contains(&k.as_str())) {
        return Err(error(&format!("has the unknown key {key:?}; known: rotate, count")));
    }
    let mut pose = PrintPose::default();
    if let Some(count) = map.get("count") {
        pose.count = count
            .as_int()
            .ok()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|&n| n >= 1)
            .ok_or_else(|| error("count must be a whole number >= 1"))?;
    }
    if let Some(rotate) = map.get("rotate") {
        let Some(list) = rotate.read_lock::<Array>() else {
            return Err(error("rotate must be [axis, degrees] or a list of them"));
        };
        // One pair, or a list of pairs.
        let pairs: Vec<Dynamic> = if list.first().is_some_and(Dynamic::is_array) {
            list.to_vec()
        } else {
            vec![rotate.clone()]
        };
        for pair in pairs {
            let Some(p) = pair.read_lock::<Array>().filter(|p| p.len() == 2) else {
                return Err(error("rotate must be [axis, degrees] or a list of them"));
            };
            let deg = super::api::finite(&p[1], &format!("part {name:?}: print rotate degrees"))?;
            pose.rotate.push((super::api::axis(&p[0])?, deg));
        }
    }
    Ok(Some(pose))
}

/// `options.color`: `"#RRGGBB"` or a basic colour name, stored as
/// `#RRGGBB` in upper case (AG-PJ-07).
fn color(name: &str, options: &Map) -> Rr<Option<String>> {
    let Some(value) = options.get("color") else { return Ok(None) };
    let text = value.clone().into_string().unwrap_or_default();
    if let Some([r, g, b]) = qoob3d_core::project::parse_color(&text) {
        return Ok(Some(format!("#{r:02X}{g:02X}{b:02X}")));
    }
    let names: Vec<&str> =
        qoob3d_core::project::COLOR_NAMES.iter().map(|(name, _)| *name).collect();
    Err(runtime(format!(
        "part {name:?}: color must be \"#RRGGBB\" (like \"#C0392B\") or one of {}, got {value}",
        names.join(", ")
    )))
}

/// `options.print_with`: the name of the part this one prints with as one
/// multi-colour object (AG-PJ-07). Whether that part exists is checked when
/// a kit is exported, because parts of other files are not known here.
fn print_with(name: &str, options: &Map) -> Rr<Option<String>> {
    let Some(value) = options.get("print_with") else { return Ok(None) };
    match value.clone().into_string() {
        Ok(host) if !host.is_empty() && host != name => Ok(Some(host)),
        _ => Err(runtime(format!(
            "part {name:?}: print_with must be the name of another part, got {value}"
        ))),
    }
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
