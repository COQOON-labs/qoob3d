//! Claude Code `PreToolUse` guard (ADR-0037, ADR-0039, ADR-0043). Reads the hook
//! input JSON and returns the reason for a denial:
//! - harness files are read-only unless the human started the session with
//!   `QOOB3D_HARNESS_UNLOCK=1` ([`Session::unlocked`]); the unlock lifts nothing else,
//! - files that can set the environment of the hooks, and `.git/`, are never
//!   written, so that no agent can unlock itself; a shell command naming `.git`
//!   is denied unless it only reads or names `.git` only to exclude it,
//! - format fixtures are never modified,
//! - one agent context writes either tests or production code, never both,
//! - shell commands must not bypass the git hooks or write harness files.
//!
//! Paths are collapsed (`.`, `..`) before any check. Shell commands are also
//! matched with quotes and backslashes removed, which split words without
//! changing what the shell runs. Paths and file names are compared ignoring
//! case, as macOS file systems do.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::paths::{self, Kind};

const TEST_AUTHOR: &str = "test-author";
const IMPLEMENTER: &str = "implementer";

/// Shell fragments that switch off or bypass a quality gate, matched lower-case
/// (git config keys ignore case).
const BYPASSES: &[&str] =
    &["--no-verify", "core.hookspath", "--admin", "qoob3d_harness_unlock", "qoob3d_skip_review"];
/// The shortest abbreviation of `--no-verify` git accepts (`--no-ver` is
/// ambiguous with `--no-verbose`).
const NO_VERIFY_MIN: &str = "--no-veri";
/// Shell fragments that (may) write files; also `dd` and `install` as words
/// and the programs with an output option in [`writes`] and [`edits_in_place`].
const WRITES: &[&str] = &[
    ">",
    "sed -i",
    "of=",
    "tee ",
    "mv ",
    "cp ",
    "rm ",
    "ln ",
    "truncate",
    "chmod",
    "touch ",
    "git checkout",
    "git restore",
    "git apply",
    "patch ",
    "python",
    "<<",
    "rsync",
    "ditto",
    "wget",
];
/// Path fragments of the harness as they appear in shell commands.
const HARNESS_IN_SHELL: &[&str] = &[
    ".claude/",
    ".githooks",
    ".github/",
    ".cargo/",
    "xtask/",
    "harness/",
    "Cargo.toml",
    "clippy.toml",
    "deny.toml",
    "rustfmt.toml",
    "rust-toolchain",
    // `cargo xtask` runs this binary; cargo links it from `deps/`.
    "target/debug/xtask",
    "target/debug/deps/xtask-",
];
/// Spellings of the repository root in shell commands besides the path itself.
const ROOT_VARIABLES: &[&str] =
    &["$CLAUDE_PROJECT_DIR", "${CLAUDE_PROJECT_DIR}", "$PWD", "${PWD}", "$(pwd)"];
/// Characters after which a path starts a new shell token.
const TOKEN_SEPARATORS: &str = "\"'=(;|&><`";

/// Files in the home directory that every new shell reads (ADR-0043).
const SHELL_STARTUP: &[&str] =
    &[".zshrc", ".zprofile", ".zshenv", ".bashrc", ".bash_profile", ".profile"];
/// Directories of Claude Code's managed settings.
const MANAGED_SETTINGS: &[&str] = &["/Library/Application Support/ClaudeCode", "/etc/claude-code"];
/// VS Code user settings, relative to the home directory.
const VSCODE_USER_SETTINGS: &[&str] =
    &["Library/Application Support/Code/User/settings.json", ".config/Code/User/settings.json"];
