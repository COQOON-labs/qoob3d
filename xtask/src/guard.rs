//! Claude Code `PreToolUse` guard (ADR-0037, ADR-0039, ADR-0043). Reads the hook
//! input JSON and returns the reason for a denial:
//! - harness files are read-only unless the human started the session with
//!   `QOOB3D_HARNESS_UNLOCK=1` ([`Session::unlocked`]); the unlock lifts nothing else,
//! - files that can set the environment of the hooks, and `.git/`, are never
//!   written, so that no agent can unlock itself; a shell command naming `.git`
//!   is denied unless it only reads (the [`pure_read`] allowlist, as for
//!   harness paths and a hook-environment file) or names `.git` only to
//!   exclude it. Limit
//!   (ADR-0043): an unlocked session may change the guard itself (`guard.rs`,
//!   `.claude/hooks/guard.sh`, the project `.claude/settings.json`); the
//!   owner's review covers that,
//! - format fixtures are never modified, also not by a shell command that is
//!   no pure read ([`check_shell_paths`]); the fixture directory and its
//!   ancestors up to the repository root, and globs whose literal prefix may
//!   reach them, count as fixtures ([`holds_fixtures`]),
//! - one agent context writes either tests or production code, never both,
//!   also through a shell command that is no pure read and names test or code
//!   paths (`git add`, `git commit` and `git push` excepted); a named
//!   source-tree directory outside `tests`, or a source-tree path with a glob
//!   or brace, holds both and conflicts with every role ([`named_kinds`]),
//! - shell commands must not bypass the git hooks; a command that names a
//!   harness path must be a pure read ([`pure_read`], an allowlist; ADR-0049).
//!   A harness directory counts also when named without its trailing slash, as a
//!   whole shell argument, also in a comment (every rule for where the shell
//!   starts a comment was bypassable: `${x:- #}`, zsh's `(#i)` and
//!   `x(a| #)`), but not in the heredoc body of an exact message form
//!   (`git commit|tag -F -`, `gh pr|issue create|edit --body-file -`, their
//!   `"$(cat <<EOF"` messages, `cat` writing one regular file with a literal
//!   name) with no `#` on its line and nothing after the marker but one file
//!   redirection, standing alone as the whole command (around it only blank
//!   lines and the closing `)"`), under an unquoted marker without `$(`, `${`
//!   or a line ending in a backslash in the body, and inside `$(…)` without a
//!   body line starting with the marker word. A command with a construct the
//!   guard does not model (backticks, `$'`, redirections with `&`, arithmetic
//!   `((`) or with a heredoc that keeps its body or has no terminator keeps
//!   all heredoc bodies: fail closed (ADR-0049). A sed with in-place editing,
//!   a script with `w`/`W` or `e`, a script file (`-f`) or a script expanded
//!   by the shell is no read. Where a `#` may start a comment, the guard takes
//!   the reading that checks more: a command is a pure read only if it is
//!   one with comments skipped and with comments as words, and a `#` on a
//!   heredoc line keeps its body. A sed that a naive split puts in command
//!   position but neither reading finds is no read.
//!
//! Edit/Write paths are collapsed (`.`, `..`) before any check. Shell commands
//! are checked as text, at the three anchors of ADR-0043 (token start, `./`,
//! the repository root), without collapsing (ADR-0049). They are also matched
//! with quotes and backslashes removed, which split words without changing
//! what the shell runs. Paths and file names are compared ignoring case, as
//! macOS file systems do.

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
/// The names of sed ([`is_sed`]).
const SED: &[&str] = &["sed", "gsed"];
/// Shells whose `-c` runs its argument as a command line.
const SHELLS: &[&str] = &["sh", "bash", "zsh"];
/// Path fragments of the harness as they appear in shell commands; a directory
/// of [`paths::HARNESS`] also as a whole argument without its slash
/// ([`harness_in_shell`]).
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
/// Characters after which a path starts a new shell token; `#` because the
/// words of a comment are checked too (`# harness`, `#harness`, ADR-0049).
const TOKEN_SEPARATORS: &str = "\"'=(;|&><`#";
/// Characters after which a bare directory name is still a whole argument:
/// separators and redirections end it, a glob expands it to the directory
/// itself (`xtask*`). Not `:` or `,`, which make it prose (`xtask: fix`).
const ARGUMENT_ENDS: &str = ";|&)<>`*?[";
/// Shell constructs [`without_prose`] does not model: command substitution
/// with backticks, ANSI-C quoting, redirections with `&` (`|&`, `2>&1`,
/// `&>`, `<&0`), and arithmetic, whose `<<` is a shift (`((`, `$((`). A
/// command with one keeps its prose (fail closed, ADR-0049).
const UNMODELLED: &[&str] = &["`", "$'", "|&", ">&", "&>", "<&", "(("];

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
/// `sort`, `uniq`, `rg`, `find` and `git rev-parse` in [`reads_only`].
const READ_ONLY: &[&str] = &[
    "cat", "head", "tail", "grep", "ls", "stat", "wc", "diff", "file", "test", "[", "du", "cut",
    "tr", "nl", "basename", "dirname", "realpath", "md5", "shasum",
];
/// Prefixes of the `find` primaries that write or run something.
const FIND_ACTIONS: &[&str] = &["-exec", "-ok", "-delete", "-fprint", "-fls"];
/// git subcommands that only read ([`git_reads`]).
const GIT_READS: &[&str] =
    &["log", "diff", "show", "status", "blame", "ls-files", "grep", "rev-parse", "cat-file"];
/// git's global options that take the next word as their argument.
const GIT_GLOBAL_ARGUMENTS: &[&str] =
    &["-c", "-C", "--git-dir", "--work-tree", "--namespace", "--config-env", "--exec-path"];

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
        "Bash" => {
            let command = tool_input["command"].as_str().unwrap_or_default();
            check_shell(root, session, command)?;
            check_shell_paths(root, input, command)
        }
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
    let normalised = unquoted(command);
    let forms = [command.to_lowercase(), normalised.to_lowercase()];
    check_bypasses(&forms[1])?;
    let read = pure_read(command);
    if let Some(file) = hook_environment_in_shell(session, &forms)
        && !read
    {
        return Err(self_unlock_message(&file));
    }
    check_git_dir_in_shell(&forms, &normalised, read)?;
    if session.unlocked {
        return Ok(());
    }
    let shell = if models_prose(command) { without_prose(command) } else { command.to_owned() };
    let shell_forms = [shell.to_lowercase(), unquoted(&shell).to_lowercase()];
    if let Some(path) =
        forms.iter().zip(&shell_forms).find_map(|(form, shell)| harness_in_shell(root, form, shell))
        && !pure_read(command)
    {
        return Err(format!(
            "Shell command could modify the harness file `{path}` (ADR-0037). Harness changes \
             only in a session unlocked by the human (QOOB3D_HARNESS_UNLOCK=1)."
        ));
    }
    Ok(())
}

/// The simple commands of `command` in both readings of every `#` that may
/// start a comment: an odd quote in a comment shifts the quote state of the
/// other one (ADR-0049).
fn both_readings(command: &str) -> Vec<Vec<Word>> {
    [false, true]
        .into_iter()
        .flat_map(|comments| simple_commands(command, comments).commands)
        .collect()
}

