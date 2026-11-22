//! Task inbox (docs/requirements/agent.md §4.1, AG-TK-01..04, ADR-0054): the
//! files `tasks/NNNN-slug.md` of a project, and which state changes a role
//! may make.
//!
//! Pure logic: the review gate gets the part and check states as input
//! ([`GateInput`]), the service fills it from what it built and ran. Paths
//! handed out are relative to the project root.
//!
//! Points the requirement leaves open, decided here:
//!
//! - In `tasks/`, sub folders and files not ending in `.md` are ignored; a
//!   `.md` file whose name is not `NNNN-slug.md` is an error.
//! - Unknown header keys are an error. `created` is kept as written, it is
//!   not checked to be a date.
//! - A new state equal to the current one is no state change. An update with
//!   neither a state change nor a note is an error.
//! - The review gate and "blocked needs a note" bind the agent only; the
//!   human may set every state without conditions.
//! - Log text of a state change: `<old> -> <new>`, followed by `: <note>` if
//!   a note is given. A note alone is the entry as it is. Notes are flattened
//!   to one line.
//! - Numbers end at 9999 (four digits); the next `create` is an error.

mod text;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Folder of the task files below the project root.
const TASKS_DIR: &str = "tasks";

/// Longest slug `create` derives from a title.
const MAX_SLUG_LEN: usize = 40;

/// Why tasks could not be read or changed. Every variant names the file or
/// the task concerned.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("cannot access {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{}: {message}", file.display())]
    File { file: PathBuf, message: String },
    #[error("task {id} is defined in {} and in {}", first.display(), second.display())]
    Duplicate { id: String, first: PathBuf, second: PathBuf },
    #[error("there is no task {id:?}")]
    UnknownTask { id: String },
    #[error(
        "task {id}: the role {} may not change the state from {} to {}",
        role.as_str(),
        from.as_str(),
        to.as_str()
    )]
    Transition { id: String, role: Role, from: TaskState, to: TaskState },
    #[error("task {id}: the state blocked needs a note with the question")]
    NoteRequired { id: String },
    #[error("task {id} cannot go to review: {}", problems.join("; "))]
    Gate { id: String, problems: Vec<String> },
    #[error("task {id}: nothing to change, give a new state or a note")]
    NothingToChange { id: String },
    #[error("a task needs a non-empty title")]
    EmptyTitle,
    #[error("the task numbers are used up (the highest is 9999)")]
    NumbersUsedUp,
    #[error("the date {date:?} is not of the form YYYY-MM-DD")]
    InvalidDate { date: String },
}

/// What a task file is about (AG-TK-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Task,
    Requirement,
    Idea,
}

impl TaskKind {
    /// The name in the header.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Requirement => "requirement",
            Self::Idea => "idea",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [Self::Task, Self::Requirement, Self::Idea].into_iter().find(|k| k.as_str() == name)
    }
}

/// State of a task (AG-TK-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Open,
    InProgress,
    Blocked,
    Review,
    Done,
    Rejected,
}

impl TaskState {
    /// The name in the header.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Review => "review",
            Self::Done => "done",
            Self::Rejected => "rejected",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [Self::Open, Self::InProgress, Self::Blocked, Self::Review, Self::Done, Self::Rejected]
            .into_iter()
            .find(|s| s.as_str() == name)
    }
}

/// Who changes a task. Only the human decides `done` and `rejected`
/// (ADR-0054).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Agent,
    Human,
}

impl Role {
    /// The `who` of a log entry.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
        }
    }

    /// AG-TK-02: the agent has five transitions, the human all.
    fn may_change(self, from: TaskState, to: TaskState) -> bool {
        use TaskState::{Blocked, InProgress, Open, Review};
        match self {
            Self::Human => true,
            Self::Agent => matches!(
                (from, to),
                (Open | Blocked | Review, InProgress) | (InProgress, Blocked | Review)
            ),
        }
    }
}

/// One line of the section `## Log`: `- <date> <who>: <text>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub date: String,
    pub who: String,
    pub text: String,
}

/// A task file (AG-TK-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    /// `T-NNNN`, from the number in the file name.
    pub id: String,
    /// Relative to the project root: `tasks/NNNN-slug.md`.
    pub file: PathBuf,
    pub title: String,
    pub kind: TaskKind,
    pub state: TaskState,
    /// Parts that must be built and current for the review gate.
    pub parts: Vec<String>,
    /// Checks that must have passed for the review gate.
    pub checks: Vec<String>,
    /// As written in the header, not validated.
    pub created: Option<String>,
    /// The Markdown between header and `## Log`, without surrounding blank
    /// lines.
    pub body: String,
    pub log: Vec<LogEntry>,
}

