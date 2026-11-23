//! The reference project as a quality gate (ADR-0013).
//!
//! `models/` is a script project whose checks compare every part with a
//! formula. The gate proves the path "design → slicer", headless through the
//! real binary, on a fresh copy with an empty cache:
//! - every part builds, its shape is valid and its mesh watertight,
//! - every check of the project passes,
//! - the 3MF and STL exports are readable, also for Info-ZIP and, if it is
//!   installed, for Bambu Studio.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use qoob3d_core::project::layout;
use serde_json::Value;

/// The reference project, relative to the repository root.
const PROJECT: &str = "models";
/// How long the service may take to come up.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// A copy of the reference project with its own service of the binary under
/// test. Dropping it stops the service; its workers end with it.
pub(crate) struct Reference {
    binary: PathBuf,
    dir: PathBuf,
    service: Child,
}

impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.service.kill();
        let _ = self.service.wait();
    }
}

impl Reference {
    /// Copies `models/` to `target/models/project` (without cache and
    /// exports) and starts `qoob3d serve` for the copy.
    pub(crate) fn start(root: &Path) -> Result<Self, String> {
        let binary = crate::cargo_target(root).join("debug/qoob3d");
        let dir = crate::cargo_target(root).join("models/project");
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        copy_project(&root.join(PROJECT), &dir)?;
        let mut service = Command::new(&binary)
            .arg("serve")
            .arg(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // Its stderr stays ours: only what a failing gate needs to show.
            .env("RUST_LOG", "warn")
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
        let ready = layout::service_json(&dir);
        let deadline = Instant::now() + START_TIMEOUT;
        while !ready.exists() {
            let exited = service.try_wait().ok().flatten();
            if exited.is_some() || Instant::now() > deadline {
                let _ = service.kill();
                let _ = service.wait();
                return Err(match exited {
                    Some(status) => format!("the service ended at once ({status})"),
                    None => format!("the service did not start within {START_TIMEOUT:?}"),
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(Self { binary, dir, service })
    }

    /// `qoob3d <args>` with the project as working directory.
    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .current_dir(&self.dir)
            .env("QOOB3D_SERVICE_CMD", &self.binary)
            .env("RUST_LOG", "warn");
        command
    }

    /// `qoob3d <args>` in the project: the `result` of its JSON envelope.
    fn call(&self, args: &[&str]) -> Result<Value, String> {
        let output = self
            .command(args)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.binary.display()))?;
        let envelope: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
            format!(
                "`qoob3d {}` printed no JSON ({e}); stderr: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
        if envelope["ok"] == Value::Bool(true) {
            Ok(envelope["result"].clone())
        } else {
            Err(format!("`qoob3d {}` failed: {}", args.join(" "), envelope["error"]["message"]))
        }
    }
}

/// Copies the project files; cache and exports of a local run stay behind.
fn copy_project(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let (source, name) = (entry.path(), entry.file_name());
        if source.is_dir() {
            if name != layout::DATA_DIR && name != layout::OUT_DIR {
                copy_project(&source, &to.join(&name))?;
            }
        } else {
            std::fs::copy(&source, to.join(&name))
                .map_err(|e| format!("{}: {e}", source.display()))?;
        }
    }
    Ok(())
}

pub(crate) fn check(root: &Path) -> Result<(), Vec<String>> {
    let reference = Reference::start(root).map_err(|e| vec![e])?;
    let mut problems = Vec::new();

    // 1. Every part builds into a valid shape with a closed mesh.
    let build = reference.call(&["build"]).map_err(|e| vec![e])?;
    let parts = build["parts"].as_object().cloned().unwrap_or_default();
    if parts.is_empty() {
        problems.push(format!("{PROJECT}/ declares no parts"));
    }
    for (name, part) in &parts {
        if part["state"] == "failed" {
            problems.push(format!("{name}: {}", part["error"]["message"]));
        } else if part["valid"] != Value::Bool(true) || part["watertight"] != Value::Bool(true) {
            problems.push(format!(
                "{name}: not printable (valid {}, watertight {})",
                part["valid"], part["watertight"]
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    // 2. The analytic expectations of the project.
    let checks = reference.call(&["check"]).map_err(|e| vec![e])?;
    let checks = checks["checks"].as_array().cloned().unwrap_or_default();
    if checks.is_empty() {
        problems.push(format!("{PROJECT}/ declares no checks"));
    }
    for check in &checks {
        if check["state"] == "passed" {
            continue;
        }
        let name = check["name"].as_str().unwrap_or("?");
        if let Some(message) = check["error"]["message"].as_str() {
            problems.push(format!("check {name}: {message}"));
        }
        for e in check["expectations"].as_array().into_iter().flatten() {
            if e["passed"] != Value::Bool(true) {
                problems.push(format!(
                    "check {name}: {} – actual {}, expected {}",
                    e["text"].as_str().unwrap_or("?"),
                    e["actual"],
                    e["expected"]
                ));
            }
        }
    }

    // 3. The exports a slicer gets.
    let names: Vec<&str> = parts.keys().map(String::as_str).collect();
    for format in ["3mf", "stl"] {
        let mut args = vec!["export"];
        args.extend(&names);
        args.extend(["--format", format]);
        if let Err(e) = reference.call(&args) {
            problems.push(e);
        }
    }
    for (name, part) in &parts {
        let volume = part["volume"].as_f64().unwrap_or(f64::NAN);
        let found = check_exports(&reference.dir.join(layout::OUT_DIR), name, volume);
        if found.is_empty() {
            println!("models: ok    {name}");
        }
        problems.extend(found.into_iter().map(|p| format!("{name}: {p}")));
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// The 3MF holds one readable object, the binary STL has a plausible size.
fn check_exports(out: &Path, name: &str, volume: f64) -> Vec<String> {
    let three = out.join(format!("{name}.3mf"));
    let mut problems = Vec::new();
    match std::fs::read(&three).map_err(|e| e.to_string()).and_then(|b| {
        qoob3d_core::export::unzip_stored(&b).map_err(|e| format!("3MF unreadable: {e}"))
    }) {
        Ok(files) => {
            let model = files
                .iter()
                .find(|(n, _)| n == "3D/3dmodel.model")
                .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
                .unwrap_or_default();
            if model.matches("<object ").count() != 1 {
                problems.push("the 3MF does not hold exactly one object".into());
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
    // The real consumer: Bambu Studio's own loader, if installed.
    if let Some(slicer) = bambu_studio() {
        problems.extend(check_in_slicer(&slicer, &three, volume));
    }
    let size = std::fs::metadata(out.join(format!("{name}.stl"))).map_or(0, |m| m.len());
    if size <= 84 || !(size - 84).is_multiple_of(50) {
        problems.push(format!("the STL has an invalid size ({size} bytes)"));
    }
    problems
}

/// Bambu Studio's CLI (`QOOB3D_BAMBU_STUDIO` or the default macOS location).
fn bambu_studio() -> Option<PathBuf> {
    let path = std::env::var_os("QOOB3D_BAMBU_STUDIO").map_or_else(
        || PathBuf::from("/Applications/BambuStudio.app/Contents/MacOS/BambuStudio"),
        Into::into,
    );
    path.exists().then_some(path)
}

/// `BambuStudio --info file.3mf` must load the one object, manifold, with our volume.
fn check_in_slicer(slicer: &Path, file: &Path, volume: f64) -> Vec<String> {
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
    if objects != 1 {
        problems.push(format!("Bambu Studio sees {objects} objects, expected 1"));
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
