//! Claude Code `PreToolUse` guard (ADR-0037, ADR-0039, ADR-0043). Reads the hook
//! input JSON and returns the reason for a denial:
//! - harness files are read-only unless the human started the session with
//!   `QOOB3D_HARNESS_UNLOCK=1` ([`Session::unlocked`]); the unlock lifts nothing else,
//! - files that can set the environment of the hooks, and `.git/`, are never
//!   written, so that no agent can unlock itself; a shell command naming `.git`
//!   is denied unless it only reads or names `.git` only to exclude it,
//! - format fixtures are never modified,
//! - one agent context writes either tests or production code, never both,
//! - shell commands must not bypass the git hooks or write harness files; a
//!   harness directory counts also when named without its trailing slash, as a
//!   whole shell argument outside comments (from a `#` that starts a word) and
//!   outside heredoc bodies that no shell or interpreter reads, also through a
//!   pipeline continued over lines (ADR-0049).
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
/// Shell fragments that (may) write files; also `dd` and `install` as words,
/// the programs with an output option in [`writes`], and the in-place editors
/// and inline-code interpreters of [`runs_with_flag`].
const WRITES: &[&str] = &[
    ">",
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
/// Programs that rewrite the files they name ([`runs_with_flag`]): names,
/// in-place short flags, short flags taking the rest of the group as
/// argument, long options. sed: `-i[ext]`, BSD `-I ext`, GNU `--in-place`;
/// perl/ruby: `-i[ext]`; awk: `-i inplace` or `--include=inplace` loads gawk's
/// inplace extension.
const IN_PLACE: &[(&[&str], &str, &str, &[&str])] = &[
    (&["sed", "gsed"], "iI", "ef", &["--in-place"]),
    (&["perl", "ruby"], "i", "IMmr", &[]),
    (&["awk", "gawk"], "i", "vFfeE", &["--include"]),
];
/// Interpreters running code from the command line, which can write any file
/// ([`runs_with_flag`]), in the shape of [`IN_PLACE`]. ruby: `-e`; perl:
/// `-e`/`-E`; node: `-e`/`--eval`, `-p`/`--print`; deno: the subcommand
/// `eval`, matched like a long option; sh/bash/zsh: `-c`. The argument
/// letters are the options taking an attached argument (`-rdigest`,
/// `-I/usr/share/perl5`, ruby's `-Ke`). python is a writer in [`WRITES`].
const INLINE_CODE: &[(&[&str], &str, &str, &[&str])] = &[
    (&["ruby"], "e", "CEFIKrTWx", &[]),
    (&["perl"], "eE", "CFIMmVx", &[]),
    (&["node"], "ep", "", &["--eval", "--print"]),
    (&["deno"], "", "", &["eval"]),
    (SHELLS, "c", "", &[]),
];
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
/// Characters after which a path starts a new shell token.
const TOKEN_SEPARATORS: &str = "\"'=(;|&><`";
/// Characters after which a bare directory name is still a whole argument:
/// separators and redirections end it, a glob expands it to the directory
/// itself (`xtask*`). Not `:` or `,`, which make it prose (`xtask: fix`).
const ARGUMENT_ENDS: &str = ";|&)<>`*?[";
/// Unquoted, unescaped characters after which a `#` starts a comment. Not `)`:
/// after `$(…)` or `$((…))` the word goes on (`$(true)#`).
const COMMENT_STARTS: &str = ";|&(";

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
    let normalised = unquoted(command);
    let forms = [command.to_lowercase(), normalised.to_lowercase()];
    check_bypasses(&forms[1])?;
    let read = is_pure_read(&normalised);
    if let Some(file) = hook_environment_in_shell(session, &forms)
        && !read
    {
        return Err(self_unlock_message(&file));
    }
    check_git_dir_in_shell(&forms, &normalised, read)?;
    let writes = forms.iter().any(|form| writes(&without_null_redirections(form)))
        || [command, normalised.as_str()]
            .into_iter()
            .any(|form| runs_with_flag(form, IN_PLACE) || runs_with_flag(form, INLINE_CODE));
    if session.unlocked || !writes {
        return Ok(());
    }
    let shell = without_prose(command);
    let shell_forms = [shell.to_lowercase(), unquoted(&shell).to_lowercase()];
    if let Some(path) =
        forms.iter().zip(&shell_forms).find_map(|(form, shell)| harness_in_shell(root, form, shell))
    {
        return Err(format!(
            "Shell command could modify the harness file `{path}` (ADR-0037). Harness changes \
             only in a session unlocked by the human (QOOB3D_HARNESS_UNLOCK=1)."
        ));
    }
    Ok(())
}

