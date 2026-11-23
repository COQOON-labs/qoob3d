//! QOOB3D developer tooling (ADR-0013).
//!
//! ```text
//! cargo xtask check [--fast]   definition of done
//! cargo xtask arch             crate dependency rules (ADR-0036)
//! cargo xtask docs [--fix]     documentation lint, regenerates the ADR index (ADR-0001)
//! cargo xtask screenshot [dir] UI screenshot + scene renders for visual review (ADR-0018)
//! cargo xtask i18n             translation keys complete and used keys defined (ADR-0020)
//! cargo xtask printers <dir>   regenerate the printer database from OrcaSlicer (ADR-0021)
//! cargo xtask models           reference project: build, check, export (ADR-0033)
//! cargo xtask hygiene          source rules beyond clippy (ADR-0037)
//! cargo xtask commits <revs>   commit policy for a revision range (ADR-0037)
//! cargo xtask commits --message <file>  same for one message (commit-msg hook)
//! cargo xtask setup            install the git hooks (ADR-0037)
//! cargo xtask selftest         the harness rejects every sabotage in harness/sabotage/ (ADR-0037)
//! cargo xtask harness-changes <base> <head>  harness files changed in between, for CI (ADR-0037)
//! ```

mod arch;
mod commits;
mod docs;
mod hygiene;
mod i18n;
mod models;
mod paths;
mod printers;
mod selftest;

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
        Some("hygiene") => report("hygiene", hygiene::check(&root)),
        Some("commits") => commits_step(&root, &args[1..]),
        Some("setup") => setup(&root),
        Some("harness-changes") => {
            if let [base, head] = &args[1..] {
                harness_changes(&root, base, head)
            } else {
                eprintln!("usage: cargo xtask harness-changes <base> <head>");
                false
            }
        }
        Some("selftest") => report("selftest", selftest::run(&root)),
        Some("models") => models_step(&root),
        Some("printers") => {
            if let Some(dir) = args.get(1) {
                report("printers", printers::generate(&root, Path::new(dir)))
            } else {
                eprintln!("usage: cargo xtask printers <OrcaSlicer resources/profiles dir>");
                false
            }
        }
        _ => {
            eprintln!(
                "usage: cargo xtask <check [--fast] | arch | docs [--fix] | i18n | hygiene | selftest | commits <revs> | harness-changes <base> <head> | setup | models | printers <dir> | screenshot [dir]>"
            );
            return ExitCode::from(2);
        }
    };
    if result { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace").into()
}

/// The cargo target directory the binaries are built into: `CARGO_TARGET_DIR`
/// (relative to `root`) or `root/target`.
pub(crate) fn cargo_target(root: &Path) -> PathBuf {
    let dir = std::env::var("CARGO_TARGET_DIR").ok().filter(|dir| !dir.is_empty());
    // `join` keeps an absolute path as it is.
    root.join(dir.as_deref().unwrap_or("target"))
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
            eprintln!("{name}: {} problem(s)", problems.len());
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
        ("hooks", Step::Internal(hooks_installed)),
        ("hygiene", Step::Internal(|root| report("hygiene", hygiene::check(root)))),
        ("arch", Step::Internal(|root| report("arch", arch::check(root)))),
        ("docs", Step::Internal(|root| report("docs", docs::check(root, false)))),
        ("i18n", Step::Internal(|root| report("i18n", i18n::check(root)))),
        ("models", Step::Internal(models_step)),
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

    println!("── summary ─────────────────────────────────────────");
    for (name, ok, t) in &summary {
        println!("{} {name:<7} {:>6.1}s", if *ok { "✔" } else { "✘" }, t.as_secs_f32());
    }
    let all_ok = summary.iter().all(|s| s.1);
    println!(
        "{}",
        if all_ok { if fast { "CHECK (fast) GREEN" } else { "CHECK GREEN" } } else { "CHECK RED" }
    );
    all_ok
}

/// `commits --message <file>` checks one message, anything else is a rev range.
fn commits_step(root: &Path, args: &[String]) -> bool {
    match args {
        [flag, file] if flag == "--message" => {
            let mut problems = match std::fs::read_to_string(file) {
                Ok(message) => commits::check_message(&message),
                Err(e) => vec![format!("{file}: {e}")],
            };
            problems.extend(commits::check_current_identity(root));
            report("commits", if problems.is_empty() { Ok(()) } else { Err(problems) })
        }
        [] => {
            eprintln!("usage: cargo xtask commits <rev-list args> | --message <file>");
            false
        }
        revs => report("commits", commits::check_range(root, revs)),
    }
}

