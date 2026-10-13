//! QOOB3D developer tooling (ADR-0013).
//!
//! ```text
//! cargo xtask check [--fast]   definition of done
//! cargo xtask arch             crate dependency rules (ADR-0003)
//! cargo xtask docs [--fix]     documentation lint, regenerates the ADR index (ADR-0001)
//! ```

mod arch;
mod docs;

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
        _ => {
            eprintln!("usage: cargo xtask <check [--fast] | arch | docs [--fix]>");
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