/// What the review gate needs to know about the project (AG-TK-04).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateInput {
    /// Part name → build state; only `"current"` passes.
    pub parts: BTreeMap<String, String>,
    /// Check name → passed in a run made after the last build. Checks that
    /// never ran or are stale are left out.
    pub checks: BTreeMap<String, bool>,
}

/// All tasks of a project, sorted by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tasks {
    tasks: Vec<Task>,
}

impl Tasks {
    /// Reads `<dir>/tasks/*.md`; a project without `tasks/` has no tasks.
    ///
    /// # Errors
    /// Unreadable folder or file, a `.md` file not named `NNNN-slug.md`,
    /// missing or malformed header, missing title, unknown kind or state,
    /// two files with the same number.
    pub fn load(dir: &Path) -> Result<Self, TaskError> {
        let tasks = task_files(dir)?
            .into_iter()
            .map(|(number, file)| text::parse(&file, number, &read_text(&dir.join(&file))?))
            .collect::<Result<_, _>>()?;
        Ok(Self { tasks })
    }

    #[must_use]
    pub fn all(&self) -> &[Task] {
        &self.tasks
    }

    /// The task with the id `T-NNNN`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

/// The unmet conditions of the gate `in_progress -> review` (AG-TK-04);
/// empty means the task may be handed over. Every part of the task must be
/// `current`, every check must have passed; a task with neither needs a
/// non-blank note instead. A note never replaces parts or checks.
#[must_use]
pub fn gate_problems(task: &Task, note: Option<&str>, gate: &GateInput) -> Vec<String> {
    let mut problems = Vec::new();
    for part in &task.parts {
        match gate.parts.get(part).map(String::as_str) {
            Some("current") => {}
            Some(state) => problems.push(format!("part {part:?} is {state:?}, not \"current\"")),
            None => problems.push(format!("part {part:?} does not exist")),
        }
    }
    for check in &task.checks {
        match gate.checks.get(check) {
            Some(true) => {}
            Some(false) => problems.push(format!("check {check:?} failed")),
            None => problems.push(format!("check {check:?} has no current result")),
        }
    }
    if task.parts.is_empty() && task.checks.is_empty() && flat_note(note).is_none() {
        problems
            .push("a task without parts and checks needs a note saying what was done".to_owned());
    }
    problems
}

/// Creates the task file with the number after the highest existing one (a
/// gap is not reused, ids stay stable) and returns the task as it loads back.
///
/// The slug comes from the title: ASCII lowercase, every character outside
/// `[a-z0-9]` becomes `-`, runs of `-` collapse, `-` at both ends is trimmed,
/// at most 40 characters; a title without any slug character gives `task`.
/// `date` becomes `created` unchecked. A line `## Log` inside `body` starts
/// the log when the file is read.
///
/// # Errors
/// Blank title, number above 9999, unreadable or invalid `tasks/` folder,
/// write failure.
pub fn create(
    dir: &Path,
    title: &str,
    kind: TaskKind,
    body: &str,
    parts: &[String],
    checks: &[String],
    date: &str,
) -> Result<Task, TaskError> {
    if title.trim().is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    let number = task_files(dir)?.last().map_or(1, |(highest, _)| highest + 1);
    if number > 9999 {
        return Err(TaskError::NumbersUsedUp);
    }
    let file = Path::new(TASKS_DIR).join(format!("{number:04}-{}.md", slug(title)));
    let content = text::new_file(title, kind, body, parts, checks, date);
    let task = text::parse(&file, number, &content)?;
    let folder = dir.join(TASKS_DIR);
    std::fs::create_dir_all(&folder).map_err(|source| TaskError::Io { path: folder, source })?;
    write_atomic(&dir.join(&file), &content)?;
    Ok(task)
}

/// Changes the state of task `id` and/or appends a note to its log, and
/// returns the task as written. Only the value of the header's `state` line
/// changes and one log line is appended; the rest of the file stays
/// byte-identical (AG-TK-03).
///
/// `new_state` equal to the current state counts as no state change. `gate`
/// is only consulted when the agent moves the task to `review`.
///
/// # Errors
/// Unknown task, transition forbidden for the role, agent sets `blocked`
/// without a note, unmet review gate (all conditions listed), neither a
/// state change nor a note, `date` not shaped `YYYY-MM-DD`, read or write
/// failure. A refused update writes nothing.
pub fn update(
    dir: &Path,
    id: &str,
    role: Role,
    new_state: Option<TaskState>,
    note: Option<&str>,
    date: &str,
    gate: &GateInput,
) -> Result<Task, TaskError> {
    let unknown = || TaskError::UnknownTask { id: id.to_owned() };
    let (number, file) = task_files(dir)?
        .into_iter()
        .find(|(number, _)| format!("T-{number:04}") == id)
        .ok_or_else(unknown)?;
    let path = dir.join(&file);
    let old = read_text(&path)?;
    let task = text::parse(&file, number, &old)?;

    let new_state = new_state.filter(|state| *state != task.state);
    let flat = flat_note(note);
    if let Some(to) = new_state {
        check_transition(&task, role, to, note, gate)?;
    }
    let entry = match (new_state, flat) {
        (Some(to), Some(note)) => format!("{} -> {}: {note}", task.state.as_str(), to.as_str()),
        (Some(to), None) => format!("{} -> {}", task.state.as_str(), to.as_str()),
        (None, Some(note)) => note,
        (None, None) => return Err(TaskError::NothingToChange { id: id.to_owned() }),
    };
    if !text::is_date(date) {
        return Err(TaskError::InvalidDate { date: date.to_owned() });
    }

    let new = text::updated(&old, new_state, date, role, &entry);
    let written = text::parse(&file, number, &new)?;
    // A `state` line this module cannot patch (e.g. a quoted key) must not
    // end in a file that says something else than was asked for.
    if written.state != new_state.unwrap_or(task.state) {
        return Err(TaskError::File {
            file,
            message: "the `state` line of the header cannot be rewritten".to_owned(),
        });
    }
    write_atomic(&path, &new)?;
    Ok(written)
}

/// AG-TK-02 and AG-TK-04 for a real state change of `task` to `to`.
fn check_transition(
    task: &Task,
    role: Role,
    to: TaskState,
    note: Option<&str>,
    gate: &GateInput,
) -> Result<(), TaskError> {
    let id = task.id.clone();
    if !role.may_change(task.state, to) {
        return Err(TaskError::Transition { id, role, from: task.state, to });
    }
    if role == Role::Human {
        return Ok(());
    }
    if to == TaskState::Blocked && flat_note(note).is_none() {
        return Err(TaskError::NoteRequired { id });
    }
    if to == TaskState::Review {
        let problems = gate_problems(task, note, gate);
        if !problems.is_empty() {
            return Err(TaskError::Gate { id, problems });
        }
    }
    Ok(())
}

/// The note on one line (a log entry is one line); `None` if it is blank.
fn flat_note(note: Option<&str>) -> Option<String> {
    let lines: Vec<&str> = note?.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// The slug of a title, see [`create`].
fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().map(|c| c.to_ascii_lowercase()) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    // Only ASCII is left, so the cut cannot split a character.
    slug.truncate(MAX_SLUG_LEN);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "task".to_owned() } else { slug.to_owned() }
}