/// The texts of `words`, joined by blanks.
fn joined_words(words: &[Word]) -> String {
    words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// `command` only reads (ADR-0049, an allowlist): no redirection besides
/// those in [`without_null_redirections`], no heredoc, no construct where the
/// shell may run more than the tokenizer sees ([`DIVERGES`]), no sed the
/// tokenizer loses ([`hides_sed`]), and every simple command of both readings
/// [`reads`].
fn pure_read(command: &str) -> bool {
    let commands = both_readings(command);
    !DIVERGES.iter().any(|construct| command.contains(construct))
        && !(command.contains("${") && command.contains('@'))
        && !redirects_to_file(command)
        && !has_heredoc(command)
        && !has_qualifier(command)
        && !hides_sed(command, &commands)
        && commands.iter().all(|words| reads(words))
}

/// Constructs where the shell may run or assign more than the tokenizer sees,
/// so a command with one is no [`pure_read`] (fail closed): arithmetic, whose
/// assignments change variables (`$((IFS=1))`), and zsh's parameter flags
/// (`${(e)x}` evaluates `x`). Bash's `${x@P}` (prompt expansion runs `$(…)`)
/// is caught as `${` with an `@`.
const DIVERGES: &[&str] = &["((", "${("];

/// `command` has a heredoc (`<<`, `<<-`; not the here-string `<<<`) in either
/// comment reading: the tokenizer cannot follow its body (`echo it's` hides the
/// commands after it), so the command is no [`pure_read`]. The bodies of safe
/// message forms are removed before the harness and role checks look at paths.
fn has_heredoc(command: &str) -> bool {
    [false, true].into_iter().any(|comments| {
        simple_commands(command, comments)
            .targets
            .iter()
            .any(|(operator, _)| operator.starts_with("<<") && !operator.starts_with("<<<"))
    })
}

/// `command` has a zsh glob qualifier or group in either comment reading
/// ([`Tokens`]): no [`pure_read`], and every word counts as a path.
fn has_qualifier(command: &str) -> bool {
    [false, true].into_iter().any(|comments| simple_commands(command, comments).qualifier)
}

/// A simple command that only reads: nothing but assignments (their
/// substitutions are simple commands of their own), a compound keyword before
/// a read, a comment (a command `#…` in the reading that keeps comments, the
/// other reading checks what follows it), `echo`, `printf`, `cd`,
/// `shellcheck`, `true`, `:`, sed without
/// in-place editing and with a readable script that writes nothing, a git
/// read ([`git_reads`]), or one of [`reads_only`]. A program after an
/// assignment (`GIT_EXTERNAL_DIFF=x git diff`) or given by an expansion is
/// none.
fn reads(words: &[Word]) -> bool {
    let Some(index) = words.iter().position(|w| !is_assignment(&w.text)) else {
        return words.iter().all(|w| harmless_assignment(&w.text));
    };
    let (program, arguments) = (&words[index], &words[index + 1..]);
    if index > 0 || program.expanded {
        return false;
    }
    match program.text.as_str() {
        // In the reading that keeps comments, a comment is a command `#…`.
        comment if comment.starts_with('#') => true,
        "if" | "then" | "else" | "elif" | "fi" | "{" | "}" => reads(arguments),
        // `printf -v NAME` assigns a variable (`PATH`).
        "printf" => !arguments.iter().any(|w| w.text.starts_with("-v")),
        "echo" | "cd" | "shellcheck" | "true" | ":" => true,
        "git" => git_reads(arguments),
        name if is_sed(name) => {
            !edits_in_place(&joined_words(words)) && !sed_command_writes(arguments)
        }
        _ => reads_only(&joined_words(words)),
    }
}

/// An assignment on its own that cannot change what a later read runs: the
/// name has no upper-case letter (exported variables such as `PATH`, `IFS`,
/// `GIT_*` are upper case) and is none of zsh's arrays tied to them (`path`,
/// `cdpath`, `fpath`, `manpath`, `module_path`).
fn harmless_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.contains(|c: char| c.is_ascii_uppercase())
            && !matches!(name, "path" | "cdpath" | "fpath" | "manpath" | "module_path")
    })
}

/// `NAME=value`, a shell assignment.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The `arguments` of git run a read subcommand ([`GIT_READS`]) without a
/// global option before it and without an option that writes a file or runs
/// a program: `--output` (and `-o`, except for grep, where it is
/// `--only-matching`), `--ext-diff`, `--textconv`, cat-file's `--filters`,
/// grep's `-O`/`--open-files-in-pager` (also bundled, `-nO…`, and
/// abbreviated, `--op`).
fn git_reads(arguments: &[Word]) -> bool {
    let Some((subcommand, rest)) = arguments.split_first() else { return false };
    let grep = subcommand.text == "grep";
    GIT_READS.contains(&subcommand.text.as_str())
        && !rest.iter().any(|w| {
            let word = w.text.as_str();
            (word == "-o" && !grep)
                || ["--output", "--ext", "--textc"].iter().any(|o| word.starts_with(o))
                || is_option(word, |_| false, &["--filters"])
                || (grep
                    && is_option(word, |group| group.contains('O'), &["--open-files-in-pager"]))
        })
}

/// A shell command that is no [`pure_read`] follows the rules of Edit/Write
/// for the repository paths named by its simple commands that do not only read
/// ([`reads`]): a fixture path is denied (ADR-0030), and every test or code
/// path goes through the role check (ADR-0039), which binds the role of a
/// context without an agent type. A read binds nothing, and neither do
/// `git add`, `git commit` and `git push`, which change no file content (the
/// main context stages tests and code separately); the bodies of safe message
/// forms are no paths. With a file redirection, whose target is no word, a
/// heredoc, whose body the tokenizer cannot follow, or a sed the tokenizer
/// lost, every word of the other commands counts, split at blanks. In a
/// cargo command only words with a `/` are paths (`-p xtask` names a package).
fn check_shell_paths(root: &Path, input: &Value, command: &str) -> Result<(), String> {
    if pure_read(command) {
        return Ok(());
    }
    let text = if models_prose(command) { without_prose(command) } else { command.to_owned() };
    let commands = both_readings(&text);
    let every = redirects_to_file(&text)
        || has_heredoc(&text)
        || has_qualifier(&text)
        || hides_sed(&text, &commands);
    let targets: Vec<Word> = [false, true]
        .into_iter()
        .filter(|_| every)
        .flat_map(|comments| simple_commands(&text, comments).targets)
        .map(|(_, word)| word)
        .collect();
    // Every word, split at blanks so that a region the tokenizer lost inside
    // a quote (a quote in a comment) still yields its paths; with whether it
    // is a pattern of the shell.
    let lists: Vec<Vec<(&str, bool)>> = commands
        .iter()
        // Assignments stay: `d=…/fixtures; rm -rf $d`.
        .filter(|words| every || !reads(words) || words.iter().all(|w| is_assignment(&w.text)))
        .chain([&targets])
        .map(|words| {
            words.iter().flat_map(|w| w.text.split_whitespace().map(|p| (p, w.pattern))).collect()
        })
        .collect();
    let named: Vec<(String, bool)> = lists
        .iter()
        .filter(|words| !stages(words.iter().map(|(word, _)| *word)))
        .flat_map(|words| {
            let cargo = words.first().is_some_and(|(w, _)| program_name(w) == "cargo");
            words.iter().filter(move |(word, _)| !cargo || word.contains('/'))
        })
        .flat_map(|(word, pattern)| word.split('=').map(|part| (part, *pattern)))
        .filter(|(word, _)| !word.is_empty())
        .filter_map(|(word, pattern)| relative(root, word).map(|rel| (rel, pattern)))
        .collect();
    // A fixture message comes before any role message.
    if let Some((fixture, _)) = named.iter().find(|(rel, pattern)| {
        rel.to_lowercase().starts_with(paths::FIXTURES) || holds_fixtures(rel, *pattern)
    }) {
        return Err(fixture_message(if fixture.is_empty() { "." } else { fixture }));
    }
    named.iter().try_for_each(|(rel, pattern)| check_named_role(root, input, rel, *pattern))
}

