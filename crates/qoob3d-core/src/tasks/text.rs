//! Text of a task file (AG-TK-01): header between two `+++` lines, body, log.
//!
//! Reading never normalizes and writing only patches: an update replaces the
//! value of the header's `state` line and appends to the end, so everything
//! the human wrote stays byte-identical (AG-TK-03).

use std::ops::Range;
use std::path::Path;

use serde::Deserialize;

use super::{LogEntry, Role, Task, TaskError, TaskKind, TaskState};

/// The heading that ends the body and starts the log.
const LOG_HEADING: &str = "## Log";

/// Byte positions of the sections of a task file.
struct Layout {
    /// The TOML between the two fences.
    header: Range<usize>,
    /// First byte after the closing fence.
    body_start: usize,
    /// The first line `## Log` after the header, if any.
    log_heading: Option<Range<usize>>,
}

/// The header as written. Unknown keys are refused, so a typo like `stat`
/// does not silently leave a task `open`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeader {
    title: Option<String>,
    kind: Option<String>,
    state: Option<String>,
    #[serde(default)]
    parts: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    created: Option<String>,
}

/// Lines of `text` including their line ending, with their byte offset.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    })
}

fn is_fence(line: &str) -> bool {
    line.trim_end() == "+++"
}

/// `None` unless the text starts with a header between two `+++` lines. A
/// later `+++` line belongs to the body.
fn layout(text: &str) -> Option<Layout> {
    let mut lines = lines(text);
    let (_, first) = lines.next().filter(|(_, line)| is_fence(line))?;
    let (header_end, fence) = lines.by_ref().find(|(_, line)| is_fence(line))?;
    let log_heading = lines
        .find(|(_, line)| line.trim_end() == LOG_HEADING)
        .map(|(start, line)| start..start + line.len());
    Some(Layout {
        header: first.len()..header_end,
        body_start: header_end + fence.len(),
        log_heading,
    })
}

/// Parses the text of the task file `file` (relative to the project) with the
/// number `number`.
///
/// The body is the text between the header and the first line `## Log`,
/// without surrounding blank lines. In the log section only lines of the form
/// `- <YYYY-MM-DD> <who>: <text>` are entries; other lines are skipped (they
/// stay in the file).
pub(super) fn parse(file: &Path, number: u16, text: &str) -> Result<Task, TaskError> {
    let invalid = |message: String| TaskError::File { file: file.to_owned(), message };
    let Some(layout) = layout(text) else {
        return Err(invalid("must start with a TOML header between two lines `+++`".to_owned()));
    };
    let raw: RawHeader =
        toml::from_str(&text[layout.header.clone()]).map_err(|e| invalid(e.to_string()))?;
    let title = raw.title.ok_or_else(|| invalid("the header has no `title`".to_owned()))?;
    if title.trim().is_empty() {
        return Err(invalid("`title` must not be empty".to_owned()));
    }
    let kind = match raw.kind {
        None => TaskKind::Task,
        Some(name) => TaskKind::from_name(&name)
            .ok_or_else(|| invalid(format!("unknown kind {name:?} (task, requirement, idea)")))?,
    };
    let state = match raw.state {
        None => TaskState::Open,
        Some(name) => TaskState::from_name(&name).ok_or_else(|| {
            invalid(format!(
                "unknown state {name:?} (open, in_progress, blocked, review, done, rejected)"
            ))
        })?,
    };
    let (body_end, log_start) =
        layout.log_heading.map_or((text.len(), text.len()), |heading| (heading.start, heading.end));
    Ok(Task {
        id: format!("T-{number:04}"),
        file: file.to_owned(),
        title,
        kind,
        state,
        parts: raw.parts,
        checks: raw.checks,
        created: raw.created,
        body: without_blank_edges(&text[layout.body_start..body_end]).to_owned(),
        log: text[log_start..].lines().filter_map(log_entry).collect(),
    })
}

/// `text` without leading and trailing blank lines; the indentation of the
/// first line is kept.
fn without_blank_edges(text: &str) -> &str {
    let text = text.trim_end();
    let blank = text.len() - text.trim_start().len();
    let first_line = text[..blank].rfind('\n').map_or(0, |newline| newline + 1);
    &text[first_line..]
}

/// `YYYY-MM-DD` by shape; the calendar is not checked.
pub(super) fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| if i == 4 || i == 7 { *b == b'-' } else { b.is_ascii_digit() })
}

/// `- <YYYY-MM-DD> <who>: <text>`; the text may itself contain `: `.
fn log_entry(line: &str) -> Option<LogEntry> {
    let (date, rest) = line.trim_end().strip_prefix("- ")?.split_once(' ')?;
    let (who, text) = rest.split_once(": ")?;
    (is_date(date) && !who.is_empty() && !who.contains(char::is_whitespace)).then(|| LogEntry {
        date: date.to_owned(),
        who: who.to_owned(),
        text: text.to_owned(),
    })
}

/// A string as a TOML value, escaped.
fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

fn toml_strings(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|item| toml_string(item)).collect();
    format!("[{}]", items.join(", "))
}

/// The text of a new task file in state `open` without a log.
pub(super) fn new_file(
    title: &str,
    kind: TaskKind,
    body: &str,
    parts: &[String],
    checks: &[String],
    date: &str,
) -> String {
    let mut text = format!(
        "+++\ntitle = {}\nkind = \"{}\"\nstate = \"{}\"\nparts = {}\nchecks = {}\ncreated = {}\n+++\n",
        toml_string(title),
        kind.as_str(),
        TaskState::Open.as_str(),
        toml_strings(parts),
        toml_strings(checks),
        toml_string(date),
    );
    if !body.trim().is_empty() {
        text.push('\n');
        text.push_str(body);
        if !body.ends_with('\n') {
            text.push('\n');
        }
    }
    text
}

/// The value (with its quotes) of a header line `state = "…"`.
fn state_value(line: &str) -> Option<Range<usize>> {
    let rest = line.trim_start().strip_prefix("state")?.trim_start().strip_prefix('=')?;
    let rest = rest.trim_start();
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let close = rest[1..].find(quote)?;
    let start = line.len() - rest.len();
    Some(start..start + close + 2)
}

/// `header` with the value of its `state` line replaced; indentation and a
/// trailing comment of that line stay. A header without the line (state
/// defaulted to `open`) gets one at its end.
fn with_state(header: &str, state: TaskState) -> String {
    let value = format!("\"{}\"", state.as_str());
    let found = lines(header)
        .find_map(|(start, line)| state_value(line).map(|v| start + v.start..start + v.end));
    match found {
        Some(old) => format!("{}{value}{}", &header[..old.start], &header[old.end..]),
        None => format!("{header}state = {value}\n"),
    }
}

/// `text` (a task file that parses) with the state set to `state` if given
/// and one log entry appended; the `## Log` section is created when missing.
/// Everything else is copied byte for byte.
pub(super) fn updated(
    text: &str,
    state: Option<TaskState>,
    date: &str,
    role: Role,
    entry: &str,
) -> String {
    let Some(layout) = layout(text) else {
        return text.to_owned();
    };
    let mut out = String::with_capacity(text.len() + entry.len() + 64);
    out.push_str(&text[..layout.header.start]);
    match state {
        Some(state) => out.push_str(&with_state(&text[layout.header.clone()], state)),
        None => out.push_str(&text[layout.header.clone()]),
    }
    out.push_str(&text[layout.header.end..]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if layout.log_heading.is_none() {
        out.push_str(&format!("\n{LOG_HEADING}\n"));
    }
    out.push_str(&format!("- {date} {}: {entry}\n", role.as_str()));
    out
}
