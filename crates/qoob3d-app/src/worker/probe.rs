//! Worker jobs that only look at built parts (R4, agent.md §4.2–4.3): one
//! check of a check file (AG-CK-01..03), a throwaway script (AG-SV-05) and
//! the measures of one or two parts (AG-SV-06). `Request::deps` maps the
//! built parts to their BREP files; nothing is written.

use std::time::Instant;

use qoob3d_core::project::Values;
use qoob3d_kernel::script::{self, CheckOptions, RunOptions, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{PartError, Request, io_error, load_error};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Probe {
    /// Runs the check `name` of `Request::file`.
    Check { name: String },
    /// Runs `source`; `part("Name")` returns the built parts.
    Run { source: String },
    /// Measures the first part, and against the second if there is one.
    Measure { parts: Vec<String> },
}

pub(super) fn answer(request: &Request, probe: &Probe) -> Result<Value, PartError> {
    match probe {
        Probe::Check { name } => check(request, name),
        Probe::Run { source } => run(request, source),
        Probe::Measure { parts } => measure(request, parts),
    }
}

/// `{min, max, size}` of the bounding box of `shape`.
pub(super) fn bbox(shape: &Shape) -> Value {
    let (min, max) = shape.bbox();
    json!({"min": min, "max": max, "size": std::array::from_fn::<f64, 3, _>(|i| max[i] - min[i])})
}

/// The measures of one shape (AG-SV-05/06).
fn measures(shape: &Shape) -> Value {
    json!({
        "volume": shape.volume(),
        "area": shape.area(),
        "bbox": bbox(shape),
        "center": shape.center(),
        "valid": shape.is_valid(),
        "solids": shape.solid_count(),
    })
}

fn options(request: &Request, only: Option<Vec<String>>) -> Result<RunOptions, PartError> {
    let values = Values::load(&request.dir).map_err(load_error)?;
    let built = request.deps.iter().map(|(n, p)| (n.clone(), request.dir.join(p))).collect();
    Ok(RunOptions {
        project_dir: Some(request.dir.clone()),
        values: values.iter().map(|v| (v.id.clone(), v.value.clone())).collect(),
        check: CheckOptions { built, only },
        ..RunOptions::default()
    })
}

/// `{expectations, error?, parts_read, duration_s}` of one check.
fn check(request: &Request, name: &str) -> Result<Value, PartError> {
    let start = Instant::now();
    let file = request.file.as_deref().ok_or_else(|| load_error("check without a file"))?;
    let text = std::fs::read_to_string(request.dir.join(file)).map_err(|e| io_error(file, e))?;
    let options = options(request, Some(vec![name.to_owned()]))?;
    let output = script::run(&text, &options).map_err(|e| PartError::script(Some(file), &e))?;
    let Some(check) = output.checks.into_iter().find(|c| c.name == name) else {
        return Err(load_error(format!("{}: no check {name:?}", file.display())));
    };
    let expectations: Vec<Value> = check
        .expectations
        .iter()
        .map(|e| {
            json!({
                "text": e.text,
                "passed": e.passed,
                "actual": e.actual,
                "expected": e.expected,
                "tol": e.tol,
            })
        })
        .collect();
    Ok(json!({
        "expectations": expectations,
        "error": check.error.as_ref().map(|e| PartError::script(Some(file), e)),
        "parts_read": check.parts_read,
        "duration_s": start.elapsed().as_secs_f64(),
    }))
}

/// `{shapes: [{name, …measures}], printed}` of a throwaway script.
fn run(request: &Request, source: &str) -> Result<Value, PartError> {
    let output =
        script::run(source, &options(request, None)?).map_err(|e| PartError::script(None, &e))?;
    let shapes: Vec<Value> = output
        .shapes
        .iter()
        .map(|(name, shape)| {
            let mut m = measures(shape);
            m["name"] = json!(name);
            m
        })
        .collect();
    Ok(json!({"shapes": shapes, "printed": output.printed}))
}

/// The measures of `parts[0]`, with `distance` and `common_volume` to
/// `parts[1]` if given.
fn measure(request: &Request, parts: &[String]) -> Result<Value, PartError> {
    let load = |name: &String| -> Result<Shape, PartError> {
        let brep = request
            .deps
            .get(name)
            .ok_or_else(|| load_error(format!("part {name:?} is not built")))?;
        let path = request.dir.join(brep);
        script::read_brep(&path).map_err(|e| io_error(&path, e))
    };
    let a = load(parts.first().ok_or_else(|| load_error("measure without a part"))?)?;
    let mut result = measures(&a);
    if let Some(b) = parts.get(1) {
        let b = load(b)?;
        let failed = |e| PartError::new(super::ErrorKind::Script, format!("measure: {e}"));
        let distance = script::distance(&a, &b).map_err(failed)?;
        // Apart: nothing in common, and no boolean that could only fail.
        let common =
            if distance > 0.0 { 0.0 } else { script::common_volume(&a, &b).map_err(failed)? };
        result["distance"] = json!(distance);
        result["common_volume"] = json!(common);
    }
    Ok(result)
}