/// `rel` is the fixture directory or one of its ancestors, up to the
/// repository root (`""`): removing, moving or restoring it touches every
/// fixture (ADR-0030). A `pattern` with a glob or brace (`*?[{`) counts if
/// its literal prefix is a prefix of the fixture directory or lies inside it
/// (`tests/*`, `tests/fix*`, `crates/*/…`, `fixture{s,}`).
fn holds_fixtures(rel: &str, pattern: bool) -> bool {
    let rel = rel.to_lowercase();
    if let Some(glob) = rel.find(GLOB).filter(|_| pattern) {
        let literal = &rel[..glob];
        return paths::FIXTURES.starts_with(literal) || literal.starts_with(paths::FIXTURES);
    }
    let dir = paths::FIXTURES.trim_end_matches('/');
    rel.is_empty() || rel == dir || dir.starts_with(&format!("{rel}/"))
}

/// Characters that make a path a pattern of the shell (globs and braces).
const GLOB: [char; 4] = ['*', '?', '[', '{'];

/// The `words` of a simple command are `git add`, `git commit` or `git push`
/// (git also by path, also after global options), which change no file
/// content.
fn stages<'a>(mut words: impl Iterator<Item = &'a str>) -> bool {
    words.next().is_some_and(|w| program_name(w) == "git")
        && git_subcommand(words).is_some_and(|sub| matches!(sub, "add" | "commit" | "push"))
}

/// `command` with quotes and backslashes removed.
fn unquoted(command: &str) -> String {
    command.chars().filter(|c| !matches!(c, '\'' | '"' | '\\')).collect()
}

/// The guard models every construct of `command` that could start or end
/// prose, so [`without_prose`] may remove it: none of [`UNMODELLED`].
/// Otherwise every word is checked (fail closed, ADR-0049).
fn models_prose(command: &str) -> bool {
    !UNMODELLED.iter().any(|construct| command.contains(construct))
}

/// `command` without the heredoc body of a message form (from the line after
/// `<<MARKER` through the line `MARKER`), which the shell reads as no words.
/// Comments stay: every rule for where the shell starts one was bypassable
/// (`${x:- #}`, zsh's `(#i)` and `x(a| #)`), so their words are checked like
/// any other (ADR-0049). Quotes, backslashes and `$(…)` (also in double
/// quotes) are followed, so a quoted `<<` starts no heredoc. A heredoc body is
/// removed only in the safe shape: the only heredoc of its line, which has no
/// `#` (it may start a comment that hides the marker, `-F - #<<EOF`), read as a
/// message or written to a file ([`is_message_form`]), with nothing after the
/// marker but one file redirection ([`only_redirects_to_file`]), as the whole
/// command ([`stands_alone`]), and under an unquoted marker only without
/// `$(`, `${` or a line ending in a backslash in the body ([`expands`]), inside
/// `$(…)` only without an early terminator ([`ends_early`]; ADR-0049). If any
/// heredoc keeps its body or has no terminator, nothing is removed and
/// `command` comes back unchanged: the quote tracking cannot follow a body
/// (`it's`). A backslash-newline continues the line. Only for commands that
/// [`models_prose`].
fn without_prose(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    // Open quotes, parentheses `(` and substitutions `$(` (as `$`), innermost last.
    let mut open: Vec<char> = Vec::new();
    let mut markers: Vec<Marker> = Vec::new();
    // Where the current line starts in `out`, where the text after the last
    // heredoc marker starts.
    let (mut line_start, mut after_marker) = (0, 0);
    let mut i = 0;
    while let Some(c) = command[i..].chars().next() {
        let rest = &command[i..];
        let mut end = i + c.len_utf8();
        let heredocs = markers.len();
        match (open.last().copied(), c) {
            (Some('\''), '\'') | (Some('"'), '"') | (Some('(' | '$'), ')') => {
                open.pop();
            }
            (Some('\''), _) => {}
            (_, '\\') => end += rest[1..].chars().next().map_or(0, char::len_utf8),
            (_, '$') if rest.starts_with("$(") => {
                open.push('$');
                end += 1;
            }
            (Some('"'), _) => {}
            (_, '\'' | '"' | '(') => open.push(c),
            (_, '<') => {
                let len = rest.len() - rest.trim_start_matches('<').len();
                end = i + len;
                if len == 2
                    && let Some((len, marker)) = heredoc(rest)
                {
                    markers.push(marker);
                    end = i + len;
                }
            }
            (_, '\n') if !markers.is_empty() => {
                let open_parens = open.iter().filter(|&&o| matches!(o, '(' | '$')).count();
                // A `#` on the heredoc line may start a comment that hides the
                // marker (`-F - #<<EOF`): then the body lines run.
                if markers.len() == 1
                    && !out[line_start..].contains('#')
                    && only_redirects_to_file(&out[after_marker..])
                    && is_message_form(
                        &out[line_start..after_marker],
                        &out[after_marker..],
                        open_parens,
                    )
                    && let Some(len) = heredoc_bodies(&command[end..], &markers)
                    && (markers[0].quoted || !expands(&command[end..end + len]))
                    && (open_parens == 0 || !ends_early(&command[end..end + len], &markers[0].word))
                    && stands_alone(&out[..line_start], &command[end + len..], open_parens > 0)
                {
                    end += len;
                } else {
                    // A kept body is shell text the quote tracking cannot follow
                    // (`it's`): remove no prose at all.
                    return command.to_owned();
                }
                markers.clear();
                out.push('\n');
                line_start = out.len();
                i = end;
                continue;
            }
            _ => {}
        }
        out.push_str(&command[i..end]);
        if markers.len() > heredocs {
            after_marker = out.len();
        }
        let outside_quotes = matches!(open.last(), None | Some('(' | '$'));
        if outside_quotes && c == '\n' {
            line_start = out.len();
        }
        i = end;
    }
    if markers.is_empty() { out } else { command.to_owned() }
}

/// A heredoc `body` under an unquoted marker is not what the program reads:
/// the shell runs `$(…)` and `${…}` in it (an escaped `\$(` counts too;
/// backticks and `$((` fail closed, [`UNMODELLED`]), and it joins a line
/// ending in a backslash with the next one, also before comparing it with the
/// terminator (`EO\` newline `F` ends the body, `$\` newline `(` runs).
fn expands(body: &str) -> bool {
    body.lines().any(|line| line.ends_with('\\')) || body.contains("$(") || body.contains("${")
}

/// Inside `$(…)` bash ends a heredoc early at a line that starts with the
/// marker `word` (`EOF)`, `EOF )`, `EOFX)`, bash 3.2 also after blanks), zsh
/// does not; the guard cannot tell which shell runs. True if a line of `body`
/// before its terminator line starts with `word` after optional blanks (fail
/// closed).
fn ends_early(body: &str, word: &str) -> bool {
    let lines: Vec<&str> = body.lines().collect();
    lines[..lines.len().saturating_sub(1)]
        .iter()
        .any(|line| line.trim_start_matches([' ', '\t']).starts_with(word))
}

/// A message form is the whole Bash command (ADR-0049): `before`, the text
/// before its logical line, is blank, and `after`, the text after the
/// terminator line, holds only blank lines (a comment line is not blank); for a
/// `"$(cat <<EOF` form (`substitution`) first the closing line `)"` with
/// [`plain_arguments`]. A command before the heredoc can route its output into
/// a shell (`exec > >(bash)`, a fifo), one after it can run the file written.
fn stands_alone(before: &str, after: &str, substitution: bool) -> bool {
    let mut lines = after.lines().peekable();
    if substitution {
        lines.next_if(|line| line.trim_start().strip_prefix(")\"").is_some_and(plain_arguments));
    }
    before.trim().is_empty() && lines.all(|line| line.trim().is_empty())
}

/// `arguments` are words that only extend the command: quotes closed, no
/// `;&|<>()`, backtick, `$` or backslash.
fn plain_arguments(arguments: &str) -> bool {
    let mut quote = None;
    for c in arguments.chars() {
        match quote {
            None if matches!(c, '\'' | '"') => quote = Some(c),
            Some(open) if c == open => quote = None,
            _ => {}
        }
    }
    quote.is_none() && !arguments.contains(|c: char| ";&|<>()`$\\".contains(c))
}