/// Shell fragments naming a file that can set the hook environment, matched
/// lower-case; the home path spelled out is added per session.
const HOOK_ENVIRONMENT_IN_SHELL: &[&str] = &[
    "settings.local.json",
    "managed-settings",
    "~/.claude/settings",
    "$home/.claude/settings",
    "${home}/.claude/settings",
    ".vscode/settings.json",
    "code/user/settings.json",
];
/// The git directory holds the hooks path, the hooks and the guard binary
/// (`.git/qoob3d/guard-target`).
const GIT_DIR: &str = ".git";
/// Options of `git rev-parse` that print the git directory.
const GIT_DIR_OPTIONS: &[&str] =
    &["--git-dir", "--absolute-git-dir", "--git-common-dir", "--git-path"];
/// What directly precedes a `.git` that is excluded, not used. Not `-E `: it
/// excludes only in `fd` ([`only_excludes_git_dir`]); `sed -E` and `rsync -E`
/// take a file after it.
const GIT_DIR_EXCLUSIONS: &[&str] = &["--exclude=", "--exclude-dir=", "--exclude ", "!"];
/// Programs that only read; a command made of them may name a hook-environment
/// file or `.git`. Not `less`/`more`: their `-o`/`--log-file` writes a file.
/// `sort`, `uniq`, `find` and `git rev-parse` in [`reads_only`].
const READ_ONLY: &[&str] = &[
    "cat", "head", "tail", "grep", "rg", "ls", "stat", "wc", "diff", "file", "test", "[", "du",
    "cut", "tr", "nl", "basename", "dirname", "realpath", "md5", "shasum",
];
/// Prefixes of the `find` primaries that write or run something.
const FIND_ACTIONS: &[&str] = &["-exec", "-ok", "-delete", "-fprint", "-fls"];

/// What the guard knows besides the hook input.
pub(crate) struct Session {
    /// The human started Claude Code with `QOOB3D_HARNESS_UNLOCK=1`.
    pub(crate) unlocked: bool,
    /// The user's home directory (`$HOME`).
    pub(crate) home: PathBuf,
}

impl Session {
    /// From the values of `QOOB3D_HARNESS_UNLOCK` and `HOME`; only exactly `1` unlocks.
    pub(crate) fn from_env_values(unlock: Option<&str>, home: Option<&str>) -> Self {
        Self { unlocked: unlock == Some("1"), home: PathBuf::from(home.unwrap_or_default()) }
    }
}

/// `Err(reason)` denies the tool call.
pub(crate) fn pre_tool_use(root: &Path, session: &Session, input: &Value) -> Result<(), String> {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let tool_input = &input["tool_input"];
    match tool {
        "Bash" => check_shell(root, session, tool_input["command"].as_str().unwrap_or_default()),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let Some(file) =
                tool_input["file_path"].as_str().or(tool_input["notebook_path"].as_str())
            else {
                return Ok(());
            };
            if sets_hook_environment(root, &session.home, file) {
                return Err(self_unlock_message(file));
            }
            let Some(rel) = relative(root, file) else { return Ok(()) };
            check_file_write(root, session, &rel)?;
            check_role(root, input, &rel)
        }
        _ => Ok(()),
    }
}

pub(crate) fn check_shell(root: &Path, session: &Session, command: &str) -> Result<(), String> {
    let normalised: String = command.chars().filter(|c| !matches!(c, '\'' | '"' | '\\')).collect();
    let forms = [command.to_lowercase(), normalised.to_lowercase()];
    check_bypasses(&forms[1])?;
    let read = is_pure_read(&normalised);
    if let Some(file) = hook_environment_in_shell(session, &forms)
        && !read
    {
        return Err(self_unlock_message(&file));
    }
    check_git_dir_in_shell(&forms, &normalised, read)?;
    let writes = forms.iter().any(|form| writes(form))
        || [command, normalised.as_str()].into_iter().any(edits_in_place);
    if session.unlocked || !writes {
        return Ok(());
    }
    if let Some(path) = forms.iter().find_map(|form| harness_in_shell(root, form)) {
        return Err(format!(
            "Shell command could modify the harness file `{path}` (ADR-0037). Harness changes \
             only in a session unlocked by the human (QOOB3D_HARNESS_UNLOCK=1)."
        ));
    }
    Ok(())
}

