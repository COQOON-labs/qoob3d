//! Harness self-test (ADR-0042): every case in `harness/sabotage/*.case` is a
//! deliberate violation that the harness must reject. Each case runs in a
//! fresh copy of the working tree (its own git repository), so the real
//! checkout is never touched.
//!
//! Case file:
//! ```text
//! # what the sabotage does
//! command: cargo xtask hygiene          (run in the copy, must fail)
//! expect: text in the command output    (proves it failed for the right reason)
//! message: commit message, `\n` = newline (optional, default "sabotage")
//! stdin: one line passed on stdin       (optional)
//! env: KEY=VALUE                        (optional, may repeat; environment of the command)
//! identity: Name <mail>                 (optional; author and committer of the sabotage
//!                                        commit, default from harness/identity.txt)
//! --- patch
//! <unified diff, applied with git apply>
//! ```

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) const CASES: &str = "harness/sabotage";

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Case {
    pub(crate) command: String,
    pub(crate) expect: String,
    pub(crate) message: String,
    pub(crate) stdin: Option<String>,
    pub(crate) patch: Option<String>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) identity: Option<String>,
}

pub(crate) fn parse(text: &str) -> Result<Case, String> {
    let (head, patch) = match text.split_once("--- patch\n") {
        Some((head, patch)) => (head, Some(patch.to_string())),
        None => (text, None),
    };
    let mut case = Case { message: "sabotage".into(), patch, ..Case::default() };
    for line in head.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (key, value) = line.split_once(':').ok_or(format!("line without `key:` – {line}"))?;
        let value = value.trim().to_string();
        match key.trim() {
            "command" => case.command = value,
            "expect" => case.expect = value,
            "message" => case.message = value.replace("\\n", "\n"),
            "stdin" => case.stdin = Some(value),
            "env" => {
                let (k, v) =
                    value.split_once('=').ok_or(format!("`env` needs KEY=VALUE – {line}"))?;
                case.env.push((k.to_string(), v.to_string()));
            }
            "identity" => case.identity = Some(value),
            other => return Err(format!("unknown key `{other}`")),
        }
    }
    if case.command.is_empty() || case.expect.is_empty() {
        return Err("`command` and `expect` are required".into());
    }
    Ok(case)
}

pub(crate) fn run(root: &Path) -> Result<(), Vec<String>> {
    let mut files: Vec<PathBuf> = fs::read_dir(root.join(CASES))
        .map_err(|e| vec![format!("{CASES}: {e}")])?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "case"))
        .collect();
    files.sort();
    let target = root.join("target/selftest");
    let work = target.join("work");
    let identity = crate::commits::read_allowed_identity(root)
        .unwrap_or_else(|_| "selftest <selftest@localhost>".into());
    prepare(root, &work, &identity).map_err(|e| vec![e])?;
    let mut problems = Vec::new();
    for file in &files {
        let name = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let outcome = fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|text| parse(&text))
            .and_then(|case| run_case(&work, &target, &case, &identity));
        match outcome {
            Ok(()) => println!("selftest: ✔ {name} is rejected"),
            Err(why) => problems.push(format!("{name}: {why}")),
        }
    }
    println!("selftest: {} of {} sabotages detected", files.len() - problems.len(), files.len());
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// `git -c …` arguments that make `identity` (`Name <mail>`) author and committer.
fn as_identity(identity: &str) -> [String; 4] {
    let (name, mail) = identity.split_once('<').unwrap_or((identity, ""));
    let mail = mail.trim_end().trim_end_matches('>');
    ["-c".into(), format!("user.name={}", name.trim()), "-c".into(), format!("user.email={mail}")]
}

/// A copy of the working tree as its own repository with one `baseline` commit.
fn prepare(root: &Path, work: &Path, identity: &str) -> Result<(), String> {
    if work.exists() {
        fs::remove_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    }
    copy_tree(root, work)?;
    sh(work, "git", &["init", "-q"], None)?;
    sh(work, "git", &["add", "-A"], None)?;
    let author = as_identity(identity);
    let mut commit: Vec<&str> = author.iter().map(String::as_str).collect();
    commit.extend(["commit", "-q", "--no-verify", "-m", "baseline"]);
    sh(work, "git", &commit, None)?;
    sh(work, "git", &["tag", "baseline"], None).map(|_| ())
}

fn run_case(work: &Path, target: &Path, case: &Case, identity: &str) -> Result<(), String> {
    sh(work, "git", &["reset", "-q", "--hard", "baseline"], None)?;
    sh(work, "git", &["clean", "-qfd"], None)?;
    if let Some(patch) = &case.patch {
        sh(work, "git", &["apply", "-"], Some(patch))
            .map_err(|e| format!("patch no longer applies: {e}"))?;
    }
    sh(work, "git", &["add", "-A"], None)?;
    let author = as_identity(case.identity.as_deref().unwrap_or(identity));
    let mut commit: Vec<&str> = author.iter().map(String::as_str).collect();
    commit.extend(["commit", "-q", "--allow-empty", "--no-verify", "-m", &case.message]);
    sh(work, "git", &commit, None)?;

    let mut parts = case.command.split_whitespace();
    let program = parts.next().unwrap_or_default();
    let args: Vec<&str> = parts.collect();
    let output = Command::new(program)
        .args(&args)
        .current_dir(work)
        .env("CARGO_TARGET_DIR", target.join("cargo"))
        .env_remove("QOOB3D_HARNESS_UNLOCK")
        .envs(case.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let (Some(input), Some(mut pipe)) = (&case.stdin, child.stdin.take()) {
                pipe.write_all(input.as_bytes())?;
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("`{}` does not start: {e}", case.command))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        return Err(format!("`{}` let the sabotage through", case.command));
    }
    if !text.contains(&case.expect) {
        let tail: Vec<&str> = text.lines().rev().take(8).collect();
        return Err(format!(
            "`{}` failed, but not with “{}”. Last output:\n{}",
            case.command,
            case.expect,
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    Ok(())
}

/// Copies tracked and untracked-but-not-ignored files (the state under test).
fn copy_tree(root: &Path, to: &Path) -> Result<(), String> {
    let listing = sh(root, "git", &["ls-files", "-co", "--exclude-standard", "-z"], None)?;
    for rel in listing.split('\0').filter(|r| !r.is_empty()) {
        let from = root.join(rel);
        if !from.is_file() {
            continue; // deleted but still tracked
        }
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(&from, &dest).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(())
}

fn sh(dir: &Path, program: &str, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input.as_bytes()).map_err(|e| format!("{program}: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests;