/// `tail`, the rest of a heredoc line after its marker,
/// leaves the body data: nothing, or one output redirection to a file
/// (`> /tmp/note`, `>>/tmp/note`). A pipe, `;`, `&&`, a subshell, a second
/// redirection or any other word may feed the body to a program.
fn only_redirects_to_file(tail: &str) -> bool {
    let tail = tail.trim();
    let Some(target) = tail.strip_prefix(">>").or_else(|| tail.strip_prefix('>')) else {
        return tail.is_empty();
    };
    let target = target.trim_start();
    !target.is_empty() && !target.contains(|c: char| c.is_whitespace() || "|&;<>(){}".contains(c))
}

/// `head`, a heredoc line up to its marker, with `tail`,
/// the rest of the line, is one of the forms whose heredoc body ends up in a
/// commit or tag message, a PR or issue body, or a file, never runs
/// (ADR-0049). Programs also by path:
/// - `git commit|tag … -F -` (`--file=-`, `--file -`), no global option
///   before the subcommand (`git -C`, `git -c`);
/// - `gh pr|issue create|edit … --body-file -`;
/// - `git commit|tag … -m "$(cat <<EOF` (`--message=`) and
///   `gh pr|issue create|edit … --body "$(cat <<EOF`, the only `$(` and the
///   `open_parens` still open;
/// - `cat > path <<EOF` (`>>`, or the redirection in `tail`): `cat` and one
///   redirection to a regular file, no other word ([`writes_regular_file`]).
///   Not `tee`, which copies the body to stdout too.
///
/// No `;&|(){}` besides that `$(` (a list, a pipe, a subshell, a group), no
/// other program in front (`env cat`, `X=1 git`). The caller also requires the
/// form to be the whole command ([`stands_alone`]).
fn is_message_form(head: &str, tail: &str, open_parens: usize) -> bool {
    let prefix = head.rfind("<<").map_or(head, |i| &head[..i]);
    let parts: Vec<&str> = prefix.split("$(").collect();
    if parts.len() - 1 != open_parens
        || parts.iter().any(|part| part.contains(|c: char| ";&|(){}".contains(c)))
    {
        return false;
    }
    match parts.as_slice() {
        [command] => {
            let words = line_words(command);
            git_message(&words).is_some_and(|rest| reads_stdin(rest, "-F", "--file"))
                || gh_body(&words)
                    .is_some_and(|rest| reads_stdin(rest, "--body-file", "--body-file"))
                || words.split_first().is_some_and(|(cat, rest)| {
                    program_name(cat) == "cat"
                        && writes_regular_file(&format!("{} {tail}", rest.join(" ")))
                })
        }
        [outer, inner] => {
            let outer = line_words(outer.trim_end().trim_end_matches('"'));
            line_words(inner).iter().map(|w| program_name(w)).eq(["cat"])
                && (git_message(&outer).is_some_and(|rest| {
                    matches!(rest.last(), Some(&("-m" | "--message" | "--message=")))
                }) || gh_body(&outer)
                    .is_some_and(|rest| matches!(rest.last(), Some(&("--body" | "--body=")))))
        }
        _ => false,
    }
}

/// `redirection`, the words of a `cat` heredoc besides `cat` and the marker, is
/// exactly one `>` or `>>` without a descriptor number (not `1>`, `2>`, `>|`)
/// to a literal target (no `$*?[{` or backtick, which the shell expands) with
/// no `dev` or `proc` component, ignoring case (`/dev/stdout`, `//dev/tty`,
/// `../dev/fd/3`, `/proc/self/fd/1`).
fn writes_regular_file(redirection: &str) -> bool {
    let redirection = redirection.trim();
    let Some(target) = redirection.strip_prefix(">>").or_else(|| redirection.strip_prefix('>'))
    else {
        return false;
    };
    only_redirects_to_file(redirection)
        && !target.contains(['$', '*', '?', '[', '{', '`'])
        && !unquoted(target).to_lowercase().split('/').any(|c| matches!(c.trim(), "dev" | "proc"))
}

/// The words after `git commit` or `git tag` (git also by path, no global
/// option before the subcommand).
fn git_message<'a>(words: &'a [&'a str]) -> Option<&'a [&'a str]> {
    match words {
        [git, "commit" | "tag", rest @ ..] if program_name(git) == "git" => Some(rest),
        _ => None,
    }
}

/// The words after `gh pr|issue create|edit` (gh also by path).
fn gh_body<'a>(words: &'a [&'a str]) -> Option<&'a [&'a str]> {
    match words {
        [gh, "pr" | "issue", "create" | "edit", rest @ ..] if program_name(gh) == "gh" => {
            Some(rest)
        }
        _ => None,
    }
}

/// `words` hand stdin to the option `short` or `long` as `-`: `short -`,
/// `long -` or `long=-` (`-F -`, `--file=-`).
fn reads_stdin(words: &[&str], short: &str, long: &str) -> bool {
    words.windows(2).any(|pair| (pair[0] == short || pair[0] == long) && pair[1] == "-")
        || words.iter().any(|w| w.strip_prefix(long).is_some_and(|value| value == "=-"))
}

/// The words of a command line part, without the `\` of a backslash-newline.
fn line_words(part: &str) -> Vec<&str> {
    part.split_whitespace().filter(|w| *w != "\\").collect()
}

/// The program name of a command word given by path (`/usr/bin/git` → `git`).
fn program_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// A heredoc marker: the terminator word without quotes, whether `<<-` strips
/// leading tabs from the terminator line, and whether the word was quoted
/// (`'EOF'`, `"EOF"`, `\EOF`, `E'O'F`), so that the shell expands nothing in
/// the body.
struct Marker {
    word: String,
    tabs: bool,
    quoted: bool,
}

/// The heredoc that `rest` starts with (`<<EOF`, `<<-'EOF'`): the length of
/// the operator with its marker, and the marker. Arithmetic, whose `<<` is a
/// shift, never gets here ([`UNMODELLED`]).
fn heredoc(rest: &str) -> Option<(usize, Marker)> {
    let operator = rest.strip_prefix("<<")?;
    let (tabs, word) = operator.strip_prefix('-').map_or((false, operator), |word| (true, word));
    let word = word.trim_start_matches([' ', '\t']);
    let word_len =
        word.find(|c: char| c.is_whitespace() || ";|&<>()".contains(c)).unwrap_or(word.len());
    let (marker, after) = word.split_at(word_len);
    let quoted = marker.contains(['\'', '"', '\\']);
    let marker = Marker { word: unquoted(marker), tabs, quoted };
    (!marker.word.is_empty()).then(|| (rest.len() - after.len(), marker))
}

/// The length of the heredoc bodies at the start of `text`, through the
/// terminator line of the last of `markers`; None if a terminator is missing.
fn heredoc_bodies(text: &str, markers: &[Marker]) -> Option<usize> {
    let mut len = 0;
    for Marker { word: marker, tabs, .. } in markers {
        loop {
            let line = text[len..].split_inclusive('\n').next()?;
            len += line.len();
            let line = line.strip_suffix('\n').unwrap_or(line);
            let line = if *tabs { line.trim_start_matches('\t') } else { line };
            if line == marker {
                break;
            }
        }
    }
    Some(len)
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

/// One of the `words` after `find` is an action primary ([`FIND_ACTIONS`]).
fn runs_find_action<'a>(mut words: impl Iterator<Item = &'a str>) -> bool {
    words.any(|w| FIND_ACTIONS.iter().any(|a| w.starts_with(a)))
}

/// The subcommand in the `words` after `git`: the first word that is no
/// global option or argument of one (`git -C . clean` → `clean`).
fn git_subcommand<'a>(mut words: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    while let Some(word) = words.next() {
        if GIT_GLOBAL_ARGUMENTS.contains(&word) {
            words.next();
        } else if !word.starts_with('-') {
            return Some(word);
        }
    }
    None
}

