//! Rule register (ADR-0040): every rule in `harness/rules.md` names at least
//! one check, and every named check exists. A rule whose check disappears
//! turns this step red instead of silently becoming a wish.
//!
//! Check kinds: `xtask:<step>`, `lint:<name>`, `ci:<job>`, `hook:<script>`,
//! `githook:<name>`, `critic:K-NN`, `agent:<name>`, `skill:<name>`,
//! `test:<fn>`, `manual`.

use std::fs;
use std::path::Path;

pub(crate) const REGISTER: &str = "harness/rules.md";
/// The critic's check items (`K-NN`, ADR-0041).
const CRITIC_CHECKLIST: &str = "harness/critic-checklist.md";
/// Source of the subcommands and check steps of this binary.
const XTASK_MAIN: &str = "xtask/src/main.rs";

/// One row of the register: rule id and its checks.
pub(crate) struct Rule {
    pub(crate) id: String,
    pub(crate) checks: Vec<String>,
}

pub(crate) fn check(root: &Path) -> Result<(), Vec<String>> {
    let text =
        fs::read_to_string(root.join(REGISTER)).map_err(|e| vec![format!("{REGISTER}: {e}")])?;
    let rules = parse(&text);
    if rules.is_empty() {
        return Err(vec![format!("{REGISTER}: no rules found")]);
    }
    let mut problems = Vec::new();
    for rule in &rules {
        if rule.checks.is_empty() {
            problems.push(format!("{}: no check given", rule.id));
        }
        for check in &rule.checks {
            if let Err(why) = exists(root, check) {
                problems.push(format!("{}: check `{check}` {why}", rule.id));
            }
        }
    }
    let manual_only =
        rules.iter().filter(|r| r.checks.iter().all(|c| c == "manual" || c.starts_with("critic:")));
    for rule in manual_only {
        println!("rules: {} is only checked by the critic or manually", rule.id);
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Table rows `| R-01 | text | `check`, `check` |` → rules.
pub(crate) fn parse(text: &str) -> Vec<Rule> {
    text.lines()
        .filter_map(|line| {
            let cells: Vec<&str> =
                line.trim().trim_matches('|').split('|').map(str::trim).collect();
            let id = cells.first()?;
            if !id.starts_with("R-") || cells.len() < 3 {
                return None;
            }
            let checks = cells[cells.len() - 1]
                .split(',')
                .map(|c| c.trim().trim_matches('`').to_string())
                .filter(|c| !c.is_empty())
                .collect();
            Some(Rule { id: (*id).to_string(), checks })
        })
        .collect()
}

/// `Ok` if the named check exists in the repository.
fn exists(root: &Path, check: &str) -> Result<(), &'static str> {
    let read = |p: &str| fs::read_to_string(root.join(p)).unwrap_or_default();
    let found = match check.split_once(':') {
        None => check == "manual",
        Some(("xtask", step)) => names_step(&read(XTASK_MAIN), step),
        Some(("lint", lint)) => {
            read("Cargo.toml").lines().any(|l| l.split('=').next().map(str::trim) == Some(lint))
        }
        Some(("ci", job)) => {
            fs::read_dir(root.join(".github/workflows")).into_iter().flatten().flatten().any(|e| {
                fs::read_to_string(e.path()).unwrap_or_default().contains(&format!("\n  {job}:\n"))
            })
        }
        Some(("hook", script)) => {
            root.join(format!(".claude/hooks/{script}.sh")).exists()
                && read(".claude/settings.json").contains(&format!("{script}.sh"))
        }
        Some(("githook", name)) => root.join(".githooks").join(name).exists(),
        Some(("critic", id)) => read(CRITIC_CHECKLIST).contains(&format!("| {id} |")),
        Some(("agent", name)) => root.join(format!(".claude/agents/{name}.md")).exists(),
        Some(("skill", name)) => root.join(format!(".claude/skills/{name}/SKILL.md")).exists(),
        Some(("test", name)) => test_exists(&root.join("crates"), &format!("fn {name}(")),
        Some(_) => return Err("has an unknown kind"),
    };
    if found { Ok(()) } else { Err("does not exist") }
}

/// `step` is a subcommand of main's dispatch (`Some("step")`) or a named step
/// of `check()` (`("step", Step::…)`).
fn names_step(main: &str, step: &str) -> bool {
    let quoted = format!("\"{step}\"");
    main.match_indices(&quoted).any(|(i, _)| {
        let before = main[..i].trim_end();
        let after = main[i + quoted.len()..].trim_start();
        (before.ends_with("Some(") && after.starts_with(')'))
            || (before.ends_with('(')
                && after.strip_prefix(',').is_some_and(|a| a.trim_start().starts_with("Step::")))
    })
}

fn test_exists(dir: &Path, needle: &str) -> bool {
    fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let path = e.path();
        if path.is_dir() {
            test_exists(&path, needle)
        } else {
            path.extension().is_some_and(|x| x == "rs")
                && fs::read_to_string(&path).is_ok_and(|t| t.contains(needle))
        }
    })
}

#[cfg(test)]
mod tests;