/// Prints the harness files changed between `base` and `head`, one per line,
/// for the owner review in CI (ADR-0037). A rename counts as deletion and
/// addition, so moving a file out of the harness is flagged, too; `-z` keeps
/// unusual names unquoted, so they start their line.
fn harness_changes(root: &Path, base: &str, head: &str) -> bool {
    match git(root, &["diff", "--name-only", "--no-renames", "-z", base, head]) {
        Ok(names) => {
            for file in paths::harness_files(&names.replace('\0', "\n")) {
                println!("{file}");
            }
            true
        }
        Err(e) => {
            eprintln!("harness-changes: {e}");
            false
        }
    }
}

const HOOKS_DIR: &str = ".githooks";

/// Points git at the versioned hooks in `.githooks/` (ADR-0037).
fn setup(root: &Path) -> bool {
    let ok = run(root, "git", &["config", "core.hooksPath", HOOKS_DIR]);
    if ok {
        println!("setup: git hooks from {HOOKS_DIR}/ active");
    }
    ok
}

/// Local checkouts must use the versioned git hooks; CI checks the same rules itself.
fn hooks_installed(root: &Path) -> bool {
    if std::env::var_os("CI").is_some() {
        return true;
    }
    let configured = git(root, &["config", "--get", "core.hooksPath"])
        .is_ok_and(|hooks_path| hooks_path.trim() == HOOKS_DIR);
    if !configured {
        eprintln!("hooks: git hooks not active → `cargo xtask setup` (ADR-0037)");
    }
    configured
}

/// Writes `ui.png` (whole app via egui_kittest) and a contact sheet of the
/// reference project into `dir` (default `target/screenshots`).
fn screenshot(root: &Path, dir: Option<PathBuf>) -> bool {
    let dir = dir.unwrap_or_else(|| root.join("target/screenshots"));
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("{}: {err}", dir.display());
        return false;
    }
    let shoot = |test: &str| {
        Command::new("cargo")
            .args(["test", "--quiet", "-p", "qoob3d-app", "--test", test])
            .arg("screenshot_when_requested")
            .current_dir(root)
            .env("QOOB3D_SCREENSHOT_DIR", &dir)
            .status()
            .is_ok_and(|s| s.success())
    };
    let ok = shoot("ui") && run(root, "cargo", &["build", "--quiet", "--package", "qoob3d-app"]);
    if !ok {
        return false;
    }
    // The reference project (ADR-0033) from three sides.
    let out = dir.join("reference.png");
    let args = ["render-project", ".", "--out", &out.to_string_lossy(), "--views", "iso,front,top"];
    let rendered = models::Reference::start(root)
        .map(|reference| reference.command(&args).status().is_ok_and(|s| s.success()));
    match rendered {
        Ok(true) => {}
        Ok(false) => return false,
        Err(e) => {
            eprintln!("screenshot: {e}");
            return false;
        }
    }
    println!("screenshots in {}", dir.display());
    true
}

/// Builds the binary, then checks the reference project through it.
fn models_step(root: &Path) -> bool {
    run(root, "cargo", &["build", "--quiet", "--package", "qoob3d-app"])
        && report("models", models::check(root))
}

fn smoke(root: &Path) -> bool {
    run(root, "cargo", &["build", "--quiet", "--package", "qoob3d-app"])
        && run(root, &cargo_target(root).join("debug/qoob3d").to_string_lossy(), &["--smoke-test"])
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
            eprintln!("could not start `{program}`: {err}");
            if program == "cargo" && args.first() == Some(&"deny") {
                eprintln!("→ install with: brew install cargo-deny");
            }
            false
        }
    }
}

/// Output of a git command in `root`, or its error message.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_command(root, args).output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Variables a git hook exports; they would point git at another repository,
/// work tree or index than the one the command runs in.
pub(crate) const GIT_ENV: &[&str] = &["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// `git` with `args` in `root`, without the hook's [`GIT_ENV`]: xtask always
/// means the repository at `root`.
pub(crate) fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.args(args).current_dir(root);
    for key in GIT_ENV {
        command.env_remove(key);
    }
    command
}

#[cfg(test)]
mod tests;