/// A sed ([`is_sed`]) followed by a short-flag group with an in-place flag
/// (`i`, BSD `I`) before any letter whose argument is the rest of the group
/// (`-ni`, `-Ei.bak`, but not `-es/i/j/`), or by `--in-place`, also
/// abbreviated (`--in`). `command` keeps its case: lower-cased, `-E -i`
/// would read as `-e` with the script `-i`.
fn edits_in_place(command: &str) -> bool {
    segments(command).any(|segment| {
        let mut words = segment.split_whitespace();
        words.any(is_sed)
            && words.any(|word| {
                let in_place = |group: &str| {
                    group.chars().take_while(|c| !"ef".contains(*c)).any(|c| "iI".contains(c))
                };
                is_option(word, in_place, &["--in-place"])
            })
    })
}

/// The script in sed's `arguments` writes a file or runs a command, or the
/// guard cannot read it ([`sed_scripts`]).
fn sed_command_writes(arguments: &[Word]) -> bool {
    sed_scripts(arguments).is_none_or(|scripts| {
        scripts
            .iter()
            .any(|script| sed_script_writes(script, false) || sed_script_writes(script, true))
    })
}

/// `word` names sed or gsed, also by path.
fn is_sed(word: &str) -> bool {
    SED.iter().any(|p| program_name(word).eq_ignore_ascii_case(p))
}

/// A `sed` that a naive split of `command` puts in command position is no
/// sed word of the tokenizer's `commands` in any reading: the tokenizer lost
/// it (a quote in a comment, an unmodelled construct), so its script is
/// unknown (fail closed, ADR-0049).
fn hides_sed(command: &str, commands: &[Vec<Word>]) -> bool {
    naive_seds(command).into_iter().any(|position| {
        !commands.iter().flatten().any(|word| word.start == position && is_sed(&word.text))
    })
}

/// Where a naive split of `command`, ignoring quotes, puts `sed` or `gsed`
/// (also by path, quotes removed) in command position: at the start of a line
/// or after `;`, `|`, `&`, `(` (also `$(`), a backtick, or the words `{`,
/// `then`, `do`, `else`, with blanks between.
fn naive_seds(command: &str) -> Vec<usize> {
    let mut seds = Vec::new();
    let (mut position, mut start) = (true, None);
    for (i, c) in command.char_indices().chain([(command.len(), '\n')]) {
        let boundary = c.is_whitespace() || ";|&()`".contains(c);
        if let Some(from) = start.filter(|_| boundary) {
            let word = &command[from..i];
            if position && is_sed(&unquoted(word)) {
                seds.push(from);
            }
            position = position && matches!(word, "{" | "then" | "do" | "else");
            start = None;
        }
        if c == '\n' || ";|&(`".contains(c) {
            position = true;
        } else if c == ')' {
            position = false;
        } else if !boundary && start.is_none() {
            start = Some(i);
        }
    }
    seds
}

/// A word of a simple command ([`simple_commands`]): its text with quotes and
/// backslashes resolved, where it starts, and whether the shell expands it (a `$` outside
/// single quotes, a backtick, `$'…'`), so that the text is not what the
/// program gets.
#[derive(Default)]
struct Word {
    text: String,
    expanded: bool,
    /// An unquoted glob or brace character (`*?[{`) makes it a pattern.
    pattern: bool,
    /// The byte offset in the command where the word starts.
    start: usize,
}

