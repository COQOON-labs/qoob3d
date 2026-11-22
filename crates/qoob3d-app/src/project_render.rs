//! `qoob3d render-project` (AG-RD-01, ADR-0054): what an agent looks at.
//! The service (started on demand) builds the selected parts if they are
//! stale; their cached meshes are rendered headless, orthographic, with the
//! camera fitted to them.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, bail};
use qoob3d_core::ShapeMesh;
use qoob3d_render::ViewPreset;
use serde_json::{Value, json};

use crate::headless_render::{self, RenderView};
use crate::live::{BUILD_TIMEOUT, Client, ProjectDir, START_TIMEOUT, service_command_from_env};

/// Margin over the build wait for the answer of the build request.
const ANSWER_MARGIN: Duration = Duration::from_secs(30);

/// Renders `parts` (all if empty) of the project in `dir` into the PNG `out`.
///
/// # Errors
/// No project, no service, an unknown or failed part (named), nothing to
/// render, or the render itself fails.
pub fn run(
    dir: &Path,
    out: &Path,
    view: ViewPreset,
    size: [u32; 2],
    parts: &[String],
) -> anyhow::Result<()> {
    let project = ProjectDir::open(dir)?;
    let client = Client::connect(&project, &service_command_from_env(), START_TIMEOUT)?;
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
    let mut meshes = Vec::new();
    for name in names {
        let part = &built[name.as_str()];
        if part["state"] == "failed" {
            let message = part["error"]["message"].as_str().unwrap_or_default();
            bail!("part {name} failed to build: {message}");
        }
        meshes.push(client.mesh(name)?.mesh);
    }
    if meshes.is_empty() {
        bail!("no part of {} to render", project.dir().display());
    }
    let view = RenderView {
        preset: view,
        orthographic: true,
        zoom: 1.0,
        printer: None,
        build_volume: false,
    };
    headless_render::run(out, view, size, Some(ShapeMesh::merge(&meshes)))
}
