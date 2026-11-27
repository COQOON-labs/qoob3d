//! Reading reference files of the project (AG-SC-14, AG-SC-23, AG-SC-25):
//! `import_step` (all of a file, or its solids by STEP name), `step_names`
//! and `import_mesh`. Files are read inside the project only and recorded
//! in `RunOutput::files_read` for the cache key (AG-SC-24). A STEP file is
//! read once per run: picking many named bodies from one export of a big
//! assembly must not translate it again for every body.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rhai::{Array, Dynamic, Engine};

use qoob3d_core::import::{read_3mf, read_stl};

use super::api::{Rr, geometry, made, runtime};
use super::{RunOutput, Shape, polyhedron, project_file};
use crate::occt::ffi;

/// A STEP file as read: all its solids and the STEP name of each.
#[derive(Clone)]
struct StepFile {
    solids: Shape,
    names: Vec<String>,
}

type StepCache = Rc<RefCell<HashMap<PathBuf, StepFile>>>;

/// At most this many names are listed when a name is missing.
const NAMES_SHOWN: usize = 12;

fn step_file(
    function: &'static str,
    root: Option<&PathBuf>,
    path: &str,
    output: &RefCell<RunOutput>,
    cache: &StepCache,
) -> Rr<StepFile> {
    let Some(root) = root else {
        return Err(runtime(format!("{function} needs a project folder")));
    };
    let file = project_file(root, path, output).map_err(runtime)?;
    if let Some(read) = cache.borrow().get(&file) {
        return Ok(read.clone());
    }
    let text = file.to_str().ok_or_else(|| runtime(format!("{path:?} is not UTF-8")))?;
    let mut names = Vec::new();
    let solids = made(function, ffi::qoob3d_read_step_named(text, &mut names))?;
    let read = StepFile { solids, names };
    cache.borrow_mut().insert(file, read.clone());
    Ok(read)
}

/// The solids named `wanted` (every solid of each name) as one shape.
fn pick(path: &str, read: &StepFile, wanted: &[String]) -> Rr<Shape> {
    let mut indices = Vec::new();
    for name in wanted {
        let before = indices.len();
        indices.extend(
            read.names.iter().enumerate().filter(|(_, n)| *n == name).map(|(i, _)| i as u32),
        );
        if indices.len() == before {
            let mut known: Vec<&str> = read.names.iter().map(String::as_str).collect();
            known.dedup();
            let more = if known.len() > NAMES_SHOWN { ", …" } else { "" };
            known.truncate(NAMES_SHOWN);
            return Err(runtime(format!(
                "import_step: {path} has no solid named {name:?}; it has {}{more} (step_names lists all)",
                known.join(", ")
            )));
        }
    }
    made("import_step", ffi::qoob3d_pick_solids(read.solids.get(), &indices))
}

fn names_of(v: &Dynamic) -> Rr<Vec<String>> {
    if let Ok(name) = v.clone().into_string() {
        return Ok(vec![name]);
    }
    let Some(list) = v.read_lock::<Array>() else {
        return Err(runtime("import_step: name must be a string or an array of strings"));
    };
    let names = list
        .iter()
        .map(|n| {
            n.clone()
                .into_string()
                .map_err(|t| runtime(format!("import_step: a name must be a string, got {t}")))
        })
        .collect::<Rr<Vec<_>>>()?;
    if names.is_empty() {
        return Err(runtime("import_step: the list of names is empty"));
    }
    Ok(names)
}

pub(super) fn register(
    engine: &mut Engine,
    output: &Rc<RefCell<RunOutput>>,
    project: Option<PathBuf>,
) {
    let cache: StepCache = Rc::default();
    let (files, root, steps) = (Rc::clone(output), project.clone(), Rc::clone(&cache));
    engine.register_fn("import_step", move |path: &str| -> Rr<Shape> {
        let read = step_file("import_step", root.as_ref(), path, &files, &steps)?;
        // One solid stays itself; several come as their compound, as before.
        if read.names.len() == 1 {
            return made("import_step", ffi::qoob3d_pick_solids(read.solids.get(), &[0]));
        }
        Ok(read.solids)
    });
    let (files, root, steps) = (Rc::clone(output), project.clone(), Rc::clone(&cache));
    engine.register_fn("import_step", move |path: &str, name: Dynamic| -> Rr<Shape> {
        let read = step_file("import_step", root.as_ref(), path, &files, &steps)?;
        pick(path, &read, &names_of(&name)?)
    });
    let (files, root, steps) = (Rc::clone(output), project.clone(), cache);
    engine.register_fn("step_names", move |path: &str| -> Rr<Array> {
        let read = step_file("step_names", root.as_ref(), path, &files, &steps)?;
        Ok(read.names.into_iter().map(Dynamic::from).collect())
    });
    let files = Rc::clone(output);
    engine.register_fn("import_mesh", move |path: &str| -> Rr<Shape> {
        let Some(root) = &project else {
            return Err(runtime("import_mesh needs a project folder"));
        };
        let extension = Path::new(path).extension().map(std::ffi::OsStr::to_ascii_lowercase);
        let read = match extension.as_ref().and_then(|e| e.to_str()) {
            Some("stl") => read_stl,
            Some("3mf") => read_3mf,
            _ => {
                return Err(runtime(format!(
                    "import_mesh: {path:?} must be an .stl or a .3mf file"
                )));
            }
        };
        let file = project_file(root, path, &files).map_err(runtime)?;
        let bytes = std::fs::read(&file).map_err(|e| runtime(format!("{path}: {e}")))?;
        let mesh = read(&bytes).map_err(|e| runtime(format!("{path}: {e}")))?;
        let solids = polyhedron::polyhedron(&mesh)
            .map_err(|e| geometry("import_mesh", format!("{path}: {e}")))?;
        made("import_mesh", ffi::qoob3d_polyhedron(&solids))
    });
}