/// `command` with quotes and backslashes removed.
fn unquoted(command: &str) -> String {
    command.chars().filter(|c| !matches!(c, '\'' | '"' | '\\')).collect()
}

/// `command` without what the shell reads as no words: heredoc bodies (from
/// the line after `<<MARKER` through the line `MARKER`) and comments (from an
/// unquoted, unescaped `#` that starts a word to the end of its line).
/// Quotes, backslashes and `$(…)` in double quotes are followed, so a quoted
/// `#` or `<<` starts neither; a `#` after a closing quote, `)` or an escaped
/// character is part of a word. A heredoc without its terminator line stays,
/// and so do the bodies of a line that [`feeds_interpreter`] (ADR-0049); a
/// backslash-newline continues the line.
fn without_prose(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    // Open quotes and parentheses, innermost last.
    let mut open: Vec<char> = Vec::new();
    let mut markers: Vec<(String, bool)> = Vec::new();
    // Where the current line starts in `out`; whether the next character starts a word.
    let (mut line_start, mut word_start) = (0, true);
    let mut i = 0;
    while let Some(c) = command[i..].chars().next() {
        let rest = &command[i..];
        let mut end = i + c.len_utf8();
        match (open.last().copied(), c) {
            (Some('\''), '\'') | (Some('"'), '"') | (Some('('), ')') => {
                open.pop();
            }
            (Some('\''), _) => {}
            (_, '\\') => end += rest[1..].chars().next().map_or(0, char::len_utf8),
            (Some('"'), '$') if rest.starts_with("$(") => {
                open.push('(');
                end += 1;
            }
            (Some('"'), _) => {}
            (_, '\'' | '"' | '(') => open.push(c),
            (_, '#') if word_start => {
                i += rest.find('\n').unwrap_or(rest.len());
                continue;
            }
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
                if let Some(len) = heredoc_bodies(&command[end..], &markers)
                    && !feeds_interpreter(&out[line_start..], &command[end + len..])
                {
                    end += len;
                }
                markers.clear();
                out.push('\n');
                (line_start, word_start) = (out.len(), true);
                i = end;
                continue;
            }
            _ => {}
        }
        out.push_str(&command[i..end]);
        let outside_quotes = matches!(open.last(), None | Some('('));
        if outside_quotes && c == '\n' {
            line_start = out.len();
        }
        word_start = outside_quotes && (c.is_whitespace() || COMMENT_STARTS.contains(c));
        i = end;
    }
    out
}

/// The heredoc `line` (comments removed) feeds a shell or interpreter, so its
/// heredoc bodies are code, not data: a pipeline of `line` with a heredoc has
/// a word that [`is_interpreter`] (`bash <<EOF`, `cat <<EOF | /bin/sh`; any
/// word counts, `env bash <<EOF`), or `line` ends in a pipe that continues in
/// `after`, the text after the terminator line, into an interpreter
/// ([`pipes_into_interpreter`]).
fn feeds_interpreter(line: &str, after: &str) -> bool {
    line.split([';', '&']).filter(|pipeline| pipeline.contains("<<")).any(|pipeline| {
        pipeline.split(|c: char| c.is_whitespace() || "|()`".contains(c)).any(is_interpreter)
    }) || (line.trim_end().trim_end_matches('&').ends_with('|') && pipes_into_interpreter(after))
}

/// The pipeline at the start of `text` has a stage whose program
/// [`is_interpreter`] (`bash`, `grep x | sh`); an argument does not count
/// (`grep -c bash`).
fn pipes_into_interpreter(text: &str) -> bool {
    let pipeline = text.trim_start().split([';', '&', '\n']).next().unwrap_or_default();
    pipeline.split('|').any(|stage| stage.split_whitespace().next().is_some_and(is_interpreter))
}

