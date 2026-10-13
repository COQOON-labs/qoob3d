//! QOOB3D developer tooling (ADR-0013).
//!
//! ```text
//! cargo xtask check [--fast]   definition of done
//! cargo xtask arch             crate dependency rules (ADR-0003)
//! cargo xtask docs [--fix]     documentation lint, regenerates the ADR index (ADR-0001)
//! cargo xtask screenshot [dir] UI screenshot + scene renders for visual review (ADR-0018)
//! cargo xtask i18n             translation keys complete and used keys defined (ADR-0020)
//! cargo xtask printers <dir>   regenerate the printer database from OrcaSlicer (ADR-0021)
//! ```

mod arch;
mod docs;
mod i18n;
mod printers;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let root = workspace_root();

    let result = match args.first().map(String::as_str) {
        Some("check") => check(&root, flag("--fast")),
        Some("arch") => report("arch", arch::check(&root)),
        Some("docs") => report("docs", docs::check(&root, flag("--fix"))),
        Some("screenshot") => screenshot(&root, args.get(1).map(PathBuf::from)),
        Some("i18n") => report("i18n", i18n::check(&root)),
        Some("printers") => match args.get(1) {
            Some(dir) => report("printers", printers::generate(&root, Path::new(dir))),
            None => {
                eprintln!("usage: cargo xtask printers <OrcaSlicer resources/profiles dir>");
                false
            }
        },
        _ => {
            eprintln!(
                "usage: cargo xtask <check [--fast] | arch | docs [--fix] | screenshot [dir]>"
            );
            return ExitCode::from(2);
        }
    };
    if result { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace").into()
}

/// Prints a list of rule violations; returns true if there were none.
fn report(name: &str, result: Result<(), Vec<String>>) -> bool {
    match result {
        Ok(()) => {
            println!("{name}: ok");
            true
        }
        Err(problems) => {
            for p in &problems {
                eprintln!("{name}: {p}");
            }
            eprintln!("{name}: {} Problem(e)", problems.len());
            false
        }
    }
}

enum Step {
    Cargo(&'static [&'static str]),
    Internal(fn(&Path) -> bool),
}

fn check(root: &Path, fast: bool) -> bool {
    let mut steps: Vec<(&str, Step)> = vec![
        ("fmt", Step::Cargo(&["fmt", "--all", "--check"])),
        (
            "clippy",
            Step::Cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
        ),
        ("test", Step::Cargo(&["test", "--workspace", "--quiet"])),
        ("arch", Step::Internal(|root| report("arch", arch::check(root)))),
        ("docs", Step::Internal(|root| report("docs", docs::check(root, false)))),
        ("i18n", Step::Internal(|root| report("i18n", i18n::check(root)))),
    ];
    if !fast {
        steps.push(("deny", Step::Cargo(&["deny", "--log-level", "error", "check"])));
        steps.push(("smoke", Step::Internal(smoke)));
    }

    let mut summary = Vec::new();
    for (name, step) in steps {
        println!("── {name} ──────────────────────────────────────────");
        let started = Instant::now();
        let ok = match step {
            Step::Cargo(args) => run(root, "cargo", args),
            Step::Internal(f) => f(root),
        };
        summary.push((name, ok, started.elapsed()));
        if !ok {
            break; // later steps are meaningless on a red build
        }
    }

    println!("── Zusammenfassung ─────────────────────────────────");
    for (name, ok, t) in &summary {
        println!("{} {name:<7} {:>6.1}s", if *ok { "✔" } else { "✘" }, t.as_secs_f32());
    }
    let all_ok = summary.iter().all(|s| s.1);
    println!(
        "{}",
        if all_ok { if fast { "CHECK (fast) GRÜN" } else { "CHECK GRÜN" } } else { "CHECK ROT" }
    );
    all_ok
}

/// Writes `ui.png` (whole app via egui_kittest) and scene renders of the
/// standard views into `dir` (default `target/screenshots`).
fn screenshot(root: &Path, dir: Option<PathBuf>) -> bool {
    let dir = dir.unwrap_or_else(|| root.join("target/screenshots"));
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("{}: {err}", dir.display());
        return false;
    }
    let ok = Command::new("cargo")
        .args(["test", "--quiet", "-p", "qoob3d-app", "--test", "ui", "screenshot_when_requested"])
        .current_dir(root)
        .env("QOOB3D_SCREENSHOT_DIR", &dir)
        .status()
        .is_ok_and(|s| s.success())
        && run(root, "cargo", &["build", "--quiet", "--package", "qoob3d-app"]);
    if !ok {
        return false;
    }
    let binary = root.join("target/debug/qoob3d");
    for view in ["iso", "front", "top"] {
        let out = dir.join(format!("scene-{view}.png"));
        let args = ["--render", &out.to_string_lossy(), "--view", view, "--size", "1200x800"];
        if !run(root, &binary.to_string_lossy(), &args) {
            return false;
        }
    }
    println!("screenshots in {}", dir.display());
    true
}

fn smoke(root: &Path) -> bool {
    run(root, "cargo", &["build", "--quiet", "--package", "qoob3d-app"])
        && run(root, &root.join("target/debug/qoob3d").to_string_lossy(), &["--smoke-test"])
}

fn run(root: &Path, program: &str, args: &[&str]) -> bool {
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        // Tests that need a GPU must fail instead of skipping inside the check.
        .env("QOOB3D_REQUIRE_GPU", "1")
        .status();
    match status {
        Ok(s) => s.success(),
        Err(err) => {
            eprintln!("konnte `{program}` nicht starten: {err}");
            if program == "cargo" && args.first() == Some(&"deny") {
                eprintln!("→ installieren mit: brew install cargo-deny");
            }
            false
        }
    }
}
