//! Acceptance against real projects (docs/requirements/agent.md §7, "Real
//! model"): opt-in, because real models are large and belong to their
//! owners, not to this repository.
//!
//! `QOOB3D_ACCEPTANCE_PROJECTS` lists project folders, separated by `:`.
//! Each is copied without `.qoob3d/` and `out/` into a temporary folder and
//! must, from an empty cache:
//! - build every part, and every part must be a valid B-rep;
//! - give every part with a print pose (`pose` option) a watertight mesh;
//! - pass every check;
//! - export the parts with a print pose as one 3MF kit whose objects are all
//!   watertight, and which Bambu Studio (`QOOB3D_BAMBU_STUDIO` or the macOS
//!   app, if installed) loads as manifold.
//!
//! Without the variable the test passes at once and says so on stderr.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_qoob3d");
const BAMBU_MAC: &str = "/Applications/BambuStudio.app/Contents/MacOS/BambuStudio";

/// Copies `from` to `to`, leaving out service data and exports.
fn copy_project(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create the copy");
    for entry in std::fs::read_dir(from).expect("read the project").flatten() {
        let name = entry.file_name();
        if name == ".qoob3d" || name == "out" || name == ".git" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            copy_project(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy a file");
        }
    }
}

/// `qoob3d ARGS` in `dir`: exit code and the JSON envelope on stdout.
fn qoob3d(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(BIN).args(args).current_dir(dir).output().expect("run qoob3d");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let envelope = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "qoob3d {args:?}: no envelope ({e}):\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap_or(-1), envelope)
}

fn bambu_studio() -> Option<PathBuf> {
    std::env::var_os("QOOB3D_BAMBU_STUDIO")
        .map(PathBuf::from)
        .or_else(|| Path::new(BAMBU_MAC).exists().then(|| PathBuf::from(BAMBU_MAC)))
}

fn accept(original: &Path) {
    let name = original.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy());
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("acceptance").join(name.as_ref());
    let _ = std::fs::remove_dir_all(&dir);
    copy_project(original, &dir);

    let (code, build) = qoob3d(&dir, &["build", "--wait", "3600"]);
    assert_eq!(code, 0, "{}: the build failed: {build}", original.display());
    let parts = build["result"]["parts"].as_object().expect("build result per part");
    for (part, result) in parts {
        assert_eq!(result["valid"], Value::Bool(true), "{part} is not a valid B-rep: {result}");
    }
    let (_, status) = qoob3d(&dir, &["status"]);
    let printed: Vec<&str> = status["result"]["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .filter(|p| !p["pose"].is_null())
        .filter_map(|p| p["name"].as_str())
        .collect();
    for part in &printed {
        let result = &parts[*part];
        assert_eq!(result["watertight"], Value::Bool(true), "{part} prints but leaks: {result}");
    }

    let (code, checks) = qoob3d(&dir, &["check"]);
    assert_eq!(code, 0, "{}: checks failed: {checks}", original.display());

    if printed.is_empty() {
        eprintln!("{}: no part declares a print pose, no kit to export", original.display());
        return;
    }
    let mut args = vec!["export"];
    args.extend(printed.iter().copied());
    args.extend(["--format", "3mf", "--kit", "acceptance"]);
    let (code, export) = qoob3d(&dir, &args);
    assert_eq!(code, 0, "the kit export failed: {export}");
    for object in export["result"]["details"][0]["objects"].as_array().expect("kit objects") {
        assert_eq!(object["watertight"], Value::Bool(true), "kit object leaks: {object}");
    }
    let kit = dir.join("out/acceptance.3mf");
    let Some(bambu) = bambu_studio() else {
        eprintln!("Bambu Studio not found: the kit is not loaded in a slicer");
        return;
    };
    let info = Command::new(bambu).arg("--info").arg(&kit).output().expect("run Bambu Studio");
    let text = String::from_utf8_lossy(&info.stdout);
    let manifold: Vec<&str> =
        text.lines().filter(|l| l.trim_start().starts_with("manifold")).collect();
    assert!(!manifold.is_empty(), "Bambu Studio printed no manifold line:\n{text}");
    assert!(manifold.iter().all(|l| l.ends_with("yes")), "Bambu Studio: {manifold:?}\n{text}");
}

#[test]
fn real_projects_build_check_and_export_cleanly() {
    let Some(list) = std::env::var_os("QOOB3D_ACCEPTANCE_PROJECTS") else {
        eprintln!("QOOB3D_ACCEPTANCE_PROJECTS is not set: no real project to accept");
        return;
    };
    for dir in std::env::split_paths(&list) {
        accept(&dir);
    }
}
