//! Headless work on a `.qbd` file (ADR-0033): `--report` (JSON with every
//! body's volume, validity and bounds), `--export` (STL/3MF) and the model
//! for `--render`. Exit code 1 on feature errors or bodies that are not
//! printable, so scripts and the coding agent can rely on it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use qoob3d_core::ShapeMesh;
use qoob3d_core::document::Document;
use qoob3d_core::export::{ExportObject, stl, three_mf};
use qoob3d_kernel::{EXPORT_TESSELLATION_TOLERANCE_MM, Kernel, RebuildResult};
use serde::Serialize;

const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Serialize)]
pub struct Report {
    pub features: usize,
    pub errors: Vec<FeatureErrorReport>,
    pub bodies: Vec<BodyReport>,
    pub total_volume_mm3: f64,
    /// No errors, every body valid and watertight.
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct FeatureErrorReport {
    pub feature: u64,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct BodyReport {
    pub name: String,
    pub volume_mm3: f64,
    pub valid: bool,
    pub watertight: bool,
    pub triangles: usize,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
}

pub fn load(path: &Path) -> anyhow::Result<Document> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Document::from_json(&text).with_context(|| format!("loading {}", path.display()))
}

pub fn rebuild(kernel: &mut Kernel, doc: &Arc<Document>) -> anyhow::Result<RebuildResult> {
    Ok(kernel.rebuild_blocking(doc.clone(), TIMEOUT)?)
}

pub fn report(doc: &Document, result: &RebuildResult) -> Report {
    let errors: Vec<FeatureErrorReport> = result
        .errors
        .iter()
        .map(|(id, error)| FeatureErrorReport {
            feature: id.0,
            name: doc.feature(*id).map_or_else(String::new, |f| f.name.clone()),
            error: error.clone(),
        })
        .collect();
    let bodies: Vec<BodyReport> = result
        .bodies
        .iter()
        .map(|b| {
            let analysis = b.mesh.faces.analyze();
            let bounds = b.mesh.faces.bounds().unwrap_or_default();
            BodyReport {
                name: format!("Body{}", b.number),
                volume_mm3: b.volume_mm3,
                valid: b.valid,
                watertight: analysis.is_watertight(),
                triangles: analysis.triangles,
                bbox_min: bounds.min,
                bbox_max: bounds.max,
            }
        })
        .collect();
    let ok = errors.is_empty() && bodies.iter().all(|b| b.valid && b.watertight);
    Report {
        features: doc.features.len(),
        total_volume_mm3: bodies.iter().map(|b| b.volume_mm3).sum(),
        errors,
        bodies,
        ok,
    }
}

/// All bodies merged, for rendering.
pub fn scene(result: &RebuildResult) -> ShapeMesh {
    ShapeMesh::merge(result.bodies.iter().map(|b| b.mesh.as_ref()))
}

/// Writes STL or 3MF depending on the extension.
pub fn export(kernel: &mut Kernel, doc: &Arc<Document>, out: &Path) -> anyhow::Result<usize> {
    let bodies = kernel.export_blocking(doc.clone(), EXPORT_TESSELLATION_TOLERANCE_MM, TIMEOUT)?;
    if bodies.is_empty() {
        bail!("the document has no bodies to export");
    }
    let objects: Vec<ExportObject<'_>> = bodies
        .iter()
        .map(|b| ExportObject { name: format!("Body{}", b.number), mesh: &b.mesh })
        .collect();
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let bytes = match ext.as_str() {
        "stl" => stl(&objects),
        "3mf" => three_mf(&objects),
        _ => bail!("unknown export format {ext:?} (use .stl or .3mf)"),
    };
    std::fs::write(out, bytes).with_context(|| format!("writing {}", out.display()))?;
    Ok(objects.len())
}