/// The number of a file name `NNNN-slug.md` (4 digits, slug `[a-z0-9-]+`).
fn task_number(name: &str) -> Option<u16> {
    let (number, slug) = name.strip_suffix(".md")?.split_once('-')?;
    let slug_ok = !slug.is_empty()
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let number_ok = number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit());
    if slug_ok && number_ok { number.parse().ok() } else { None }
}

/// The task files of the project as (number, path relative to `dir`), sorted
/// by number. Sub folders and files not ending in `.md` are skipped.
fn task_files(dir: &Path) -> Result<Vec<(u16, PathBuf)>, TaskError> {
    let folder = dir.join(TASKS_DIR);
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let io = |source| TaskError::Io { path: folder.clone(), source };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&folder).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".md") || entry.file_type().map_err(io)?.is_dir() {
            continue;
        }
        let file = Path::new(TASKS_DIR).join(&*name);
        let Some(number) = task_number(&name) else {
            return Err(TaskError::File {
                file,
                message: "a task file is named NNNN-slug.md (4 digits, slug of a-z, 0-9, -)"
                    .to_owned(),
            });
        };
        files.push((number, file));
    }
    files.sort();
    if let Some([first, second]) = files.array_windows().find(|[a, b]| a.0 == b.0) {
        return Err(TaskError::Duplicate {
            id: format!("T-{:04}", first.0),
            first: first.1.clone(),
            second: second.1.clone(),
        });
    }
    Ok(files)
}

fn read_text(path: &Path) -> Result<String, TaskError> {
    std::fs::read_to_string(path).map_err(|source| TaskError::Io { path: path.to_owned(), source })
}

/// Writes via a temporary file in the same folder and a rename, so a reader
/// (or a crash) never sees half a task. The temporary name does not end in
/// `.md`, a leftover is therefore no task file.
fn write_atomic(path: &Path, content: &str) -> Result<(), TaskError> {
    let temp = path.with_extension("md.tmp");
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| TaskError::Io { path, source }
    };
    std::fs::write(&temp, content).map_err(io(&temp))?;
    std::fs::rename(&temp, path).map_err(io(path))
}
