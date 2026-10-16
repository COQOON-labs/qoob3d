//! QOOB3D developer tooling (ADR-0013).
//!
//! ```text
//! cargo xtask check [--fast]   definition of done
//! cargo xtask check-rev <rev>  the full check of a commit in a temporary worktree (ADR-0048)
//! cargo xtask arch             crate dependency rules (ADR-0036)
//! cargo xtask docs [--fix]     documentation lint, regenerates the ADR index (ADR-0001)
//! cargo xtask screenshot [dir] UI screenshot + scene renders for visual review (ADR-0018)
//! cargo xtask i18n             translation keys complete and used keys defined (ADR-0020)
//! cargo xtask printers <dir>   regenerate the printer database from OrcaSlicer (ADR-0021)
//! cargo xtask models [--write] reference models: build, measure, export (ADR-0033)
//! cargo xtask hygiene          source rules beyond clippy (ADR-0039, ADR-0040, ADR-0044, ADR-0045)
//! cargo xtask commits <revs>   commit policy for a revision range (ADR-0038, ADR-0039, ADR-0044)
//! cargo xtask commits --message <file>  same for one message (commit-msg hook)
//! cargo xtask setup            install the git hooks (ADR-0037)
//! cargo xtask review <tip> <revs>  critic record for pushed code (ADR-0041, ADR-0052)
//! cargo xtask review --plan <tip> <revs>  the review plan of a pushed range (ADR-0052)
//! cargo xtask rules            every rule in harness/rules.md has existing checks (ADR-0040)
//! cargo xtask selftest         the harness rejects every sabotage in harness/sabotage/ (ADR-0042)
//! cargo xtask guard            Claude Code PreToolUse hook: JSON on stdin, exit 2 = deny (ADR-0043)
//! cargo xtask harness-changes <base> <head>  harness files changed in between, for CI (ADR-0037)
//! ```

mod arch;
mod commits;
mod docs;
mod guard;
mod hygiene;
mod i18n;
mod models;
mod paths;
mod printers;
mod review;
mod rules;
mod selftest;
mod worktree;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let root = workspace_root();

    let result = match args.first().map(String::as_str) {
        Some("check") => check(&root, flag("--fast")),
        Some("check-rev") => {
            if let Some(rev) = args.get(1) {
                check_rev(&root, rev)
            } else {
                eprintln!("usage: cargo xtask check-rev <rev>");
                false
            }
        }
        Some("arch") => report("arch", arch::check(&root)),
        Some("docs") => report("docs", docs::check(&root, flag("--fix"))),
        Some("screenshot") => screenshot(&root, args.get(1).map(PathBuf::from)),
        Some("i18n") => report("i18n", i18n::check(&root)),
        Some("hygiene") => report("hygiene", hygiene::check(&root)),
        Some("commits") => commits_step(&root, &args[1..]),
        Some("setup") => setup(&root),
        Some("guard") => return guard_hook(&root),
        Some("harness-changes") => {
            if let [base, head] = &args[1..] {
                harness_changes(&root, base, head)
            } else {
                eprintln!("usage: cargo xtask harness-changes <base> <head>");
                false
            }
        }
        Some("rules") => report("rules", rules::check(&root)),
        Some("selftest") => report("selftest", selftest::run(&root)),
        Some("review") => match review::parse_args(&args[1..]) {
            Some(review::Invocation::Plan { tip, revs }) => {
                match review::plan_range(&root, tip, revs) {
                    Ok(plan) => {
                        print!("{plan}");
                        true
                    }
                    Err(problems) => report("review --plan", Err(problems)),
                }
            }
            Some(review::Invocation::Gate { tip, revs }) => {
                report("review", review::check(&root, tip, revs))
            }
            None => {
                eprintln!("usage: cargo xtask review [--plan] <tip> <rev-list args>");
                false
            }
        },
        Some("models") if flag("--write") => report("models", models::write(&root)),
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
                "usage: cargo xtask <check [--fast] | check-rev <rev> | arch | docs [--fix] | i18n | hygiene | rules | selftest | commits <revs> | review [--plan] <tip> <revs> | guard | harness-changes <base> <head> | setup | models [--write] | screenshot [dir]>"
            );
            return ExitCode::from(2);
        }
    };
    if result { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace").into()
}

/// The cargo target directory the binaries are built into, from
/// `CARGO_TARGET_DIR`; in a `check-rev` worktree that is the check-rev
/// directory (ADR-0048).
pub(crate) fn cargo_target(root: &Path) -> PathBuf {
    worktree::target_dir(root, std::env::var("CARGO_TARGET_DIR").ok().as_deref())
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
        ("rules", Step::Internal(|root| report("rules", rules::check(root)))),
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

/// The full check of the committed state of `rev` in a temporary worktree,
/// independent of the working tree (ADR-0048).
fn check_rev(root: &Path, rev: &str) -> bool {
    let cargo_target_dir = std::env::var("CARGO_TARGET_DIR").ok();
    let target = worktree::check_rev_target_dir(root, cargo_target_dir.as_deref());
    let result = worktree::with_worktree(root, rev, |dir| {
        println!("check-rev: checking {rev} in {}", dir.display());
        match worktree::check_command(dir, &target).status() {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("check of {rev} in {} failed ({status})", dir.display())),
            Err(e) => Err(format!("could not start cargo in {}: {e}", dir.display())),
        }
    });
    match result.flatten() {
        Ok(()) => {
            println!("check-rev: {rev} green");
            true
        }
        Err(e) => {
            eprintln!("check-rev: {e}");
            false
        }
    }
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

/// Exit code 2 tells Claude Code to block the tool call and show stderr.
fn guard_hook(root: &Path) -> ExitCode {
    let env = |key: &str| std::env::var(key).ok();
    let session = guard::Session::from_env_values(
        env("QOOB3D_HARNESS_UNLOCK").as_deref(),
        env("HOME").as_deref(),
    );
    let input = std::io::read_to_string(std::io::stdin())
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()));
    match input
        .map_err(|e| format!("guard: unreadable hook input: {e}"))
        .and_then(|v| guard::pre_tool_use(root, &session, &v))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            eprintln!("{reason}");
            ExitCode::from(2)
        }
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

/// Writes `ui.png` (whole app via egui_kittest) and scene renders of the
/// standard views into `dir` (default `target/screenshots`).
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
    let ok = shoot("ui")
        && shoot("modeling")
        && run(root, "cargo", &["build", "--quiet", "--package", "qoob3d-app"]);
    if !ok {
        return false;
    }
    let binary = cargo_target(root).join("debug/qoob3d");
    for view in ["iso", "front", "top"] {
        let out = dir.join(format!("scene-{view}.png"));
        let args = ["--render", &out.to_string_lossy(), "--view", view, "--size", "1200x800"];
        if !run(root, &binary.to_string_lossy(), &args) {
            return false;
        }
    }
    // Every reference model (ADR-0033).
    let models = root.join("models");
    let mut names: Vec<String> = std::fs::read_dir(&models)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".qbd").map(str::to_string))
        .collect();
    names.sort();
    for name in names {
        let file = models.join(format!("{name}.qbd"));
        let out = dir.join(format!("model-{name}.png"));
        let args =
            [&*file.to_string_lossy(), "--render", &out.to_string_lossy(), "--size", "900x600"];
        if !run(root, &binary.to_string_lossy(), &args) {
            return false;
        }
    }
    println!("screenshots in {}", dir.display());
    true
}

/// Builds the binary, then checks every reference model through it.
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
/// work tree or index than the one the command runs in (ADR-0048).
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