/// The simple commands of `command` as words, with quotes and backslashes
/// resolved as the shell does ([`segments`] splits inside quotes too).
/// `;|&()` and newlines outside quotes separate them, and so do command
/// substitutions (`$(…)`, backticks), also inside double quotes. A `${…}` is
/// followed like a quote, one word with its blanks and quotes. Redirections
/// (`2> file`, `&>file`, `<<EOF`) are no words. With `comments`, a `#` that
/// may start a comment ([`starts_comment`]) ends the line; without, comments
/// are words like any other. The callers take both readings, so that a
/// comment never hides a command (ADR-0049). Each word records where it starts
/// in `command`.
fn simple_commands(command: &str, comments: bool) -> Tokens {
    let (mut commands, mut words, mut targets, mut qualifier) =
        (Vec::new(), Vec::new(), Vec::new(), false);
    // The word being read, and the operator whose target it is.
    let (mut word, mut target): (Option<Word>, Option<String>) = (None, None);
    // Open quotes and substitutions, innermost last: `'`, `"`, `$` for
    // `$'…'`, `(` for `(…)` and `$(…)`, `` ` ``, and for `${…}` `{` (or `}`
    // inside double quotes, where `'` is literal).
    let mut open: Vec<char> = Vec::new();
    // The character before `c` (`(` for `$(`), where the word being read
    // starts, and the first redirection of the line (for heredoc bodies).
    let (mut previous, mut start, mut line_targets) = (None, 0, 0);
    let peek = |chars: &std::str::Chars<'_>| chars.clone().next();
    let mut chars = command.chars();
    loop {
        let i = command.len() - chars.as_str().len();
        let Some(c) = chars.next() else { break };
        if word.is_none() {
            start = i;
        }
        let top = open.last().copied();
        let unquoted = matches!(top, None | Some('(' | '`'));
        let expanding = !matches!(top, Some('\'' | '$'));
        let substitution = expanding && (c == '`' || (c == '$' && peek(&chars) == Some('(')));
        let opens = opened(top, c, peek(&chars));
        let separator = unquoted && (c.is_whitespace() || ";|&()<>".contains(c));
        if substitution {
            // The output of the substitution is (part of) a word.
            word.get_or_insert_default().expanded = true;
        }
        if substitution || separator {
            // An unquoted `(` glued to a word: a zsh glob qualifier or group.
            qualifier |= c == '(' && word.is_some();
            let word = word.take().map(|w| Word { start, ..w });
            end_word(&mut words, &mut targets, word, &mut target, matches!(c, '<' | '>'));
        }
        match (top, c) {
            (Some('\''), '\'') | (Some('"'), '"') | (Some('$'), '\'') | (Some('{' | '}'), '}') => {
                open.pop();
            }
            // ANSI-C quoting: `\'` does not end it; escapes stay undecoded.
            (Some('$'), _) => {
                let word = &mut word.get_or_insert_default().text;
                word.push(c);
                word.extend((c == '\\').then(|| chars.next()).flatten());
            }
            (_, '\\') if top != Some('\'') => match chars.next() {
                Some('\n') | None => {}
                // In double quotes a backslash escapes only these.
                Some(next) if !matches!(top, Some('"' | '}')) || "$`\"\\".contains(next) => {
                    word.get_or_insert_default().text.push(next);
                }
                Some(next) => word.get_or_insert_default().text.extend(['\\', next]),
            },
            _ if substitution => {
                commands.push(std::mem::take(&mut words));
                target = None;
                match (c, top) {
                    ('$', _) => {
                        chars.next();
                        open.push('(');
                    }
                    (_, Some('`')) => _ = open.pop(),
                    _ => open.push('`'),
                }
            }
            (_, '#') if comments && starts_comment(unquoted, word.is_some(), previous) => {
                skip_while(&mut chars, |c| c != '\n');
            }
            _ if opens.is_some() => {
                open.extend(opens);
                let word = word.get_or_insert_default();
                if c == '$' {
                    chars.next();
                    word.expanded = true;
                }
            }
            _ if separator => match c {
                '<' | '>' => target = Some(redirection_operator(c, &mut chars)),
                '&' if peek(&chars) == Some('>') => {}
                ';' | '|' | '&' | '(' | ')' | '\n' => {
                    commands.push(std::mem::take(&mut words));
                    if c == '\n' {
                        line_targets =
                            skip_bodies(&mut chars, &targets, line_targets, &mut commands);
                    }
                    // `<(…)` and `>(…)` are commands, not targets.
                    target = None;
                    match (c, top) {
                        ('(', _) => open.push('('),
                        (')', Some('(')) => {
                            open.pop();
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            _ => {
                let word = word.get_or_insert_default();
                word.text.push(c);
                word.expanded |= c == '$' && top != Some('\'');
                word.pattern |= unquoted && GLOB.contains(&c);
            }
        }
        previous = Some(if substitution && c == '$' { '(' } else { c });
    }
    let word = word.map(|w| Word { start, ..w });
    end_word(&mut words, &mut targets, word, &mut target, false);
    commands.push(words);
    Tokens { commands, targets, qualifier }
}

/// What [`simple_commands`] reads from a command: its simple commands as
/// words, its redirections as operator (`>`, `>>`, `2>`'s `>`, `>&`, `<<`)
/// and target word, and whether an unquoted `(` is glued to a word: a zsh
/// glob qualifier or group (`*(e:'…':)`, `x(#qe:…:)`) can run code.
struct Tokens {
    commands: Vec<Vec<Word>>,
    targets: Vec<(String, Word)>,
    qualifier: bool,
}

/// A redirection writes a file: an operator with `>` (also `<>`, `>|`) to a
/// target other than `/dev/null`, or a duplication (`>&2`, `2>&1`) of anything
/// but a descriptor number or `-`.
fn writes_file(operator: &str, target: &Word) -> bool {
    let descriptor = target.text == "-" || target.text.bytes().all(|b| b.is_ascii_digit());
    operator.contains('>')
        && (target.expanded || target.text != "/dev/null")
        && !(operator.contains('&') && descriptor && !target.text.is_empty())
}

/// `command` redirects output to a file in either comment reading (a quoted
/// `>` is none); a command with a construct the tokenizer does not model
/// ([`models_prose`]) is read as text: every `>` but the harmless ones of
/// [`without_null_redirections`] counts (fail closed, ADR-0049).
fn redirects_to_file(command: &str) -> bool {
    if !models_prose(command) {
        return without_null_redirections(command).contains('>');
    }
    [false, true].into_iter().any(|comments| {
        simple_commands(command, comments)
            .targets
            .iter()
            .any(|(operator, target)| writes_file(operator, target))
    })
}

/// The redirection operator that starts with `first` (`>`, `>>`, `>|`, `>&`,
/// `<<-` …), advancing `chars` past the rest of it.
fn redirection_operator(first: char, chars: &mut std::str::Chars<'_>) -> String {
    let rest = chars.as_str();
    skip_while(chars, |c| "<>&|-".contains(c));
    format!("{first}{}", &rest[..rest.len() - chars.as_str().len()])
}

/// Skips in `chars` the bodies of the heredocs among `targets` from `line`,
/// the redirections of the line that just ended, and adds their lines to
/// `commands` as words split at blanks: data or code the tokenizer does not
/// read, but whose words may name paths (taken literally). Without a
/// terminator nothing is skipped and the rest is read as commands. Returns
/// where the next line's redirections start.
fn skip_bodies(
    chars: &mut std::str::Chars<'_>,
    targets: &[(String, Word)],
    line: usize,
    commands: &mut Vec<Vec<Word>>,
) -> usize {
    let markers: Vec<Marker> = targets[line..]
        .iter()
        .filter(|(operator, _)| operator.starts_with("<<") && !operator.starts_with("<<<"))
        .map(|(operator, word)| Marker {
            word: word.text.clone(),
            tabs: operator.starts_with("<<-"),
            quoted: false,
        })
        .collect();
    let rest = chars.as_str();
    let Some(len) = heredoc_bodies(rest, &markers).filter(|_| !markers.is_empty()) else {
        return targets.len();
    };
    commands.extend(rest[..len].lines().map(|line| {
        line.split_whitespace()
            .map(|text| Word { text: text.to_owned(), ..Word::default() })
            .collect()
    }));
    *chars = rest[len..].chars();
    targets.len()
}

/// Advances `chars` past the characters that satisfy `f`.
fn skip_while(chars: &mut std::str::Chars<'_>, f: impl Fn(char) -> bool) {
    while chars.clone().next().is_some_and(&f) {
        chars.next();
    }
}

/// Ends the `word` being read at a separator or substitution: it joins
/// `targets` with its operator if it is the `target` of a redirection, is
/// dropped if it is the descriptor number before a `redirection` operator
/// (`2>`), and joins `words` otherwise.
fn end_word(
    words: &mut Vec<Word>,
    targets: &mut Vec<(String, Word)>,
    word: Option<Word>,
    target: &mut Option<String>,
    redirection: bool,
) {
    let Some(word) = word else { return };
    match target.take() {
        Some(operator) => targets.push((operator, word)),
        None if redirection && word.text.bytes().all(|b| b.is_ascii_digit()) => {}
        None => words.push(word),
    }
}

/// A `#` after `previous` may start a comment for the shell: outside quotes
/// (`unquoted`), not inside a word, at the start or after whitespace or one of
/// `;|&()<>` and a backtick (bash and zsh read `true;#x`, `(true)#x` and
/// `$(#x` as comments).
fn starts_comment(unquoted: bool, in_word: bool, previous: Option<char>) -> bool {
    unquoted && !in_word && previous.is_none_or(|p| p.is_whitespace() || ";|&()<>`".contains(p))
}

/// What `c`, followed by `next`, opens inside `top` in [`simple_commands`]: a
/// quote, `$` for `$'…'`, or `{` for `${…}` (`}` inside double quotes, where
/// `'` is literal and `"` nests).
fn opened(top: Option<char>, c: char, next: Option<char>) -> Option<char> {
    let unquoted = matches!(top, None | Some('(' | '`'));
    let brace = top == Some('{');
    match (c, next) {
        ('$', Some('{')) if !matches!(top, Some('\'' | '$')) => {
            Some(if matches!(top, Some('"' | '}')) { '}' } else { '{' })
        }
        ('$', Some('\'')) if unquoted || brace => Some('$'),
        ('"', _) if unquoted || matches!(top, Some('{' | '}')) => Some('"'),
        ('\'', _) if unquoted || brace => Some('\''),
        _ => None,
    }
}

/// The scripts in sed's `arguments`: the `-e`/`--expression` scripts joined
/// by newlines, as sed joins them, and the first non-option argument if it
/// comes before any `-e`/`-f`. BSD sed takes that one as the script and the
/// rest as files; GNU sed also reads options after it. None if a script is no
/// literal: from a file (`-f`, `--file`), expanded by the shell, or xargs' and
/// find's placeholder `{}`.
fn sed_scripts(arguments: &[Word]) -> Option<Vec<String>> {
    let (mut expressions, mut first, mut script_option) = (Vec::new(), None, false);
    let mut words = arguments.iter();
    let literal = |word: &Word| (!word.expanded && word.text != "{}").then(|| word.text.clone());
    while let Some(word) = words.next() {
        let (name, value) =
            word.text.split_once('=').map_or((word.text.as_str(), None), |(n, v)| (n, Some(v)));
        let long = |option: &str, min: usize| name.len() >= min && option.starts_with(name);
        if word.text == "--" {
            if !script_option && first.is_none() {
                first = words.next();
            }
            break;
        } else if long("--file", 4) {
            return None;
        } else if long("--expression", 3) {
            let argument = match value {
                Some(value) => literal(word).map(|_| value.to_owned()),
                None => literal(words.next()?),
            };
            expressions.push(argument?);
            script_option = true;
        } else if long("--line-length", 3) {
            if value.is_none() {
                words.next();
            }
        } else if let Some(flags) = short_flags(&word.text) {
            // getopt: `e`, `f` and GNU's `l` take the rest of the group or the
            // next word, `i`/`I` the rest of the group. BSD's `-l` takes none,
            // so only a number is skipped.
            let Some(at) = flags.find(['e', 'f', 'l', 'i', 'I']) else { continue };
            let (letter, rest) = (&flags[at..=at], &flags[at + 1..]);
            if letter == "f" {
                return None;
            } else if letter == "e" {
                let argument = match rest {
                    "" => literal(words.next()?),
                    rest => literal(word).map(|_| rest.to_owned()),
                };
                expressions.push(argument?);
                script_option = true;
            } else if letter == "l"
                && rest.is_empty()
                && words.clone().next().is_some_and(|n| n.text.bytes().all(|b| b.is_ascii_digit()))
            {
                words.next();
            }
        } else if !word.text.starts_with("--") && !script_option && first.is_none() {
            first = Some(word);
        }
    }
    let first = match first {
        Some(word) => Some(literal(word)?),
        None => None,
    };
    Some(
        first
            .into_iter()
            .chain((!expressions.is_empty()).then(|| expressions.join("\n")))
            .collect(),
    )
}

/// A sed script that writes a file or runs a command: the command `w`/`W` or
/// GNU's `e`, or the flag `w`/`e` of `s`; a `w`/`e` in a regex, replacement,
/// text, `y` list or label is none. Follows the grammar of BSD and GNU sed,
/// leniently; `brackets` reads a bracket expression in a regex as BSD sed
/// does, where the delimiter does not end it (`s/[/]/x/`). A syntax error ends
/// the scan: sed then runs nothing.
fn sed_script_writes(script: &str, brackets: bool) -> bool {
    let s: Vec<char> = script.chars().collect();
    let mut i = 0;
    loop {
        while s.get(i).is_some_and(|c| c.is_whitespace() || *c == ';') {
            i += 1;
        }
        let Some(after) = sed_address(&s, i, brackets) else { return false };
        let Some(&command) = s.get(after) else { return false };
        i = after + 1;
        let end = match command {
            'w' | 'W' | 'e' => return true,
            '{' | '}' => Some(i),
            '#' | 'r' | 'R' => Some(sed_line_end(&s, i, false)),
            'a' | 'i' | 'c' => Some(sed_line_end(&s, i, true)),
            // A label ends at a blank or `;` (GNU) or the line (BSD).
            ':' | 'b' | 't' | 'T' | 'v' => {
                let start = i + count_while(&s[i..], |c| matches!(c, ' ' | '\t'));
                Some(start + count_while(&s[start..], |c| !c.is_whitespace() && c != ';'))
            }
            's' | 'y' => {
                let Some(&delimiter) = s.get(i).filter(|c| !matches!(c, '\n' | '\\')) else {
                    return false;
                };
                let Some(mut j) = sed_delimited(&s, i + 1, delimiter, brackets && command == 's')
                    .and_then(|j| sed_delimited(&s, j, delimiter, false))
                else {
                    return false;
                };
                if command == 's' {
                    j += count_while(&s[j..], |c| c.is_ascii_digit() || "gpiImM \t".contains(c));
                    if matches!(s.get(j), Some('w' | 'e')) {
                        return true;
                    }
                }
                sed_end_of_command(&s, j)
            }
            'l' | 'L' | 'q' | 'Q' => {
                sed_end_of_command(&s, i + count_while(&s[i..], |c| c.is_ascii_digit() || c == ' '))
            }
            '=' | 'd' | 'D' | 'F' | 'g' | 'G' | 'h' | 'H' | 'n' | 'N' | 'p' | 'P' | 'x' | 'z' => {
                sed_end_of_command(&s, i)
            }
            _ => None,
        };
        let Some(end) = end else { return false };
        i = end;
    }
}

/// The index after the address or range of a sed command at `i`, with any
/// `!`; `i` itself if there is none. None at a syntax error.
fn sed_address(s: &[char], i: usize, brackets: bool) -> Option<usize> {
    let mut i = sed_address_part(s, i, brackets)?;
    if s.get(i) == Some(&',') {
        i += 1 + count_while(&s[i + 1..], |c| c == ' ');
        i = match s.get(i) {
            Some('+' | '~') => i + 1 + count_while(&s[i + 1..], |c| c.is_ascii_digit()),
            _ => sed_address_part(s, i, brackets)?,
        };
    }
    Some(i + count_while(&s[i..], |c| matches!(c, ' ' | '\t' | '!')))
}

/// One sed address at `i`: a line `N` (GNU `N~M`), `$`, `/re/` or `\cREc`
/// with GNU's flags `I` and `M`.
fn sed_address_part(s: &[char], i: usize, brackets: bool) -> Option<usize> {
    let (delimiter, start) = match s.get(i) {
        Some('/') => ('/', i + 1),
        Some('\\') => (*s.get(i + 1)?, i + 2),
        Some('$') => return Some(i + 1),
        _ => return Some(i + count_while(&s[i..], |c| c.is_ascii_digit() || c == '~')),
    };
    let end = sed_delimited(s, start, delimiter, brackets)?;
    Some(end + count_while(&s[end..], |c| matches!(c, 'I' | 'M')))
}

/// The index after the closing `delimiter` of a regex, replacement or `y`
/// list starting at `i`; None if an unescaped newline or the end comes first.
/// With `brackets`, a bracket expression may hold the delimiter.
fn sed_delimited(s: &[char], mut i: usize, delimiter: char, brackets: bool) -> Option<usize> {
    while let Some(&c) = s.get(i) {
        i += match c {
            _ if c == delimiter => return Some(i + 1),
            '\n' => return None,
            '\\' => 2,
            '[' if brackets => sed_bracket_len(&s[i..])?,
            _ => 1,
        };
    }
    None
}

/// The length of the bracket expression `s` starts with, as BSD sed reads it:
/// a leading `^` and `]` are literal, `[:…:]`, `[.….]` and `[=…=]` nest.
fn sed_bracket_len(s: &[char]) -> Option<usize> {
    let mut i = 1;
    i += usize::from(s.get(i) == Some(&'^'));
    i += usize::from(s.get(i) == Some(&']'));
    while let Some(&c) = s.get(i) {
        match (c, s.get(i + 1)) {
            (']', _) => return Some(i + 1),
            ('\n', _) => return None,
            ('[', Some(&d @ ('.' | ':' | '='))) => {
                i += 1 + s[i + 1..].windows(2).position(|w| w == [d, ']'])? + 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// The index after the line at `i` in a sed script, a backslash escaping the
/// next character if `escapes` (the text of `a`, `i`, `c`).
fn sed_line_end(s: &[char], mut i: usize, escapes: bool) -> usize {
    while let Some(&c) = s.get(i) {
        i += match c {
            '\n' => return i + 1,
            '\\' if escapes => 2,
            _ => 1,
        };
    }
    s.len()
}

/// `i` if a sed command may end there (blanks, then `;`, a newline, `}`, `#`
/// or the end); None at extra characters, which sed rejects.
fn sed_end_of_command(s: &[char], i: usize) -> Option<usize> {
    let i = i + count_while(&s[i..], |c| matches!(c, ' ' | '\t'));
    matches!(s.get(i), None | Some(';' | '\n' | '}' | '#')).then_some(i)
}

/// How many characters at the start of `s` satisfy `f`.
fn count_while(s: &[char], f: impl Fn(char) -> bool) -> usize {
    s.iter().take_while(|&&c| f(c)).count()
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
            && words.any(|word| is_option(word, |flags| flags.contains(flag), long))
    })
}

/// `word`, without its `=value`, is a short-flag group that `short` accepts or
/// one of the `long` options, also abbreviated (`--out`) or extended
/// (`--output-dir`); `--` alone is none.
fn is_option(word: &str, short: impl Fn(&str) -> bool, long: &[&str]) -> bool {
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    short_flags(name).is_some_and(short)
        || (name.len() > 2 && long.iter().any(|o| name.starts_with(o) || o.starts_with(name)))
}

/// The letters of a short-flag group (`-pi` → `pi`).
fn short_flags(word: &str) -> Option<&str> {
    word.strip_prefix('-').filter(|flags| !flags.starts_with('-'))
}

/// A simple command that starts with a read-only program (a leading
/// `VAR=value` is not one): `sort` without an output file or compress program
/// (GNU accepts it abbreviated to `--co`), `uniq` without an output operand,
/// `rg` without a preprocessor, `find` without actions, `git rev-parse`.
fn reads_only(segment: &str) -> bool {
    let mut words = segment.split_whitespace();
    match words.next() {
        Some("sort") => {
            !runs_with_option(segment, &["sort"], 'o', &["--output", "--compress-program"])
        }
        Some("rg") => !words.any(|w| {
            matches!(w.split_once('=').map_or(w, |(name, _)| name), "--pre" | "--pre-glob")
        }),
        Some("uniq") => words.filter(|w| !w.starts_with('-')).count() <= 1,
        Some("find") => !runs_find_action(words),
        // `-C`/`--compile` writes a compiled magic file.
        Some("file") => !words.any(|w| is_option(w, |group| group.contains('C'), &["--compile"])),
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
/// A directory also counts without its slash ([`bare_positions`] in `shell`,
/// the command [`without_prose`]). `command` and `shell` are lower-cased.
fn harness_in_shell(root: &Path, command: &str, shell: &str) -> Option<&'static str> {
    let mut roots: Vec<String> = [root.to_path_buf(), canonical(root)]
        .iter()
        .map(|r| r.to_string_lossy().trim_end_matches('/').to_lowercase())
        .filter(|r| !r.is_empty())
        .collect();
    roots.extend(ROOT_VARIABLES.iter().map(|v| v.to_lowercase()));
    HARNESS_IN_SHELL.iter().copied().find(|fragment| {
        let fragment = fragment.to_lowercase();
        let name = fragment.trim_end_matches('/');
        let dir = paths::HARNESS.iter().any(|h| h.strip_suffix('/') == Some(name)).then_some(name);
        let path = dir.map_or_else(|| fragment.clone(), |dir| format!("{dir}/"));
        command.match_indices(&path).any(|(i, _)| refers_to_repo(&command[..i], &roots))
            || dir.is_some_and(|dir| {
                bare_positions(shell, dir).any(|i| refers_to_repo(&shell[..i], &roots))
            })
    })
}

/// Where the directory name `dir` stands in `shell` as a whole argument: a
/// whole file name followed by the end, whitespace or [`ARGUMENT_ENDS`], and
/// not cargo's package `xtask` ([`is_cargo_package`]).
fn bare_positions<'a>(shell: &'a str, dir: &'a str) -> impl Iterator<Item = usize> + 'a {
    file_name_positions(shell, dir).filter(move |&i| {
        shell[i + dir.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || ARGUMENT_ENDS.contains(c))
            && !(dir == "xtask" && is_cargo_package(&shell[..i]))
    })
}

/// The word after `before` names a cargo package: cargo, also by path, is the
/// program of the segment and the word is its subcommand (`cargo xtask check`)
/// or follows `-p`, `--package` or `--package=` (`cargo test -p xtask`).
fn is_cargo_package(before: &str) -> bool {
    let segment = before.rsplit(['|', ';', '&', '\n', '(', '`']).next().unwrap_or_default();
    let words = command_words(segment);
    let Some((program, arguments)) = words.split_first() else { return false };
    let program = program.trim_start_matches(['\'', '"']);
    program_name(program) == "cargo"
        && arguments.last().is_none_or(|w| matches!(*w, "-p" | "--package" | "--package="))
}

/// The program of the simple command `segment` and its arguments; for inline
/// code of a shell (`bash -c 'cargo xtask check'`) those of the code.
fn command_words(segment: &str) -> Vec<&str> {
    let words: Vec<&str> = segment.split_whitespace().collect();
    let shell = words.first().is_some_and(|w| SHELLS.contains(&program_name(w)));
    match words.iter().position(|w| *w == "-c") {
        Some(c) if shell => words[c + 1..].to_vec(),
        _ => words,
    }
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
    if (lower_rel.starts_with(paths::FIXTURES) && root.join(rel).exists())
        || holds_fixtures(rel, false)
    {
        return Err(fixture_message(rel));
    }
    Ok(())
}

fn fixture_message(rel: &str) -> String {
    format!(
        "`{rel}` is a format fixture and immutable (ADR-0030). New format version → new \
         fixture + migration."
    )
}

/// The kind of file a role agent writes; None for any other context.
fn agent_role(input: &Value) -> Option<Kind> {
    match input["agent_type"].as_str().unwrap_or_default() {
        TEST_AUTHOR => Some(Kind::Test),
        IMPLEMENTER => Some(Kind::Code),
        _ => None,
    }
}

/// The role check for a path named in a shell command ([`named_kinds`]): a
/// directory that holds tests and code conflicts with every role; a context
/// without an agent type is denied before it would be bound to both.
/// `pattern`: the shell expands globs and braces in it.
fn check_named_role(root: &Path, input: &Value, rel: &str, pattern: bool) -> Result<(), String> {
    match named_kinds(rel, pattern) {
        [kind] => check_role_as(root, input, rel, *kind),
        _ if agent_role(input).is_none() => Err(format!(
            "`{rel}` holds tests and production code, which no agent context changes together \
             (ADR-0039). Name the files, or delegate to `{TEST_AUTHOR}` and `{IMPLEMENTER}`."
        )),
        kinds => kinds.iter().try_for_each(|kind| check_role_as(root, input, rel, *kind)),
    }
}

/// The kinds of file a path named in a shell command can be. A `pattern`
/// with a glob or brace ([`GLOB`]) anywhere: a test below a literal `tests` directory
/// or `models`, both if it may reach a source tree, else neither. Otherwise a
/// file (a last component with a `.`) its [`paths::kind`]; a directory below
/// `tests` or `models` a test, any other source-tree directory both
/// (`crates/<crate>/src` and its modules hold unit tests in `tests.rs`;
/// `crates`, a crate root, `xtask`).
fn named_kinds(rel: &str, pattern: bool) -> &'static [Kind] {
    let lower = rel.to_lowercase();
    if let Some(glob) = lower.find(GLOB).filter(|_| pattern) {
        // The literal directories before the pattern decide.
        let literal = &lower[..glob];
        let dirs: Vec<&str> = literal.split('/').collect();
        let dirs = &dirs[..dirs.len() - 1];
        let source = ["crates/", "xtask/", "third_party/"];
        return if dirs.first() == Some(&"models")
            || dirs.iter().any(|d| matches!(*d, "tests" | "proptest-regressions"))
        {
            &[Kind::Test]
        } else if source.iter().any(|s| literal.starts_with(s) || s.starts_with(literal)) {
            &[Kind::Code, Kind::Test]
        } else {
            &[Kind::Other]
        };
    }
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let kind = if name.contains('.') {
        paths::kind(rel)
    } else {
        if lower == "models" {
            return &[Kind::Test];
        }
        match paths::kind(&format!("{lower}/x.rs")) {
            Kind::Code => return &[Kind::Code, Kind::Test],
            kind => kind,
        }
    };
    match kind {
        Kind::Test => &[Kind::Test],
        Kind::Code => &[Kind::Code],
        Kind::Other => &[Kind::Other],
    }
}

/// One context writes tests or code, never both (ADR-0039). Role agents are
/// fixed by their type; any other context is bound by its first write.
fn check_role(root: &Path, input: &Value, rel: &str) -> Result<(), String> {
    check_role_as(root, input, rel, paths::kind(rel))
}

/// [`check_role`] for `rel` as a file of the kind `wanted`.
fn check_role_as(root: &Path, input: &Value, rel: &str, wanted: Kind) -> Result<(), String> {
    if wanted == Kind::Other {
        return Ok(());
    }
    let role = match agent_role(input) {
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