/// The first file named in the lower-cased `forms` of a command (as given,
/// normalised) that can set the hook environment. Startup files count only as
/// whole names.
fn hook_environment_in_shell(session: &Session, forms: &[String; 2]) -> Option<String> {
    let home_settings = format!("{}/.claude/settings", session.home.display()).to_lowercase();
    let fragment = HOOK_ENVIRONMENT_IN_SHELL
        .iter()
        .copied()
        .chain([home_settings.as_str()])
        .find(|f| forms[1].contains(f));
    let name = || {
        SHELL_STARTUP.iter().copied().find(|name| forms.iter().any(|form| names_file(form, name)))
    };
    fragment.or_else(name).map(str::to_owned)
}

/// A command naming the git dir in its lower-cased `forms` (as given,
/// normalised) — `.git` by name or a `git rev-parse` query — that is no pure
/// `read` and names `.git` not only to exclude it (ADR-0043).
fn check_git_dir_in_shell(forms: &[String; 2], normalised: &str, read: bool) -> Result<(), String> {
    let query = segments(&forms[1]).any(queries_git_dir);
    let named = forms.iter().any(|form| names_file(form, GIT_DIR));
    if !read && (query || (named && !only_excludes_git_dir(normalised))) {
        return Err(git_dir_message(GIT_DIR));
    }
    Ok(())
}

/// A segment with the word `rev-parse` and an option printing the git dir.
fn queries_git_dir(segment: &str) -> bool {
    let words = || segment.split(|c: char| !(c.is_alphanumeric() || c == '-'));
    words().any(|w| w == "rev-parse") && words().any(|w| GIT_DIR_OPTIONS.contains(&w))
}

/// Every `.git` in a normalised command directly follows an exclusion
/// prefix, or `-E ` in an `fd` segment.
fn only_excludes_git_dir(command: &str) -> bool {
    file_name_positions(command, GIT_DIR).all(|i| {
        let before = &command[..i];
        let segment = before.rsplit(['|', ';', '&', '\n']).next().unwrap_or_default();
        GIT_DIR_EXCLUSIONS.iter().any(|e| before.ends_with(e))
            || (before.ends_with("-E ") && segment.trim_start().starts_with("fd "))
    })
}

/// `name` stands in `command` as a whole file name.
fn names_file(command: &str, name: &str) -> bool {
    file_name_positions(command, name).next().is_some()
}

/// Where `name` stands in `command` as a whole file name, ignoring ASCII
/// case: no file-name character directly before or after it. Globs and brace
/// expansion therefore end a name (`~/.zshrc*`); a glob class inside it
/// (`~/.zsh[r]c`) is not caught.
fn file_name_positions<'a>(command: &'a str, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    command.char_indices().map(|(i, _)| i).filter(move |&i| {
        command[i..].get(..name.len()).is_some_and(|s| s.eq_ignore_ascii_case(name))
            && !command[..i].chars().next_back().is_some_and(is_name_char)
            && !command[i + name.len()..].chars().next().is_some_and(is_name_char)
    })
}

/// A lower-cased command that (may) write files; `perl -i` in
/// [`edits_in_place`] needs the case.
fn writes(command: &str) -> bool {
    WRITES.iter().any(|w| command.contains(w))
        || command
            .split(|c: char| !(c.is_ascii_alphanumeric() || "_-".contains(c)))
            .any(|w| w == "dd" || w == "install")
        || runs_with_option(command, &["curl"], 'o', &["--output", "--remote-name"])
        || runs_with_option(command, &["tar"], 'x', &["--extract"])
        || runs_with_option(command, &["git", "config"], 'f', &["--file"])
        || segments(command).any(|segment| {
            // `tar xzf`: the first word bundles the flags without a dash.
            let mut words = segment.split_whitespace();
            words.any(|w| w == "tar")
                && words.next().is_some_and(|w| !w.starts_with('-') && w.contains('x'))
        })
}

