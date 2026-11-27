//! `qoob3d render-project` (AG-RD-01, ADR-0054): what an agent looks at.
//! The service (started on demand) builds the selected parts if they are
//! stale; their cached meshes are rendered headless, orthographic, with the
//! camera fitted to them. Several views become one contact sheet with a
//! label per view (AG-MC-09).

mod sheet;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, bail};
use qoob3d_core::{Aabb, ShapeMesh};
use qoob3d_render::ViewPreset;
use serde_json::{Value, json};

use crate::headless_render;
use qoob3d_project::client::{
    BUILD_TIMEOUT, Client, ProjectDir, START_TIMEOUT, service_command_from_env,
};

/// Margin over the build wait for the answer of the build request.
const ANSWER_MARGIN: Duration = Duration::from_secs(30);

/// What to render.
#[derive(Debug, Clone)]
pub enum Scene {
    /// These parts (all if empty), built first if they are stale.
    Parts(Vec<String>),
    /// The shapes a throwaway script shows (AG-MC-12).
    Script(PathBuf),
}

/// Renders the scene of the project in `dir` into the PNG `out`: one view
/// as it is, several as a contact sheet of cells of `size`, fitted to
/// `frame` or to everything.
///
/// # Errors
/// No project, no service, an unknown or failed part (named), a failed
/// script, nothing to render, or the render itself fails.
pub fn run(
    dir: &Path,
    out: &Path,
    views: &[ViewPreset],
    size: [u32; 2],
    scene: &Scene,
    frame: Option<Aabb>,
) -> anyhow::Result<()> {
    let project = ProjectDir::open(dir)?;
    let client = Client::connect(&project, &service_command_from_env(), START_TIMEOUT)?;
    let meshes = match scene {
        Scene::Parts(parts) => part_meshes(&client, &project, parts)?,
        Scene::Script(file) => script_meshes(&client, &project, file)?,
    };
    let scene = ShapeMesh::merge(&meshes);
    if let [single] = views {
        return headless_render::run(out, *single, size, &scene, frame);
    }
    let images = headless_render::render_views(&scene, views, size, frame)?;
    let labels: Vec<&str> = views.iter().map(|v| v.name()).collect();
    let (sheet_size, sheet) = sheet::compose(&images, &labels, size);
    headless_render::write_png(out, sheet_size, &sheet)
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} (contact sheet of {})", out.display(), labels.join(", "));
    Ok(())
}

/// The meshes of the shapes the script `file` shows: `/run` writes them
/// into `.qoob3d/run/<job>/`, which is removed once they are read.
fn script_meshes(
    client: &Client,
    project: &ProjectDir,
    file: &Path,
) -> anyhow::Result<Vec<ShapeMesh>> {
    let source =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let body = json!({"source": source, "meshes": true, "wait_s": BUILD_TIMEOUT.as_secs()});
    let answer = client.call("POST", "/run", Some(&body), BUILD_TIMEOUT + ANSWER_MARGIN)?;
    if answer["ok"] != json!(true) {
        bail!("{}", answer["error"]["message"].as_str().unwrap_or("the script failed"));
    }
    let shapes = answer["result"]["shapes"].as_array().context("the script did not finish")?;
    let mut meshes = Vec::new();
    let mut folders = std::collections::BTreeSet::new();
    for shape in shapes {
        let relative = shape["mesh"].as_str().context("a shape without mesh")?;
        let path = project.dir().join(relative);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        meshes.push(qoob3d_core::qmesh::read(&bytes).with_context(|| path.display().to_string())?);
        if let Some(folder) = path.parent() {
            folders.insert(folder.to_path_buf());
        }
    }
    for folder in folders {
        let _ = std::fs::remove_dir_all(folder);
    }
    if meshes.is_empty() {
        bail!("the script shows no shape: call show(shape, \"name\")");
    }
    Ok(meshes)
}

/// The meshes of `parts` (all if empty), built first if needed.
fn part_meshes(
    client: &Client,
    project: &ProjectDir,
    parts: &[String],
) -> anyhow::Result<Vec<ShapeMesh>> {
    let wanted = if parts.is_empty() { Value::Null } else { json!(parts) };
    let body = json!({"parts": wanted, "wait_s": BUILD_TIMEOUT.as_secs()});
    let answer = client.call("POST", "/build", Some(&body), BUILD_TIMEOUT + ANSWER_MARGIN)?;
    if answer["ok"] != json!(true) {
        bail!("{}", answer["error"]["message"].as_str().unwrap_or("the build was refused"));
    }
    let built = answer["result"]["parts"]
        .as_object()
        .with_context(|| format!("build not finished after {} s", BUILD_TIMEOUT.as_secs()))?;
    let names: Vec<&String> =
        if parts.is_empty() { built.keys().collect() } else { parts.iter().collect() };
    // Parts that declare a colour are drawn in it (AG-PJ-07).
    let colors: std::collections::HashMap<String, [u8; 3]> = client
        .status()?
        .into_iter()
        .filter_map(|p| Some((p.name, qoob3d_core::project::parse_color(p.color.as_deref()?)?)))
        .collect();
    let mut meshes = Vec::new();
    for name in names {
        let part = &built[name.as_str()];
        if part["state"] == "failed" {
            let message = part["error"]["message"].as_str().unwrap_or_default();
            bail!("part {name} failed to build: {message}");
        }
        let color = colors.get(name.as_str()).copied();
        meshes.push(client.mesh(name)?.mesh.tinted(color));
    }
    if meshes.is_empty() {
        bail!("no part of {} to render", project.dir().display());
    }
    Ok(meshes)
}
