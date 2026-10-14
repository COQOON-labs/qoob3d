//! Crate dependency rules from ADR-0036, checked against `cargo metadata`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const WORKSPACE_CRATES: &[&str] =
    &["qoob3d-core", "qoob3d-sketch", "qoob3d-kernel", "qoob3d-render", "qoob3d-app", "xtask"];

/// Allowed direct dependencies on other workspace crates.
fn allowed_workspace_deps(krate: &str) -> &'static [&'static str] {
    match krate {
        "qoob3d-sketch" | "qoob3d-kernel" | "qoob3d-render" => &["qoob3d-core"],
        "qoob3d-app" => &["qoob3d-core", "qoob3d-sketch", "qoob3d-kernel", "qoob3d-render"],
        // Gates read models and exports; geometry is checked via the binary.
        "xtask" => &["qoob3d-core"],
        _ => &[],
    }
}

/// Packages that must not appear in the (non-dev) transitive dependency tree.
fn forbidden_transitive(krate: &str) -> &'static [&'static str] {
    const OCCT: [&str; 3] = ["opencascade", "opencascade-sys", "occt-sys"];
    const GPU_UI: [&str; 5] = ["wgpu", "eframe", "egui", "egui-wgpu", "winit"];
    match krate {
        // Pure crates: no kernel, GPU or UI, not even transitively.
        "qoob3d-core" | "qoob3d-sketch" => &[
            "opencascade",
            "opencascade-sys",
            "occt-sys",
            "wgpu",
            "eframe",
            "egui",
            "egui-wgpu",
            "winit",
        ],
        "qoob3d-kernel" => &GPU_UI,
        "qoob3d-render" => &OCCT,
        _ => &[],
    }
}

pub(crate) fn check(root: &Path) -> Result<(), Vec<String>> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| vec![format!("cargo metadata: {e}")])?;
    if !output.status.success() {
        return Err(vec![String::from_utf8_lossy(&output.stderr).into_owned()]);
    }
    let meta: Value = serde_json::from_slice(&output.stdout).map_err(|e| vec![e.to_string()])?;
    let graph = Graph::from_metadata(&meta).map_err(|e| vec![e])?;

    let mut problems = Vec::new();
    for &krate in WORKSPACE_CRATES {
        let Some(direct) = graph.edges.get(krate) else {
            problems.push(format!("{krate}: not found in the workspace"));
            continue;
        };
        for dep in direct.iter().filter(|d| WORKSPACE_CRATES.contains(&d.as_str())) {
            if !allowed_workspace_deps(krate).contains(&dep.as_str()) {
                problems.push(format!(
                    "{krate} → {dep} is not allowed (ADR-0036; allowed: {:?})",
                    allowed_workspace_deps(krate)
                ));
            }
        }
        let reachable = graph.transitive(krate);
        for &bad in forbidden_transitive(krate) {
            if reachable.contains(bad) {
                problems.push(format!(
                    "{krate} pulls in `{bad}` (transitively) – forbidden by ADR-0036. Path: {}",
                    graph.path(krate, bad).join(" → ")
                ));
            }
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Package-name graph of normal + build dependencies (dev-dependencies ignored).
struct Graph {
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    fn from_metadata(meta: &Value) -> Result<Self, String> {
        let names: BTreeMap<&str, &str> = meta["packages"]
            .as_array()
            .ok_or("metadata: packages is missing")?
            .iter()
            .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
            .collect();
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for node in meta["resolve"]["nodes"].as_array().ok_or("metadata: resolve is missing")? {
            let from = names[node["id"].as_str().ok_or("node id")?].to_string();
            let entry = edges.entry(from).or_default();
            for dep in node["deps"].as_array().into_iter().flatten() {
                let non_dev = dep["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| k["kind"].as_str() != Some("dev"));
                if non_dev && let Some(name) = dep["pkg"].as_str().and_then(|id| names.get(id)) {
                    entry.insert((*name).to_string());
                }
            }
        }
        Ok(Self { edges })
    }

    fn transitive(&self, start: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(n) = stack.pop() {
            for d in self.edges.get(&n).into_iter().flatten() {
                if seen.insert(d.clone()) {
                    stack.push(d.clone());
                }
            }
        }
        seen
    }

    /// Breadth-first path from `from` to `to`, for readable error messages.
    fn path(&self, from: &str, to: &str) -> Vec<String> {
        let mut prev: BTreeMap<String, String> = BTreeMap::new();
        let mut queue = std::collections::VecDeque::from([from.to_string()]);
        while let Some(n) = queue.pop_front() {
            if n == to {
                let mut path = vec![n];
                while let Some(p) = prev.get(path.last().expect("non-empty")) {
                    path.push(p.clone());
                }
                path.reverse();
                return path;
            }
            for d in self.edges.get(&n).into_iter().flatten() {
                if d != from && !prev.contains_key(d) {
                    prev.insert(d.clone(), n.clone());
                    queue.push_back(d.clone());
                }
            }
        }
        vec![from.into(), "…".into(), to.into()]
    }
}
