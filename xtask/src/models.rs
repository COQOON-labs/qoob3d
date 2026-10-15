//! Reference models as a quality gate (ADR-0033).
//!
//! `models/<name>.qbd` are generated from `qoob3d_core::document::examples`
//! (`--write`). The check proves, headless through the real binary:
//! - every example is written and up to date, every model has an
//!   `<name>.expect.json` with analytic truths;
//! - every feature kind and curve kind is used by at least one model;
//! - `qoob3d <model> --report` reports no errors, the expected number of
//!   valid, watertight bodies, the volume and the bounding box;
//! - `--export` writes a readable 3MF and a non-empty STL.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use qoob3d_core::document::{CurveKind, FeatureKind, examples};
use serde_json::Value;

pub(crate) fn write(root: &Path) -> Result<(), Vec<String>> {
    let dir = root.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| vec![e.to_string()])?;
    for (name, doc) in examples::all() {
        let path = dir.join(format!("{name}.qbd"));
        std::fs::write(&path, doc.to_json() + "\n").map_err(|e| vec![format!("{name}: {e}")])?;
        println!("models: wrote {}", path.display());
    }
    Ok(())
}

pub(crate) fn check(root: &Path) -> Result<(), Vec<String>> {
    let dir = root.join("models");
    let mut problems = Vec::new();
    let mut kinds = BTreeSet::new();
    let mut curves = BTreeSet::new();
    let examples = examples::all();

    // 1. Files in sync with the examples.
    for (name, doc) in &examples {
        let path = dir.join(format!("{name}.qbd"));
        match std::fs::read_to_string(&path) {
            Ok(text) if text == doc.to_json() + "\n" => {}
            Ok(_) => problems.push(format!(
                "{name}.qbd differs from examples::{name} – run `cargo xtask models --write`"
            )),
            Err(_) => {
                problems.push(format!("{name}.qbd is missing – run `cargo xtask models --write`"));
            }
        }
        if !dir.join(format!("{name}.expect.json")).exists() {
            problems.push(format!("{name}.expect.json is missing (analytic expectations)"));
        }
        for f in &doc.features {
            kinds.insert(f.kind.kind_id());
            if let FeatureKind::Sketch(s) = &f.kind {
                for c in &s.curves {
                    curves.insert(match c.kind {
                        CurveKind::Line { .. } => "line",
                        CurveKind::Circle { .. } => "circle",
                        CurveKind::Arc { .. } => "arc",
                    });
                }
            }
        }
    }
    // 2. Coverage: a new feature or curve kind needs a reference model.
    for kind in FeatureKind::KIND_IDS {
        if !kinds.contains(kind) {
            problems.push(format!("feature kind `{kind}` is used by no model in models/"));
        }
    }
    for curve in ["line", "circle", "arc"] {
        if !curves.contains(curve) {
            problems.push(format!("curve kind `{curve}` is used by no model in models/"));
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    // 3. Build every model through the real binary.
    let binary = crate::cargo_target(root).join("debug/qoob3d");
    let out_dir = root.join("target/models");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        return Err(vec![e.to_string()]);
    }
    for (name, _) in &examples {
        if let Err(p) = check_model(&binary, &dir, &out_dir, name) {
            problems.extend(p.into_iter().map(|m| format!("{name}: {m}")));
        } else {
            println!("models: ok    {name}");
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

fn check_model(binary: &Path, dir: &Path, out: &Path, name: &str) -> Result<(), Vec<String>> {
    let model = dir.join(format!("{name}.qbd"));
    let expect: Value = std::fs::read_to_string(dir.join(format!("{name}.expect.json")))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| vec!["expect.json is not valid JSON".to_string()])?;
    let three = out.join(format!("{name}.3mf"));
    let stl = out.join(format!("{name}.stl"));
    let mut problems = Vec::new();

    let run = |extra: &[&str]| {
        Command::new(binary)
            .arg(&model)
            .args(extra)
            .output()
            .map_err(|e| vec![format!("cannot run {}: {e}", binary.display())])
    };
    let output = run(&["--report", "--export", &three.to_string_lossy()])?;
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        vec![format!("no JSON report ({e}); stderr: {}", String::from_utf8_lossy(&output.stderr))]
    })?;
    if !output.status.success() || report["ok"] != Value::Bool(true) {
        problems.push(format!("report not ok: {}", report["errors"]));
    }
    let bodies = report["bodies"].as_array().cloned().unwrap_or_default();
    if let Some(n) = expect["bodies"].as_u64()
        && bodies.len() as u64 != n
    {
        problems.push(format!("{} bodies, expected {n}", bodies.len()));
    }
    let volume = report["total_volume_mm3"].as_f64().unwrap_or(f64::NAN);
    if let (Some(v), Some(tol)) = (expect["volume_mm3"].as_f64(), expect["volume_rel_tol"].as_f64())
        && ((volume - v).abs() / v > tol || volume.is_nan())
    {
        problems.push(format!("volume {volume:.4} mm³, expected {v:.4} (±{tol})"));
    }
    if let (Some(lo), Some(hi)) = (expect["volume_min"].as_f64(), expect["volume_max"].as_f64())
        && !(lo..=hi).contains(&volume)
    {
        problems.push(format!("volume {volume:.2} mm³ outside {lo}…{hi}"));
    }
    if let Some(tol) = expect["bbox_tol"].as_f64() {
        let axis = |key: &str, pick: fn(f64, f64) -> f64| -> Vec<f64> {
            (0..3)
                .map(|k| {
                    bodies
                        .iter()
                        .filter_map(|b| b[key][k].as_f64())
                        .reduce(pick)
                        .unwrap_or(f64::NAN)
                })
                .collect()
        };
        let (min, max) = (axis("bbox_min", f64::min), axis("bbox_max", f64::max));
        for (key, got) in [("bbox_min", &min), ("bbox_max", &max)] {
            let want: Vec<f64> =
                expect[key].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
            let off = |k: usize| (got[k] - want[k]).abs() > tol || got[k].is_nan();
            if want.len() == 3 && (0..3).any(off) {
                problems.push(format!("{key} {got:?}, expected {want:?} (±{tol})"));
            }
        }
    }

    // Export: 3MF readable with one object per body, STL non-empty.
    match std::fs::read(&three).map_err(|e| e.to_string()).and_then(|b| {
        qoob3d_core::export::unzip_stored(&b).map_err(|e| format!("3MF unreadable: {e}"))
    }) {
        Ok(files) => {
            let model = files
                .iter()
                .find(|(n, _)| n == "3D/3dmodel.model")
                .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
                .unwrap_or_default();
            if model.matches("<object ").count() != bodies.len() {
                problems.push("3MF object count differs from the body count".into());
            }
        }
        Err(e) => problems.push(e),
    }
    // Independent check with Info-ZIP (our own reader once missed a bug).
    if let Ok(o) = Command::new("unzip").arg("-tq").arg(&three).output()
        && !o.status.success()
    {
        problems.push(format!("unzip -t rejects the 3MF: {}", String::from_utf8_lossy(&o.stdout)));
    }
    // The real consumer: Bambu Studio's own loader, if installed (ADR-0033).
    if let Some(slicer) = bambu_studio() {
        problems.extend(check_in_slicer(&slicer, &three, bodies.len(), volume));
    }
    match run(&["--export", &stl.to_string_lossy()]) {
        Ok(o) if o.status.success() => {
            let size = std::fs::metadata(&stl).map_or(0, |m| m.len());
            if size <= 84 || !(size - 84).is_multiple_of(50) {
                problems.push(format!("STL has an invalid size ({size} bytes)"));
            }
        }
        Ok(o) => {
            problems.push(format!("STL export failed: {}", String::from_utf8_lossy(&o.stderr)));
        }
        Err(e) => problems.extend(e),
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Bambu Studio's CLI (`QOOB3D_BAMBU_STUDIO` or the default macOS location).
fn bambu_studio() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("QOOB3D_BAMBU_STUDIO").map_or_else(
        || std::path::PathBuf::from("/Applications/BambuStudio.app/Contents/MacOS/BambuStudio"),
        Into::into,
    );
    path.exists().then_some(path)
}

/// `BambuStudio --info file.3mf` must load every body, manifold, with our volume.
fn check_in_slicer(slicer: &Path, file: &Path, bodies: usize, volume: f64) -> Vec<String> {
    // Bambu Studio writes result.json into its working directory.
    let output = match Command::new(slicer)
        .arg("--info")
        .arg(file)
        .current_dir(std::env::temp_dir())
        .output()
    {
        Ok(o) => o,
        Err(e) => return vec![format!("Bambu Studio did not start: {e}")],
    };
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let mut problems = Vec::new();
    if text.contains("failed") || !output.status.success() {
        let line = text.lines().find(|l| l.contains("failed")).unwrap_or("non-zero exit");
        return vec![format!("Bambu Studio cannot load the 3MF: {line}")];
    }
    let values = |key: &str| -> Vec<String> {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix(key)?.trim().strip_prefix('=').map(str::trim))
            .map(str::to_string)
            .collect()
    };
    let objects = values("number_of_facets").len();
    if objects != bodies {
        problems.push(format!("Bambu Studio sees {objects} objects, expected {bodies}"));
    }
    if values("manifold").iter().any(|m| m != "yes") {
        problems.push("Bambu Studio reports a non-manifold object".into());
    }
    let total: f64 = values("volume").iter().filter_map(|v| v.parse::<f64>().ok()).sum();
    if (total - volume).abs() / volume > 0.01 {
        problems.push(format!("Bambu Studio volume {total:.2} mm³ vs {volume:.2} mm³"));
    }
    problems
}