/// `word`, also by path or quoted, names a program that runs the text it
/// reads: the programs of [`INLINE_CODE`], python (a writer in [`WRITES`];
/// also versioned, `python3.12`), `xargs`, and the builtins `eval`, `source`
/// and `.`.
fn is_interpreter(word: &str) -> bool {
    let word = unquoted(word).to_ascii_lowercase();
    let name = word.rsplit('/').next().unwrap_or_default();
    INLINE_CODE.iter().any(|(programs, ..)| programs.contains(&name))
        || name
            .strip_prefix("python")
            .is_some_and(|version| version.chars().all(|c| c.is_ascii_digit() || c == '.'))
        || matches!(name, "xargs" | "eval" | "source" | ".")
}

/// The heredoc that `rest` starts with (`<<EOF`, `<<-'EOF'`): the length of
/// the operator with its marker, and the marker without quotes with whether
/// `<<-` strips leading tabs from the terminator line. None for `<<` in
/// arithmetic, whose "marker" a `)` follows (`$((1 << 2))`).
fn heredoc(rest: &str) -> Option<(usize, (String, bool))> {
    let operator = rest.strip_prefix("<<")?;
    let (tabs, word) = operator.strip_prefix('-').map_or((false, operator), |word| (true, word));
    let word = word.trim_start_matches([' ', '\t']);
    let word_len =
        word.find(|c: char| c.is_whitespace() || ";|&<>()".contains(c)).unwrap_or(word.len());
    let marker = unquoted(&word[..word_len]);
    let after = &word[word_len..];
    if marker.is_empty() || after.trim_start_matches([' ', '\t']).starts_with(')') {
        return None;
    }
    Some((rest.len() - after.len(), (marker, tabs)))
}

/// The length of the heredoc bodies at the start of `text`, through the
/// terminator line of the last of `markers`; None if a terminator is missing.
fn heredoc_bodies(text: &str, markers: &[(String, bool)]) -> Option<usize> {
    let mut len = 0;
    for (marker, tabs) in markers {
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

/// A lower-cased command that (may) write files; in-place editors and
/// inline-code interpreters in [`runs_with_flag`] need the case.
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

/// A program of `table` ([`IN_PLACE`], [`INLINE_CODE`]), also by path,
/// followed by a short-flag group with one of its flag letters before any
/// letter whose argument is the rest of the group (`-pi`, `-Ei.bak`, but not
/// `-es/i/j/`), or by its long option, also abbreviated (`--in`). `command`
/// keeps its case: lower-cased, sed's `-E -i` would read as `-e` with the
/// script `-i`.
fn runs_with_flag(command: &str, table: &[(&[&str], &str, &str, &[&str])]) -> bool {
    segments(command).any(|segment| {
        table.iter().any(|(programs, flags, argument, long)| {
            let mut words = segment.split_whitespace();
            words.any(|w| {
                w.rsplit('/')
                    .next()
                    .is_some_and(|name| programs.iter().any(|p| name.eq_ignore_ascii_case(p)))
            }) && words.any(|word| {
                is_option(
                    word,
                    |group| {
                        group
                            .chars()
                            .take_while(|c| !argument.contains(*c))
                            .any(|c| flags.contains(c))
                    },
                    long,
                )
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

/// No redirection besides those in [`without_null_redirections`], no
/// substitution, and every segment [`reads_only`].
fn is_pure_read(command: &str) -> bool {
    let command = without_null_redirections(command);
    !command.contains(['>', '`', '(']) && segments(&command).all(reads_only)
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
    program.trim_start_matches(['\'', '"']).rsplit('/').next() == Some("cargo")
        && arguments.last().is_none_or(|w| matches!(*w, "-p" | "--package" | "--package="))
}

/// The program of the simple command `segment` and its arguments; for inline
/// code of a shell (`bash -c 'cargo xtask check'`) those of the code.
fn command_words(segment: &str) -> Vec<&str> {
    let words: Vec<&str> = segment.split_whitespace().collect();
    let shell =
        words.first().and_then(|w| w.rsplit('/').next()).is_some_and(|name| SHELLS.contains(&name));
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