/// `perl`, also by path, followed by a short-flag group with a lower-case `i`
/// before any `-I<dir>`/`-M<module>` argument (`-pi`, `-i.bak`).
fn edits_in_place(command: &str) -> bool {
    segments(command).any(|segment| {
        let mut words = segment.split_whitespace();
        words.any(|w| w.rsplit('/').next().is_some_and(|name| name.eq_ignore_ascii_case("perl")))
            && words.any(|word| {
                short_flags(word).is_some_and(|flags| {
                    flags.chars().take_while(|c| !"IMm".contains(*c)).any(|c| c == 'i')
                })
            })
    })
}

/// Gate bypasses in a normalised, lower-cased command.
fn check_bypasses(command: &str) -> Result<(), String> {
    let bypass = BYPASSES
        .iter()
        .copied()
        .find(|b| command.contains(b))
        .or_else(|| abbreviates_no_verify(command).then_some("--no-verify"))
        .or_else(|| commits_without_verify(command).then_some("git commit -n"));
    if let Some(bypass) = bypass {
        return Err(format!(
            "`{bypass}` bypasses the quality gates (ADR-0037). Fix the cause instead of \
             bypassing it."
        ));
    }
    if command.match_indices("launchctl").any(|(i, word)| {
        let rest = &command[i + word.len()..];
        rest.starts_with(char::is_whitespace) && rest.trim_start().starts_with("setenv")
    }) {
        return Err("`launchctl setenv` changes the environment of future sessions and can \
                    unlock the harness; agents never run it (ADR-0043). Ask the human."
            .into());
    }
    Ok(())
}

/// A word that git expands to `--no-verify`.
fn abbreviates_no_verify(command: &str) -> bool {
    command
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .any(|word| word.len() >= NO_VERIFY_MIN.len() && "--no-verify".starts_with(word))
}

/// `git … commit` followed by a short-flag group containing `n`, the short
/// form of `--no-verify`.
fn commits_without_verify(command: &str) -> bool {
    runs_with_option(command, &["git", "commit"], 'n', &[])
}

/// A segment with the words of `program` in this order, followed by a
/// short-flag group containing `flag` or by one of the `long` options, also
/// abbreviated (`--out`) or extended (`--output-dir`).
fn runs_with_option(command: &str, program: &[&str], flag: char, long: &[&str]) -> bool {
    segments(command).any(|segment| {
        let mut words = segment.split_whitespace();
        program.iter().all(|p| words.any(|w| w == *p))
            && words.any(|word| {
                let name = word.split_once('=').map_or(word, |(name, _)| name);
                short_flags(name).is_some_and(|flags| flags.contains(flag))
                    || (name.len() > 2
                        && long.iter().any(|o| name.starts_with(o) || o.starts_with(name)))
            })
    })
}

/// The letters of a short-flag group (`-pi` → `pi`).
fn short_flags(word: &str) -> Option<&str> {
    word.strip_prefix('-').filter(|flags| !flags.starts_with('-'))
}

/// No redirection besides those in [`without_null_redirections`], no
/// substitution, and every segment [`reads_only`].
fn is_pure_read(command: &str) -> bool {
    let command = without_null_redirections(command);
    !command.contains(['>', '`', '(']) && segments(&command).all(reads_only)
}

/// A simple command that starts with a read-only program (a leading
/// `VAR=value` is not one): `sort` without an output file, `uniq` without an
/// output operand, `find` without actions, `git rev-parse`.
fn reads_only(segment: &str) -> bool {
    let mut words = segment.split_whitespace();
    match words.next() {
        Some("sort") => !runs_with_option(segment, &["sort"], 'o', &["--output"]),
        Some("uniq") => words.filter(|w| !w.starts_with('-')).count() <= 1,
        Some("find") => !words.any(|w| FIND_ACTIONS.iter().any(|a| w.starts_with(a))),
        Some("git") => words.next() == Some("rev-parse"),
        Some(program) => READ_ONLY.contains(&program),
        None => false,
    }
}

/// `command` without the redirections that write no file: `2>/dev/null`,
/// `>/dev/null`, `&>/dev/null` and `2>&1`, also with whitespace after `>`.
fn without_null_redirections(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut rest = command;
    while let Some(i) = rest.find('>') {
        let (before, target) = (&rest[..i], rest[i + 1..].trim_start());
        let word = |w: &str| {
            target.strip_prefix(w).filter(|after| {
                after.chars().next().is_none_or(|c| c.is_whitespace() || ";|&)".contains(c))
            })
        };
        let after = word("/dev/null").or_else(|| word("&1").filter(|_| before.ends_with('2')));
        if let Some(after) = after {
            out.push_str(before.strip_suffix(['2', '&']).unwrap_or(before));
            rest = after;
        } else {
            out.push_str(&rest[..=i]);
            rest = &rest[i + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// The simple commands of a shell command line.
fn segments(command: &str) -> impl Iterator<Item = &str> {
    command.split(['|', ';', '&', '\n']).map(str::trim).filter(|s| !s.is_empty())
}

fn self_unlock_message(path: &str) -> String {
    format!(
        "`{path}` can set the environment of Claude Code hooks (self-unlock) and is never \
         written by agents, not even in an unlocked session (ADR-0043). Ask the human to make \
         this change."
    )
}

fn git_dir_message(path: &str) -> String {
    format!(
        "`{path}`: `.git/` is managed by git and the guard; agents never write it, not even in \
         an unlocked session (ADR-0043). Ask the human to make this change."
    )
}

/// Claude, VS Code and shell configuration that reaches the environment of the
/// hooks (ADR-0043). A relative `file` is relative to `root`.
fn sets_hook_environment(root: &Path, home: &Path, file: &str) -> bool {
    let path = lower(&canonical(&collapse(&root.join(file))));
    let home = lower(&canonical(home));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new(""));
    let parent_name = parent.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    (parent == home.join(".claude") && name.starts_with("settings") && name.ends_with(".json"))
        || MANAGED_SETTINGS.iter().any(|dir| path.starts_with(lower(&canonical(Path::new(dir)))))
        || (parent_name == ".claude" && name == "settings.local.json")
        || (parent_name == ".vscode" && name == "settings.json")
        || VSCODE_USER_SETTINGS.iter().any(|rel| path == home.join(rel.to_lowercase()))
        || (parent == home && SHELL_STARTUP.contains(&name))
}

/// The first harness fragment that refers to this repository: at the start of
/// a token, after a `./` that starts a token, or right after the root (ADR-0043).
/// `command` is lower-cased.
fn harness_in_shell(root: &Path, command: &str) -> Option<&'static str> {
    let mut roots: Vec<String> = [root.to_path_buf(), canonical(root)]
        .iter()
        .map(|r| r.to_string_lossy().trim_end_matches('/').to_lowercase())
        .filter(|r| !r.is_empty())
        .collect();
    roots.extend(ROOT_VARIABLES.iter().map(|v| v.to_lowercase()));
    HARNESS_IN_SHELL.iter().copied().find(|fragment| {
        command
            .match_indices(&fragment.to_lowercase())
            .any(|(i, _)| refers_to_repo(&command[..i], &roots))
    })
}

/// Whether a path that follows `before` is inside the repository.
fn refers_to_repo(before: &str, roots: &[String]) -> bool {
    let token_start = |s: &str| {
        s.chars().next_back().is_none_or(|c| c.is_whitespace() || TOKEN_SEPARATORS.contains(c))
    };
    if token_start(before) || before.strip_suffix("./").is_some_and(token_start) {
        return true;
    }
    let Some(dir) = before.strip_suffix('/') else { return false };
    let dir = dir.strip_suffix(['"', '\'']).unwrap_or(dir);
    roots.iter().any(|r| dir.ends_with(r.as_str()))
}

fn check_file_write(root: &Path, session: &Session, rel: &str) -> Result<(), String> {
    let lower_rel = rel.to_lowercase();
    if lower_rel == GIT_DIR || lower_rel.starts_with(".git/") {
        return Err(git_dir_message(rel));
    }
    if paths::is_harness(rel) && !session.unlocked {
        return Err(format!(
            "`{rel}` belongs to the harness and is read-only for agents (ADR-0037). Propose the \
             change to the human; they unlock with QOOB3D_HARNESS_UNLOCK=1."
        ));
    }
    if lower_rel.starts_with(paths::FIXTURES) && root.join(rel).exists() {
        return Err(format!(
            "`{rel}` is a format fixture and immutable (ADR-0030). \
             New format version → new fixture + migration."
        ));
    }
    Ok(())
}

/// One context writes tests or code, never both (ADR-0039). Role agents are
/// fixed by their type; any other context is bound by its first write.
fn check_role(root: &Path, input: &Value, rel: &str) -> Result<(), String> {
    let wanted = match paths::kind(rel) {
        Kind::Other => return Ok(()),
        kind => kind,
    };
    let agent_type = input["agent_type"].as_str().unwrap_or_default();
    let role = match agent_type {
        TEST_AUTHOR => Some(Kind::Test),
        IMPLEMENTER => Some(Kind::Code),
        _ => None,
    };
    let role = match role {
        Some(role) => role,
        None => bind_role(root, input, wanted)?,
    };
    if role == wanted {
        return Ok(());
    }
    let (own, other_agent) = match role {
        Kind::Test => ("tests", IMPLEMENTER),
        _ => ("production code", TEST_AUTHOR),
    };
    Err(format!(
        "This agent context already writes {own}; `{rel}` must be changed by another context \
         (ADR-0039). Delegate to the subagent `{other_agent}`."
    ))
}

/// Reads or records the role of this (session, agent) context in `.git/qoob3d/roles/`.
fn bind_role(root: &Path, input: &Value, wanted: Kind) -> Result<Kind, String> {
    let session = input["session_id"].as_str().unwrap_or("unknown");
    let agent = input["agent_id"].as_str().unwrap_or("main");
    let dir = root.join(".git/qoob3d/roles");
    let file = dir.join(format!("{}--{}", sanitize(session), sanitize(agent)));
    match fs::read_to_string(&file).ok().as_deref().map(str::trim) {
        Some("test") => Ok(Kind::Test),
        Some("code") => Ok(Kind::Code),
        _ => {
            let label = if wanted == Kind::Test { "test" } else { "code" };
            fs::create_dir_all(&dir)
                .and_then(|()| fs::write(&file, label))
                .map_err(|e| format!("role ledger {}: {e}", file.display()))?;
            Ok(wanted)
        }
    }
}

fn sanitize(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Repository-relative path with `/`, or `None` outside the repository. A
/// relative `file` is relative to `root`; `.` and `..` are collapsed first.
pub(crate) fn relative(root: &Path, file: &str) -> Option<String> {
    let path = canonical(&collapse(&root.join(file)));
    let rel = path.strip_prefix(canonical(root)).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Drops `.` and resolves `..` lexically in an absolute path, so that a detour
/// through a missing directory cannot hide the real target.
fn collapse(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn lower(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

/// Canonicalises the longest existing ancestor, so new files and symlinked
/// roots (macOS `/private/tmp`) compare correctly.
fn canonical(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else { break };
        missing.push(name);
        existing = parent;
    }
    let mut out = existing.canonicalize().unwrap_or_else(|_| existing.to_path_buf());
    out.extend(missing.iter().rev());
    out
}

#[cfg(test)]
mod tests;
