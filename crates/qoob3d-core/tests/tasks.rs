//! Task inbox (R4, pure layer): docs/requirements/agent.md §4.1 (AG-TK-01..04)
//! and §6.2 (AG-TK-06, AG-TK-07; binding, AG-TK-05, is in `tasks_bind.rs`),
//! ADR-0054. Module `qoob3d_core::tasks` reads and writes `tasks/NNNN-slug.md`
//! below a project folder and decides which state transitions a role may make.
//!
//! All expected values come from the requirement text, none from the
//! implementation. Where the requirement leaves a detail open the tests stay
//! tolerant and say so:
//!
//! - Surrounding blank lines of the body are not specified, so bodies are
//!   compared trimmed.
//! - Whether a run of replaced characters in a slug collapses to one `-` and
//!   whether leading/trailing `-` are trimmed is not specified, so exact slugs
//!   are only asserted for titles where every reading gives the same result.
//! - The log text of a state change is `<old> -> <new>`, followed by
//!   `: <note>` when a note is given (AG-TK-07). Only for the hand-over to
//!   review the entry also lists the bound parts and checks (AG-TK-06);
//!   where in the entry is not specified, so there the tests assert the
//!   start, the names and the note, not the exact text.
//! - `X -> X` is not asserted for either role (it may count as "no state
//!   change").
//!
//! "Byte-identical" (AG-TK-03) is checked on the raw file: after removing the
//! header's `state` line from both versions, the old file must be a byte
//! prefix of the new one, and the rest must be exactly the new log lines.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use qoob3d_core::tasks::{
    GateInput, LogEntry, Role, Task, TaskError, TaskKind, TaskState, Tasks, create, update,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique temporary project folder, removed on drop.
struct Dir {
    root: PathBuf,
}

impl Dir {
    fn new(tag: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("qoob3d-core-tasks-{tag}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).expect("create temp project");
        Self { root }
    }

    /// Writes `text` to `rel` (relative to the project), creating folders.
    fn write(&self, rel: &str, text: &str) -> &Self {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().expect("file has a parent"))
            .expect("create parent folder");
        std::fs::write(&path, text).expect("write project file");
        self
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).expect("read project file")
    }

    /// File names in `tasks/`, sorted; empty when the folder does not exist.
    fn task_files(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.root.join("tasks")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| e.expect("dir entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

const ALL_STATES: [TaskState; 6] = [
    TaskState::Open,
    TaskState::InProgress,
    TaskState::Blocked,
    TaskState::Review,
    TaskState::Done,
    TaskState::Rejected,
];

/// AG-TK-02, role agent: exactly these five transitions.
const AGENT_ALLOWED: [(TaskState, TaskState); 5] = [
    (TaskState::Open, TaskState::InProgress),
    (TaskState::InProgress, TaskState::Blocked),
    (TaskState::Blocked, TaskState::InProgress),
    (TaskState::InProgress, TaskState::Review),
    (TaskState::Review, TaskState::InProgress),
];

/// The `who` of a log entry and the role name in errors (AG-TK-03).
fn who(role: Role) -> &'static str {
    match role {
        Role::Agent => "agent",
        Role::Human => "human",
    }
}

/// Asserts that `message` mentions every word of `words`.
fn assert_mentions(message: &str, words: &[&str]) {
    for w in words {
        assert!(message.contains(w), "error must name {w:?}: {message}");
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn gate(parts: &[(&str, &str)], checks: &[(&str, bool)]) -> GateInput {
    GateInput {
        parts: parts.iter().map(|(n, s)| ((*n).to_owned(), (*s).to_owned())).collect(),
        checks: checks.iter().map(|(n, p)| ((*n).to_owned(), *p)).collect(),
    }
}

fn no_gate() -> GateInput {
    GateInput { parts: BTreeMap::new(), checks: BTreeMap::new() }
}

fn load(d: &Dir) -> Tasks {
    Tasks::load(d.path()).expect("tasks load")
}

fn loaded(d: &Dir, id: &str) -> Task {
    load(d).get(id).unwrap_or_else(|| panic!("{id} is loaded")).clone()
}

/// A task file with only `title` and `state`, no parts, no checks, no log.
fn minimal(state: TaskState) -> String {
    format!("+++\ntitle = \"Probe\"\nstate = \"{}\"\n+++\n\nBody.\n", state.as_str())
}

/// AG-TK-01 file name rule: `NNNN-slug.md`, 4 digits, slug `[a-z0-9-]+`.
fn is_valid_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".md") else {
        return false;
    };
    let b = stem.as_bytes();
    b.len() >= 6
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The slug of a task's file name (after `NNNN-`, before `.md`); also asserts
/// that the name follows AG-TK-01.
fn slug_of(task: &Task) -> String {
    let name = task.file.file_name().expect("file name").to_string_lossy().into_owned();
    assert!(is_valid_name(&name), "{name:?} is no valid task file name");
    name[5..name.len() - 3].to_owned()
}

fn is_state_line(line: &str) -> bool {
    line.trim_start().strip_prefix("state").is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// `text` without the `state` line of its header (lines between the first two
/// `+++`). A line `state = …` in the body is kept.
fn without_state_line(text: &str) -> String {
    let mut out = String::new();
    let mut fences = 0;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare == "+++" {
            fences += 1;
        } else if fences == 1 && is_state_line(bare) {
            continue;
        }
        out.push_str(line);
    }
    out
}

/// AG-TK-03 "byte-identical": apart from the header's `state` line, `new` is
/// `old` plus appended text. Returns the non-empty appended lines.
fn appended_lines(old: &str, new: &str) -> Vec<String> {
    let (o, n) = (without_state_line(old), without_state_line(new));
    assert!(
        n.starts_with(&o),
        "everything but the state line must stay byte-identical\n--- old\n{old}\n--- new\n{new}"
    );
    n[o.len()..].lines().map(str::trim_end).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

fn count_lines(text: &str, line: &str) -> usize {
    text.lines().filter(|l| l.trim_end() == line).count()
}

// ---------------------------------------------------------------------------
// AG-TK-01: file, header, body, log
// ---------------------------------------------------------------------------

const LID_FILE: &str = "tasks/0007-lid-with-pin.md";

/// A hand-written task with every header field, comments, uneven spacing, a
/// decoy `state` line in the body, an own `##` section and a log.
const LID: &str = "\
+++
# written by hand, the comment must survive updates
title = \"Lid with pin\"   # trailing comment
kind = \"requirement\"
state = \"open\"
checks = [ \"fit\",\"wall\" ]
parts = [\"Lid\", 'Box']
created = \"2026-11-20\"
+++

The lid closes with **0.2 mm** clearance.

state = \"open\" is also a line of the body.

## Notes

- keep the pin short

## Log
- 2026-11-20 human: created by hand
- 2026-11-21 agent: started: first draft
";

const LID_BODY: &str = "\
The lid closes with **0.2 mm** clearance.

state = \"open\" is also a line of the body.

## Notes

- keep the pin short";

#[test]
fn ag_tk_01_full_task_file_is_parsed() {
    let d = Dir::new("full");
    d.write(LID_FILE, LID);
    let tasks = load(&d);
    assert_eq!(tasks.all().len(), 1);
    let t = &tasks.all()[0];
    // "Its id is T-NNNN": file 0007-… is T-0007.
    assert_eq!(t.id, "T-0007");
    assert_eq!(slash(&t.file), LID_FILE, "file is relative to the project");
    assert_eq!(t.title, "Lid with pin");
    assert_eq!(t.kind, TaskKind::Requirement);
    assert_eq!(t.state, TaskState::Open);
    assert_eq!(t.parts, ["Lid", "Box"]);
    assert_eq!(t.checks, ["fit", "wall"]);
    assert_eq!(t.created.as_deref(), Some("2026-11-20"));
    // The body is everything between header and `## Log`, including other
    // `##` sections and list lines that look like log entries.
    assert_eq!(t.body.trim(), LID_BODY);
    // `- <date> <who>: <text>`: the text may itself contain ": ".
    assert_eq!(
        t.log,
        [
            LogEntry {
                date: "2026-11-20".to_owned(),
                who: "human".to_owned(),
                text: "created by hand".to_owned()
            },
            LogEntry {
                date: "2026-11-21".to_owned(),
                who: "agent".to_owned(),
                text: "started: first draft".to_owned()
            },
        ]
    );
}

#[test]
fn ag_tk_01_header_defaults() {
    // Only `title` is required: kind defaults to task, state to open, parts
    // and checks to empty, `created` and the log section are optional.
    let d = Dir::new("defaults");
    d.write("tasks/0001-box.md", "+++\ntitle = \"Box\"\n+++\nA box, 40 mm wide.\n");
    let t = loaded(&d, "T-0001");
    assert_eq!(t.title, "Box");
    assert_eq!(t.kind, TaskKind::Task);
    assert_eq!(t.state, TaskState::Open);
    assert!(t.parts.is_empty(), "{:?}", t.parts);
    assert!(t.checks.is_empty(), "{:?}", t.checks);
    assert_eq!(t.created, None);
    assert_eq!(t.body.trim(), "A box, 40 mm wide.");
    assert!(t.log.is_empty(), "{:?}", t.log);
}

#[test]
fn ag_tk_01_every_kind_and_state_is_read() {
    let kinds = [
        ("task", TaskKind::Task),
        ("requirement", TaskKind::Requirement),
        ("idea", TaskKind::Idea),
    ];
    for (name, kind) in kinds {
        let d = Dir::new("kinds");
        d.write("tasks/0001-k.md", &format!("+++\ntitle = \"K\"\nkind = \"{name}\"\n+++\n"));
        assert_eq!(loaded(&d, "T-0001").kind, kind, "kind {name}");
    }
    // AG-TK-02 lists the six state names; `as_str` gives exactly them.
    let names = ["open", "in_progress", "blocked", "review", "done", "rejected"];
    for (name, state) in names.into_iter().zip(ALL_STATES) {
        assert_eq!(state.as_str(), name);
        let d = Dir::new("states");
        d.write("tasks/0001-s.md", &format!("+++\ntitle = \"S\"\nstate = \"{name}\"\n+++\n"));
        assert_eq!(loaded(&d, "T-0001").state, state, "state {name}");
    }
}

#[test]
fn ag_tk_01_header_only_file_has_empty_body_and_log() {
    let d = Dir::new("header-only");
    d.write("tasks/0001-bare.md", "+++\ntitle = \"Bare\"\n+++\n");
    let t = loaded(&d, "T-0001");
    assert_eq!(t.body.trim(), "");
    assert!(t.log.is_empty());
}

#[test]
fn ag_tk_01_log_without_body_and_empty_log_section() {
    let d = Dir::new("log-only");
    d.write(
        "tasks/0001-log.md",
        "+++\ntitle = \"L\"\n+++\n## Log\n- 2026-11-22 human: only a log\n",
    )
    .write("tasks/0002-empty-log.md", "+++\ntitle = \"E\"\n+++\nText.\n\n## Log\n");
    let tasks = load(&d);
    let with_log = tasks.get("T-0001").expect("T-0001");
    assert_eq!(with_log.body.trim(), "");
    assert_eq!(with_log.log.len(), 1);
    assert_eq!(with_log.log[0].text, "only a log");
    let empty = tasks.get("T-0002").expect("T-0002");
    assert_eq!(empty.body.trim(), "Text.");
    assert!(empty.log.is_empty());
}

#[test]
fn ag_tk_01_only_a_log_heading_starts_the_log() {
    // The section is `## Log`; `## Logistics` and `### Log` are the human's
    // Markdown and belong to the body.
    let d = Dir::new("log-heading");
    let body = "## Logistics\n\n- 2026-11-01 human: ship by mail\n\n### Log\n\nnot the log";
    d.write(
        "tasks/0001-ship.md",
        &format!("+++\ntitle = \"Ship\"\n+++\n{body}\n\n## Log\n- 2026-11-22 agent: seen\n"),
    );
    let t = loaded(&d, "T-0001");
    assert_eq!(t.body.trim(), body);
    assert_eq!(t.log.len(), 1, "{:?}", t.log);
    assert_eq!(t.log[0].who, "agent");
    assert_eq!(t.log[0].text, "seen");
}

#[test]
fn ag_tk_01_header_ends_at_the_second_fence() {
    // The header is between the first two `+++` lines; a later `+++` is body.
    let d = Dir::new("fence-in-body");
    d.write("tasks/0001-fence.md", "+++\ntitle = \"Fence\"\n+++\nabove\n+++\nbelow\n");
    let t = loaded(&d, "T-0001");
    assert_eq!(t.title, "Fence");
    assert_eq!(t.body.trim(), "above\n+++\nbelow");
}

#[test]
fn ag_tk_01_non_ascii_text_is_kept() {
    // Title and body are the human's words; "Gehaeuse" with a-umlaut and a
    // diameter sign must survive. (Escapes keep this file ASCII.)
    let d = Dir::new("non-ascii");
    let title = "Geh\u{e4}use \u{2300} 40";
    let body = "Wandst\u{e4}rke 1,2 mm.";
    d.write("tasks/0001-gehaeuse.md", &format!("+++\ntitle = \"{title}\"\n+++\n{body}\n"));
    let t = loaded(&d, "T-0001");
    assert_eq!(t.title, title);
    assert_eq!(t.body.trim(), body);
}

// ---------------------------------------------------------------------------
// AG-TK-03: Tasks::load
// ---------------------------------------------------------------------------

#[test]
fn ag_tk_03_load_sorts_by_id_and_get_finds_by_id() {
    let d = Dir::new("sorted");
    d.write("tasks/0010-a.md", "+++\ntitle = \"Ten\"\n+++\n")
        .write("tasks/0002-z.md", "+++\ntitle = \"Two\"\n+++\n")
        .write("tasks/0001-m.md", "+++\ntitle = \"One\"\n+++\n")
        .write("tasks/0300-b.md", "+++\ntitle = \"Three hundred\"\n+++\n");
    let tasks = load(&d);
    let ids: Vec<&str> = tasks.all().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["T-0001", "T-0002", "T-0010", "T-0300"]);
    assert_eq!(tasks.get("T-0010").map(|t| t.title.as_str()), Some("Ten"));
    assert_eq!(tasks.get("T-0002").map(|t| slash(&t.file)).as_deref(), Some("tasks/0002-z.md"));
    assert!(tasks.get("T-0003").is_none(), "a gap is no task");
    assert!(tasks.get("").is_none());
}

#[test]
fn ag_tk_03_empty_or_missing_tasks_folder_has_no_tasks() {
    let d = Dir::new("empty");
    // Interpretation: a project that never had a task has no `tasks/` yet
    // (same rule as `values/` in AG-PJ-03).
    assert!(load(&d).all().is_empty(), "missing tasks/ folder");
    std::fs::create_dir_all(d.path().join("tasks")).expect("create tasks/");
    let tasks = load(&d);
    assert!(tasks.all().is_empty(), "empty tasks/ folder");
    assert!(tasks.get("T-0001").is_none());
}

/// Loads a project with one valid task and `name` holding `content`, expects
/// an error and returns its text after asserting that it names the file.
fn load_error(name: &str, content: &str) -> String {
    let d = Dir::new("bad");
    d.write("tasks/0001-good.md", "+++\ntitle = \"Good\"\n+++\n")
        .write(&format!("tasks/{name}"), content);
    let e = Tasks::load(d.path())
        .expect_err(&format!("{name:?} with {content:?} must not load"))
        .to_string();
    assert_mentions(&e, &[name]);
    e
}

#[test]
fn ag_tk_03_bad_file_names_are_errors_naming_the_file() {
    // AG-TK-01: 4 digits, `-`, lowercase slug `[a-z0-9-]+`.
    let valid = "+++\ntitle = \"T\"\n+++\n";
    for name in [
        "2-short-number.md",    // 1 digit
        "002-three-digits.md",  // 3 digits
        "00002-five-digits.md", // 5 digits
        "abcd-letters.md",      // no number
        "0002.md",              // no slug
        "0002-.md",             // empty slug
        "0002_underscore.md",   // `_` instead of `-`
        "0002-Upper.md",        // uppercase
        "0002-two words.md",    // space
        "0002-under_score.md",  // `_` is not in the slug alphabet
        "0002-geh\u{e4}use.md", // non-ASCII
        "notes.md",             // no task name at all
    ] {
        assert!(!is_valid_name(name), "test data: {name}");
        load_error(name, valid);
    }
}

#[test]
fn ag_tk_03_missing_header_is_an_error_naming_the_file() {
    for content in [
        "",                                           // empty file
        "Just some words.\n",                         // no header at all
        "+++\ntitle = \"Never closed\"\n\nBody.\n",   // only one `+++`
        "Intro first.\n+++\ntitle = \"Late\"\n+++\n", // the file must *start* with it
        "title = \"No fences\"\n",                    // TOML without `+++`
    ] {
        load_error("0002-header.md", content);
    }
}

#[test]
fn ag_tk_03_malformed_header_is_an_error_naming_the_file() {
    load_error("0002-toml.md", "+++\ntitle = \n+++\n");
    load_error("0002-toml.md", "+++\ntitle \"x\"\n+++\n");
    // Wrong types are no valid header either.
    load_error("0002-toml.md", "+++\ntitle = 3\n+++\n");
    load_error("0002-toml.md", "+++\ntitle = \"x\"\nparts = \"Lid\"\n+++\n");
    load_error("0002-toml.md", "+++\ntitle = \"x\"\nchecks = [1, 2]\n+++\n");
}

#[test]
fn ag_tk_03_missing_or_empty_title_is_an_error_naming_the_file() {
    // "`title` (required, non-empty)".
    let e = load_error("0002-untitled.md", "+++\nkind = \"idea\"\n+++\nBody.\n");
    assert_mentions(&e, &["title"]);
    let e = load_error("0002-untitled.md", "+++\n+++\nBody.\n");
    assert_mentions(&e, &["title"]);
    let e = load_error("0002-untitled.md", "+++\ntitle = \"\"\n+++\n");
    assert_mentions(&e, &["title"]);
}

#[test]
fn ag_tk_03_unknown_kind_is_an_error_naming_file_and_kind() {
    let e = load_error("0002-kind.md", "+++\ntitle = \"x\"\nkind = \"bug\"\n+++\n");
    assert_mentions(&e, &["bug"]);
    // Names are lowercase; `Task` is not `task`.
    load_error("0002-kind.md", "+++\ntitle = \"x\"\nkind = \"Task\"\n+++\n");
}

#[test]
fn ag_tk_03_unknown_state_is_an_error_naming_file_and_state() {
    let e = load_error("0002-state.md", "+++\ntitle = \"x\"\nstate = \"closed\"\n+++\n");
    assert_mentions(&e, &["closed"]);
    // The names of AG-TK-02 are snake_case; other spellings are unknown.
    load_error("0002-state.md", "+++\ntitle = \"x\"\nstate = \"in-progress\"\n+++\n");
    load_error("0002-state.md", "+++\ntitle = \"x\"\nstate = \"Open\"\n+++\n");
}

#[test]
fn ag_tk_03_duplicate_number_is_an_error_naming_the_files() {
    // Two files with number 0003 would both be T-0003. The duplicate consists
    // of two files, so both are named (as for duplicate values, AG-PJ-03).
    let d = Dir::new("duplicate");
    d.write("tasks/0001-first.md", "+++\ntitle = \"1\"\n+++\n")
        .write("tasks/0003-lid.md", "+++\ntitle = \"Lid\"\n+++\n")
        .write("tasks/0003-box.md", "+++\ntitle = \"Box\"\n+++\n");
    let e = Tasks::load(d.path()).expect_err("duplicate number").to_string();
    assert_mentions(&e, &["0003-lid.md", "0003-box.md"]);
}

// ---------------------------------------------------------------------------
// AG-TK-03: create
// ---------------------------------------------------------------------------

const DATE: &str = "2026-11-22";

fn new_task(d: &Dir, title: &str) -> Task {
    create(d.path(), title, TaskKind::Task, "", &[], &[], DATE)
        .unwrap_or_else(|e| panic!("create {title:?}: {e}"))
}

#[test]
fn ag_tk_03_create_writes_the_first_task_and_returns_it() {
    let d = Dir::new("create");
    let body = "Lid for the box.\n\nClearance **0.2 mm**.";
    let t = create(
        d.path(),
        "Box with lid",
        TaskKind::Requirement,
        body,
        &strings(&["Box", "Lid"]),
        &strings(&["fit"]),
        DATE,
    )
    .expect("create in a project without tasks/");
    // No task yet: the next free number is 1. Slug: lowercase, space -> `-`.
    assert_eq!(t.id, "T-0001");
    assert_eq!(slash(&t.file), "tasks/0001-box-with-lid.md");
    assert_eq!(t.title, "Box with lid");
    assert_eq!(t.kind, TaskKind::Requirement);
    assert_eq!(t.state, TaskState::Open, "a new task is open (default state)");
    assert_eq!(t.parts, ["Box", "Lid"]);
    assert_eq!(t.checks, ["fit"]);
    assert_eq!(t.created.as_deref(), Some(DATE));
    assert_eq!(t.body.trim(), body);
    assert!(t.log.is_empty(), "{:?}", t.log);

    assert_eq!(d.task_files(), ["0001-box-with-lid.md"]);
    let text = d.read("tasks/0001-box-with-lid.md");
    assert!(text.starts_with("+++\n"), "the file starts with the header: {text}");
    assert!(text.contains(body), "the body is written verbatim: {text}");
    // The written file loads back as exactly the returned task.
    assert_eq!(loaded(&d, "T-0001"), t);
}

#[test]
fn ag_tk_03_create_takes_the_number_after_the_highest() {
    // 0001 and 0003 exist: the gap 0002 is not reused (ids are stable, a
    // deleted task's id must not come back), the next free number is 0004.
    let d = Dir::new("create-gap");
    d.write("tasks/0001-a.md", "+++\ntitle = \"A\"\n+++\n")
        .write("tasks/0003-c.md", "+++\ntitle = \"C\"\n+++\n");
    assert_eq!(new_task(&d, "Next").id, "T-0004");
    assert_eq!(slash(&new_task(&d, "After").file), "tasks/0005-after.md");

    let d = Dir::new("create-high");
    d.write("tasks/0041-only.md", "+++\ntitle = \"Only\"\n+++\n");
    assert_eq!(new_task(&d, "Next").id, "T-0042");
    let ids: Vec<String> = load(&d).all().iter().map(|t| t.id.clone()).collect();
    assert_eq!(ids, ["T-0041", "T-0042"]);
}

#[test]
fn ag_tk_03_create_same_title_twice_gives_two_tasks() {
    let d = Dir::new("create-twice");
    let (a, b) = (new_task(&d, "Hook"), new_task(&d, "Hook"));
    assert_eq!((a.id.as_str(), b.id.as_str()), ("T-0001", "T-0002"));
    assert_eq!(d.task_files(), ["0001-hook.md", "0002-hook.md"]);
    assert_eq!(load(&d).all(), [a, b]);
}

#[test]
fn ag_tk_03_slug_is_ascii_lowercase_with_dashes() {
    // "ASCII lowercase, other characters -> `-`". Every title here has single
    // separators inside the text, so collapsing/trimming rules do not matter.
    for (title, slug) in [
        ("Box With LID", "box-with-lid"),
        ("hook", "hook"),
        ("lid/box", "lid-box"),
        ("snap_fit", "snap-fit"), // `_` is not in `[a-z0-9-]`
        ("already-dashed", "already-dashed"),
        ("Geh\u{e4}use", "geh-use"), // a-umlaut is one character -> one `-`
        ("Stra\u{df}e", "stra-e"),   // sharp s
        ("\u{d6}l", "-l"),           // uppercase O-umlaut is not ASCII: no `ol`, no `oel`
    ] {
        let d = Dir::new("slug");
        let t = new_task(&d, title);
        // A leading `-` may be trimmed (not specified), the letters may not change.
        assert_eq!(slug_of(&t).trim_matches('-'), slug.trim_matches('-'), "title {title:?}");
        assert_eq!(loaded(&d, "T-0001").title, title, "the title itself is kept");
    }
}

#[test]
fn ag_tk_03_slug_keeps_digits() {
    // The slug alphabet of AG-TK-01 is `[a-z0-9-]`: digits are slug characters.
    let d = Dir::new("slug-digits");
    assert_eq!(slug_of(&new_task(&d, "M3 holder v2")), "m3-holder-v2");
}

#[test]
fn ag_tk_03_slug_has_at_most_40_characters() {
    // 40 -> 40, 41 -> 40, 50 -> 40: a run of `a` has no separator, so the
    // slug is exactly the first 40 characters.
    for len in [39, 40, 41, 50, 200] {
        let d = Dir::new("slug-len");
        let t = new_task(&d, &"a".repeat(len));
        assert_eq!(slug_of(&t), "a".repeat(len.min(40)), "title of {len} characters");
        assert_eq!(loaded(&d, "T-0001"), t);
    }
    // 6 words of 9 letters = 59 characters; the first 40 are 4 words and 4
    // dashes. Whether the cut-off dash stays is not specified.
    let d = Dir::new("slug-len-words");
    let slug = slug_of(&new_task(&d, &["abcdefghi"; 6].join(" ")));
    assert!(slug.len() <= 40, "{slug:?} has {} characters", slug.len());
    assert_eq!(slug.trim_end_matches('-'), ["abcdefghi"; 4].join("-"));
}

#[test]
fn ag_tk_03_slug_of_awkward_titles_is_still_a_valid_file_name() {
    // Whatever the title, the file must follow AG-TK-01 (`slug_of` asserts
    // it), stay within 40 characters and load back. Multi-byte characters at
    // the 40-character cut must not split a character.
    for title in [
        "\u{e4}\u{f6}\u{fc}".to_owned(),        // nothing ASCII at all
        "!!!".to_owned(),                       // only punctuation
        "(padded)".to_owned(),                  // leading/trailing separators
        "a  --  b".to_owned(),                  // runs of separators
        "\u{e4}".repeat(60),                    // 120 bytes, 60 characters
        format!("{}bc", "\u{2300}".repeat(39)), // 3-byte characters up to the cut
        "\u{1f600} smile".to_owned(),           // 4-byte character
    ] {
        let d = Dir::new("slug-awkward");
        let t = new_task(&d, &title);
        let slug = slug_of(&t);
        assert!(slug.len() <= 40, "title {title:?}: slug {slug:?}");
        assert_eq!(t.title, title);
        assert_eq!(loaded(&d, "T-0001"), t, "title {title:?}");
    }
}

#[test]
fn ag_tk_03_create_round_trips_text_that_needs_escaping() {
    // The header is TOML: quotes, backslashes and `#` in title, parts and
    // checks must be escaped on write so the file loads back identically.
    let d = Dir::new("create-escape");
    let title = "Lid \"v2\" \\ C:\\tmp # not a comment";
    let parts = strings(&["Lid \"A\"", "Back\\slash"]);
    let checks = strings(&["fit #1", "it's tight"]);
    let body = "Line one.\n+++\nstate = \"done\"\ntitle = \"decoy\"\n\nGeh\u{e4}use.";
    let t = create(d.path(), title, TaskKind::Idea, body, &parts, &checks, DATE).expect("create");
    assert_eq!(t.title, title);
    assert_eq!(t.kind, TaskKind::Idea);
    assert_eq!(t.parts, parts);
    assert_eq!(t.checks, checks);
    assert_eq!(t.body.trim(), body);
    assert_eq!(t.state, TaskState::Open, "the decoy state line in the body is body");
    assert_eq!(loaded(&d, "T-0001"), t);
}

#[test]
fn ag_tk_03_create_round_trips_every_kind_and_an_empty_body() {
    for kind in [TaskKind::Task, TaskKind::Requirement, TaskKind::Idea] {
        let d = Dir::new("create-kind");
        let t = create(d.path(), "Thing", kind, "", &[], &[], "2026-01-31").expect("create");
        assert_eq!(t.kind, kind);
        assert_eq!(t.body.trim(), "");
        assert!(t.parts.is_empty() && t.checks.is_empty());
        assert_eq!(t.created.as_deref(), Some("2026-01-31"));
        assert_eq!(loaded(&d, "T-0001"), t);
    }
}

#[test]
fn ag_tk_03_create_refuses_an_empty_title_and_writes_nothing() {
    // AG-TK-01: `title` is required and non-empty; create must not write a
    // file that load would refuse.
    let d = Dir::new("create-empty-title");
    create(d.path(), "", TaskKind::Task, "body", &[], &[], DATE).expect_err("empty title");
    assert!(d.task_files().is_empty(), "{:?}", d.task_files());
}

// ---------------------------------------------------------------------------
// AG-TK-03: update
// ---------------------------------------------------------------------------

const TODAY: &str = "2026-11-23";

#[test]
fn ag_tk_03_update_rewrites_only_the_state_line_and_appends_the_log_entry() {
    let d = Dir::new("update");
    d.write(LID_FILE, LID);
    let before = loaded(&d, "T-0007");
    let t = update(
        d.path(),
        "T-0007",
        Role::Agent,
        Some(TaskState::InProgress),
        Some("taking it"),
        TODAY,
        &no_gate(),
    )
    .expect("agent: open -> in_progress");

    // Raw bytes: comments, spacing, field order, body (with its decoy
    // `state = "open"` line) and the old log are untouched; one line is new.
    let new = d.read(LID_FILE);
    let lines = appended_lines(LID, &new);
    assert_eq!(lines.len(), 1, "exactly one log line is appended: {lines:?}");
    // AG-TK-07: `<old> -> <new>: <note>`.
    assert_eq!(lines[0], "- 2026-11-23 agent: open -> in_progress: taking it");
    assert_eq!(count_lines(&new, "## Log"), 1, "the existing log section is reused");
    assert!(new.contains("state = \"open\" is also a line of the body."), "body line changed");
    assert!(new.contains("in_progress"), "{new}");

    // Parsed: only state and log differ.
    assert_eq!(t.state, TaskState::InProgress);
    assert_eq!(
        (&t.id, &t.file, &t.title, t.kind, &t.parts, &t.checks, &t.created, &t.body),
        (
            &before.id,
            &before.file,
            &before.title,
            before.kind,
            &before.parts,
            &before.checks,
            &before.created,
            &before.body
        )
    );
    assert_eq!(t.log.len(), 3);
    assert_eq!(t.log[..2], before.log[..]);
    assert_eq!((t.log[2].date.as_str(), t.log[2].who.as_str()), (TODAY, "agent"));
    assert_eq!(t.log[2].text, "open -> in_progress: taking it");
    assert_eq!(loaded(&d, "T-0007"), t, "update returns the task as written");
}

#[test]
fn ag_tk_03_update_without_note_still_appends_a_log_entry() {
    // "rewrites only the state line and appends the log entry": the change
    // itself is logged with date and role, as `<old> -> <new>` (AG-TK-07; a
    // hand-over to review may also list parts and checks, AG-TK-06).
    let d = Dir::new("update-no-note");
    d.write(LID_FILE, LID);
    let t =
        update(d.path(), "T-0007", Role::Human, Some(TaskState::Review), None, TODAY, &no_gate())
            .expect("human: open -> review");
    assert_eq!(t.state, TaskState::Review);
    assert_eq!(t.log.len(), 3);
    assert_eq!((t.log[2].date.as_str(), t.log[2].who.as_str()), (TODAY, "human"));
    let lines = appended_lines(LID, &d.read(LID_FILE));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("- 2026-11-23 human: open -> review"), "{:?}", lines[0]);
}

#[test]
fn ag_tk_03_update_creates_the_log_section_when_missing() {
    let d = Dir::new("update-new-log");
    let old =
        "+++\ntitle = \"Hook\"\nstate = \"open\"\nparts = [\"Hook\"]\n+++\n\nA hook for 5 kg.\n";
    d.write("tasks/0002-hook.md", old);
    let before = loaded(&d, "T-0002");
    let t = update(
        d.path(),
        "T-0002",
        Role::Human,
        Some(TaskState::Rejected),
        Some("not needed"),
        TODAY,
        &no_gate(),
    )
    .expect("human: open -> rejected");
    let new = d.read("tasks/0002-hook.md");
    let lines = appended_lines(old, &new);
    assert_eq!(lines.len(), 2, "heading and entry: {lines:?}");
    assert_eq!(lines[0], "## Log");
    assert_eq!(lines[1], "- 2026-11-23 human: open -> rejected: not needed");
    assert_eq!(t.state, TaskState::Rejected);
    assert_eq!(t.body, before.body, "the new section is not part of the body");
    assert_eq!(t.log.len(), 1);
    assert_eq!(loaded(&d, "T-0002"), t);

    // A second update reuses the section.
    let t = update(d.path(), "T-0002", Role::Human, None, Some("really"), TODAY, &no_gate())
        .expect("note");
    assert_eq!(count_lines(&d.read("tasks/0002-hook.md"), "## Log"), 1);
    assert_eq!(t.log.len(), 2);
    assert_eq!(t.body, before.body);
}

#[test]
fn ag_tk_03_update_of_a_file_without_final_newline_keeps_entries_apart() {
    // Hand-edited files may end without `\n`; the new entry must not be glued
    // to the last log line or to the last body line.
    let d = Dir::new("update-no-newline");
    d.write(LID_FILE, LID.trim_end());
    let t = update(d.path(), "T-0007", Role::Agent, None, Some("a question"), TODAY, &no_gate())
        .expect("note");
    assert_eq!(t.log.len(), 3, "{:?}", t.log);
    assert_eq!(t.log[1].text, "started: first draft");
    assert_eq!(t.log[2].text, "a question");

    let old = "+++\ntitle = \"Hook\"\n+++\nlast body line";
    d.write("tasks/0002-hook.md", old);
    let t = update(d.path(), "T-0002", Role::Agent, None, Some("seen"), TODAY, &no_gate())
        .expect("note");
    assert_eq!(t.body.trim(), "last body line");
    assert_eq!(t.log.len(), 1);
    assert_eq!(appended_lines(old, &d.read("tasks/0002-hook.md")).len(), 2);
}

#[test]
fn ag_tk_03_update_of_a_header_without_state_line_sets_the_state() {
    // `state` defaulted to open; the change has to be stored although there
    // is no line to rewrite. Everything else stays byte-identical.
    let d = Dir::new("update-default-state");
    let old = "+++\n# no state yet\ntitle = \"Box\"\nparts = []\n+++\nA box.\n";
    d.write("tasks/0001-box.md", old);
    let t = update(
        d.path(),
        "T-0001",
        Role::Agent,
        Some(TaskState::InProgress),
        None,
        TODAY,
        &no_gate(),
    )
    .expect("agent: open -> in_progress");
    assert_eq!(t.state, TaskState::InProgress);
    let new = d.read("tasks/0001-box.md");
    assert_eq!(appended_lines(old, &new).len(), 2, "heading and entry");
    let reloaded = loaded(&d, "T-0001");
    assert_eq!(reloaded.state, TaskState::InProgress);
    assert_eq!(reloaded.title, "Box");
    assert_eq!(reloaded.body.trim(), "A box.");
}

#[test]
fn ag_tk_03_note_without_state_change_only_appends_to_the_log() {
    for role in [Role::Agent, Role::Human] {
        let d = Dir::new("note-only");
        d.write(LID_FILE, LID);
        let note = "Which screw: M3 or M4?";
        let t = update(d.path(), "T-0007", role, None, Some(note), TODAY, &no_gate())
            .expect("a note needs no transition");
        // Nothing is rewritten, not even the state line: the old file is a
        // byte prefix of the new one, followed by exactly the entry line.
        let new = d.read(LID_FILE);
        assert!(new.starts_with(LID), "only an append is allowed:\n{new}");
        let tail: Vec<&str> = new[LID.len()..].lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(tail, [format!("- 2026-11-23 {}: {note}", who(role))]);
        assert_eq!(t.state, TaskState::Open);
        assert_eq!(
            t.log.last(),
            Some(&LogEntry {
                date: TODAY.to_owned(),
                who: who(role).to_owned(),
                text: note.to_owned()
            })
        );
        assert_eq!(t.log.len(), 3);
        assert_eq!(loaded(&d, "T-0007"), t);
    }
}

#[test]
fn ag_tk_03_note_only_needs_no_gate_and_no_transition_right() {
    // The agent may comment on a task in review without moving it; the gate
    // (unmet here: nothing is built) only guards `in_progress -> review`.
    let d = Dir::new("note-review");
    d.write(
        "tasks/0001-lid.md",
        "+++\ntitle = \"Lid\"\nstate = \"review\"\nparts = [\"Lid\"]\nchecks = [\"fit\"]\n+++\n",
    );
    let t =
        update(d.path(), "T-0001", Role::Agent, None, Some("render attached"), TODAY, &no_gate())
            .expect("note on a task in review");
    assert_eq!(t.state, TaskState::Review);
    assert_eq!(t.log.len(), 1);
    assert_eq!(t.log[0].text, "render attached");
}

#[test]
fn ag_tk_03_update_touches_only_the_addressed_task() {
    let d = Dir::new("update-other");
    let other = "+++\ntitle = \"Other\"\nstate = \"open\"\n+++\nUntouched.\n";
    d.write(LID_FILE, LID).write("tasks/0008-other.md", other);
    update(d.path(), "T-0007", Role::Human, Some(TaskState::Done), None, TODAY, &no_gate())
        .expect("human: open -> done");
    assert_eq!(d.read("tasks/0008-other.md"), other);
    assert_eq!(d.task_files(), ["0007-lid-with-pin.md", "0008-other.md"], "no file renamed/added");
    assert_eq!(loaded(&d, "T-0007").state, TaskState::Done);
}

#[test]
fn ag_tk_03_update_of_an_unknown_task_is_an_error_naming_it() {
    let d = Dir::new("update-unknown");
    d.write(LID_FILE, LID);
    for id in ["T-0099", "lid-with-pin", ""] {
        let r =
            update(d.path(), id, Role::Human, Some(TaskState::Done), Some("n"), TODAY, &no_gate());
        let e = r.expect_err(&format!("{id:?} is no task")).to_string();
        assert_mentions(&e, &[id]);
    }
    assert_eq!(d.read(LID_FILE), LID, "a failed update writes nothing");
}

// ---------------------------------------------------------------------------
// AG-TK-02: transitions
// ---------------------------------------------------------------------------

const PROBE_FILE: &str = "tasks/0001-probe.md";

/// A task with one part and one check bound, as the gate of AG-TK-06 needs.
fn bound(state: TaskState) -> String {
    format!(
        "+++\ntitle = \"Probe\"\nstate = \"{}\"\nparts = [\"Lid\"]\nchecks = [\"fit\"]\n+++\n\nBody.\n",
        state.as_str()
    )
}

/// The gate input that lets [`bound`] pass.
fn green() -> GateInput {
    gate(&[("Lid", "current")], &[("fit", true)])
}

/// Tries `from -> to` as `role` on a task with a part and a check that are
/// fine, with a note (so neither the `blocked` rule nor the gate of
/// AG-TK-04/06 interferes). Returns a description if the outcome contradicts
/// AG-TK-02.
fn transition_mismatch(role: Role, from: TaskState, to: TaskState) -> Option<String> {
    let allowed = match role {
        Role::Human => true,
        Role::Agent => AGENT_ALLOWED.contains(&(from, to)),
    };
    let label = format!("{} {} -> {}", who(role), from.as_str(), to.as_str());
    let d = Dir::new("matrix");
    let before = bound(from);
    d.write(PROBE_FILE, &before);
    let result = update(d.path(), "T-0001", role, Some(to), Some("note"), TODAY, &green());
    match (allowed, result) {
        (true, Err(e)) => Some(format!("{label}: must be allowed, got: {e}")),
        (false, Ok(_)) => Some(format!("{label}: must be forbidden")),
        (true, Ok(task)) => {
            let stored = loaded(&d, "T-0001").state;
            (task.state != to || stored != to)
                .then(|| format!("{label}: returned {:?}, stored {stored:?}", task.state))
        }
        (false, Err(e)) => {
            let e = e.to_string();
            // "an error naming task, states and role".
            let named = ["T-0001", from.as_str(), to.as_str(), who(role)];
            if let Some(missing) = named.iter().find(|w| !e.contains(**w)) {
                return Some(format!("{label}: error must name {missing:?}: {e}"));
            }
            (d.read(PROBE_FILE) != before).then(|| format!("{label}: refused but file changed"))
        }
    }
}

#[test]
fn ag_tk_02_transition_matrix_for_both_roles() {
    // Every ordered pair of different states for both roles: 2 * 6 * 5 = 60
    // cases. Agent: exactly the 5 transitions of AG-TK-02, the other 25 are
    // errors. Human: all 30.
    let mut cases = 0;
    let mut wrong = Vec::new();
    for role in [Role::Agent, Role::Human] {
        for from in ALL_STATES {
            for to in ALL_STATES.into_iter().filter(|to| *to != from) {
                cases += 1;
                wrong.extend(transition_mismatch(role, from, to));
            }
        }
    }
    assert_eq!(cases, 60);
    assert!(wrong.is_empty(), "{} of 60 transitions wrong:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn ag_tk_02_agent_transitions_other_than_blocked_need_no_note() {
    // Only `in_progress -> blocked` carries "note required". The task has a
    // part and a check that are fine, so the gate does not ask for a note.
    let ok = gate(&[("Lid", "current")], &[("fit", true)]);
    for (from, to) in AGENT_ALLOWED.into_iter().filter(|(_, to)| *to != TaskState::Blocked) {
        let d = Dir::new("no-note");
        d.write(
            PROBE_FILE,
            &format!(
                "+++\ntitle = \"Probe\"\nstate = \"{}\"\nparts = [\"Lid\"]\nchecks = [\"fit\"]\n+++\n",
                from.as_str()
            ),
        );
        let t = update(d.path(), "T-0001", Role::Agent, Some(to), None, TODAY, &ok)
            .unwrap_or_else(|e| panic!("{} -> {}: {e}", from.as_str(), to.as_str()));
        assert_eq!(t.state, to);
    }
}

#[test]
fn ag_tk_02_agent_cannot_block_without_a_question() {
    // "`in_progress -> blocked` (note required: the question)". A blank note
    // asks nothing, so it counts as missing.
    for note in [None, Some(""), Some("   "), Some("\n")] {
        let d = Dir::new("blocked-no-note");
        let before = minimal(TaskState::InProgress);
        d.write(PROBE_FILE, &before);
        let r = update(
            d.path(),
            "T-0001",
            Role::Agent,
            Some(TaskState::Blocked),
            note,
            TODAY,
            &no_gate(),
        );
        let e = r.expect_err(&format!("blocked with note {note:?}")).to_string();
        assert_mentions(&e, &["T-0001", "note"]);
        assert_eq!(d.read(PROBE_FILE), before, "a refused update writes nothing");
    }
}

#[test]
fn ag_tk_02_agent_blocks_with_a_question_that_lands_in_the_log() {
    let d = Dir::new("blocked");
    d.write(PROBE_FILE, &minimal(TaskState::InProgress));
    let question = "Is the pin 4 mm or 5 mm?";
    let t = update(
        d.path(),
        "T-0001",
        Role::Agent,
        Some(TaskState::Blocked),
        Some(question),
        TODAY,
        &no_gate(),
    )
    .expect("blocked with a question");
    assert_eq!(t.state, TaskState::Blocked);
    let entry = t.log.last().expect("log entry");
    assert_eq!((entry.date.as_str(), entry.who.as_str()), (TODAY, "agent"));
    // AG-TK-07: `<old> -> <new>: <note>`.
    assert_eq!(entry.text, format!("in_progress -> blocked: {question}"));
}

#[test]
fn ag_tk_02_agent_can_never_set_done_or_rejected() {
    // ADR-0054: the human alone decides done/rejected. Not from any state,
    // not with a note, not with a perfectly green gate.
    let green = gate(&[("Lid", "current")], &[("fit", true)]);
    for from in ALL_STATES {
        for to in [TaskState::Done, TaskState::Rejected].into_iter().filter(|to| *to != from) {
            let d = Dir::new("agent-done");
            let before = format!(
                "+++\ntitle = \"Probe\"\nstate = \"{}\"\nparts = [\"Lid\"]\nchecks = [\"fit\"]\n+++\n",
                from.as_str()
            );
            d.write(PROBE_FILE, &before);
            let r =
                update(d.path(), "T-0001", Role::Agent, Some(to), Some("all green"), TODAY, &green);
            let e =
                r.expect_err(&format!("agent {} -> {}", from.as_str(), to.as_str())).to_string();
            assert_mentions(&e, &["T-0001", from.as_str(), to.as_str(), "agent"]);
            assert_eq!(d.read(PROBE_FILE), before);
        }
    }
}

#[test]
fn ag_tk_02_human_can_always_set_done_or_rejected() {
    // "Role human: every transition, including done and rejected": from every
    // state, without a note, and although nothing is built or checked.
    for from in ALL_STATES {
        for to in [TaskState::Done, TaskState::Rejected].into_iter().filter(|to| *to != from) {
            let d = Dir::new("human-done");
            d.write(
                PROBE_FILE,
                &format!(
                    "+++\ntitle = \"Probe\"\nstate = \"{}\"\nparts = [\"Lid\"]\nchecks = [\"fit\"]\n+++\n",
                    from.as_str()
                ),
            );
            let t = update(d.path(), "T-0001", Role::Human, Some(to), None, TODAY, &no_gate())
                .unwrap_or_else(|e| panic!("human {} -> {}: {e}", from.as_str(), to.as_str()));
            assert_eq!(t.state, to);
            assert_eq!(t.log.last().map(|l| l.who.as_str()), Some("human"));
            assert_eq!(loaded(&d, "T-0001").state, to);
        }
    }
}

// ---------------------------------------------------------------------------
// AG-TK-04: review gate
// ---------------------------------------------------------------------------

/// In progress, needs the parts Lid and Box and the checks fit and wall.
const GATED: &str = "\
+++
title = \"Box with lid\"
state = \"in_progress\"
parts = [\"Lid\", \"Box\"]
checks = [\"fit\", \"wall\"]
+++

Pin connection, 0.2 mm clearance.
";

/// Agent hands `content` over for review with `gate`; on refusal the file
/// must be unchanged.
fn to_review(content: &str, note: Option<&str>, gate: &GateInput) -> Result<Task, String> {
    let d = Dir::new("gate");
    d.write(PROBE_FILE, content);
    let r = update(d.path(), "T-0001", Role::Agent, Some(TaskState::Review), note, TODAY, gate);
    match r {
        Ok(task) => {
            assert_eq!(loaded(&d, "T-0001").state, TaskState::Review);
            Ok(task)
        }
        Err(e) => {
            assert_eq!(d.read(PROBE_FILE), content, "a refused hand-over writes nothing");
            Err(e.to_string())
        }
    }
}

#[test]
fn ag_tk_04_gate_passes_when_all_parts_are_current_and_all_checks_passed() {
    // No note needed: the task has parts and checks. Entries the task
    // does not list (Hook, strength) are none of its business, whatever
    // their state.
    let g = gate(
        &[("Lid", "current"), ("Box", "current"), ("Hook", "failed")],
        &[("fit", true), ("wall", true), ("strength", false)],
    );
    let t = to_review(GATED, None, &g).expect("gate is met");
    assert_eq!(t.state, TaskState::Review);
    assert_eq!(t.log.last().map(|l| l.who.as_str()), Some("agent"));
}

#[test]
fn ag_tk_04_missing_part_fails_the_gate() {
    // "every part in `parts` exists": Box is not in the project.
    let g = gate(&[("Lid", "current")], &[("fit", true), ("wall", true)]);
    let e = to_review(GATED, Some("done"), &g).expect_err("Box does not exist");
    assert_mentions(&e, &["T-0001", "Box"]);
}

#[test]
fn ag_tk_04_part_that_is_not_current_fails_the_gate() {
    // "…and is in state `current`": every other state string fails, also
    // spellings that merely resemble it.
    for state in ["stale", "failed", "building", "queued", "", "Current", "current "] {
        let g = gate(&[("Lid", "current"), ("Box", state)], &[("fit", true), ("wall", true)]);
        let e = to_review(GATED, Some("done"), &g).expect_err(&format!("Box is {state:?}"));
        assert_mentions(&e, &["T-0001", "Box"]);
    }
}

#[test]
fn ag_tk_04_missing_check_fails_the_gate() {
    // "every check in `checks` exists": wall was never declared or never run.
    let g = gate(&[("Lid", "current"), ("Box", "current")], &[("fit", true)]);
    let e = to_review(GATED, Some("done"), &g).expect_err("wall does not exist");
    assert_mentions(&e, &["T-0001", "wall"]);
}

#[test]
fn ag_tk_04_failed_check_fails_the_gate() {
    let g = gate(&[("Lid", "current"), ("Box", "current")], &[("fit", false), ("wall", true)]);
    let e = to_review(GATED, Some("done"), &g).expect_err("fit failed");
    assert_mentions(&e, &["T-0001", "fit"]);
}

#[test]
fn ag_tk_04_every_unmet_condition_is_reported() {
    // Four different failures at once: Lid missing, Box stale, fit failed,
    // wall missing. The error lists all four, not only the first.
    let g = gate(&[("Box", "stale")], &[("fit", false)]);
    let e = to_review(GATED, Some("done"), &g).expect_err("nothing is met");
    assert_mentions(&e, &["T-0001", "Lid", "Box", "fit", "wall"]);

    // Nothing known at all (fresh service): still all four.
    let e = to_review(GATED, None, &no_gate()).expect_err("nothing is known");
    assert_mentions(&e, &["Lid", "Box", "fit", "wall"]);

    // Only parts listed: every unmet part is reported.
    let only_parts =
        "+++\ntitle = \"P\"\nstate = \"in_progress\"\nparts = [\"Arm\", \"Base\", \"Cap\"]\n+++\n";
    let g = gate(&[("Arm", "failed"), ("Base", "current")], &[]);
    let e = to_review(only_parts, None, &g).expect_err("Arm failed, Cap missing");
    assert_mentions(&e, &["Arm", "Cap"]);
}

/// An in-progress task of `kind` with the given header lines for parts and
/// checks (empty string = no line).
fn kind_file(kind: &str, lists: &str) -> String {
    format!("+++\ntitle = \"K\"\nkind = \"{kind}\"\nstate = \"in_progress\"\n{lists}+++\n\nBody.\n")
}

#[test]
fn ag_tk_06_task_and_requirement_need_a_part_and_a_check() {
    // AG-TK-06: "Gate for kinds `task` and `requirement`: at least one part
    // **and** at least one check bound". The bound side is green in every
    // case, so only the missing side can close the gate; a note (the
    // fallback of R4) does not help any more.
    let all_green = gate(&[("Arm", "current")], &[("reach", true)]);
    for kind in ["task", "requirement"] {
        for note in [None, Some("did everything by hand")] {
            let only_parts = kind_file(kind, "parts = [\"Arm\"]\n");
            let e = to_review(&only_parts, note, &all_green)
                .expect_err(&format!("{kind} without a check, note {note:?}"));
            assert_mentions(&e, &["T-0001", "check"]);

            let only_checks = kind_file(kind, "checks = [\"reach\"]\n");
            let e = to_review(&only_checks, note, &all_green)
                .expect_err(&format!("{kind} without a part, note {note:?}"));
            assert_mentions(&e, &["T-0001", "part"]);

            // Nothing bound: both conditions are unmet and both are reported
            // (AG-TK-04: "reports every unmet condition"). Empty lists count
            // as nothing bound, like missing lines.
            for lists in ["", "parts = []\nchecks = []\n"] {
                let e = to_review(&kind_file(kind, lists), note, &all_green)
                    .expect_err(&format!("{kind} with nothing bound, note {note:?}"));
                assert_mentions(&e, &["T-0001", "part", "check"]);
            }
        }
        // One of each, both fine: the gate opens without a note.
        let both = kind_file(kind, "parts = [\"Arm\"]\nchecks = [\"reach\"]\n");
        let t = to_review(&both, None, &all_green).expect("a part and a check, both green");
        assert_eq!(t.state, TaskState::Review);
    }
}

#[test]
fn ag_tk_06_default_kind_is_gated_like_a_task() {
    // AG-TK-01: `kind` defaults to `task`, so a header without `kind` needs
    // a part and a check as well. This is the first fresh-agent run of R5:
    // nothing bound, a note, and the gate must not open.
    let plain = minimal(TaskState::InProgress);
    let unrelated = gate(&[("Lid", "current")], &[("fit", true)]);
    for g in [no_gate(), unrelated] {
        let e = to_review(&plain, Some("Built a hook, four checks green."), &g)
            .expect_err("a task without parts and checks");
        assert_mentions(&e, &["T-0001", "part", "check"]);
    }
}

#[test]
fn ag_tk_06_idea_needs_a_note_only() {
    // AG-TK-06: "kind `idea` needs a note only". With nothing measurable the
    // agent has to say what it did; a blank note says nothing. Other parts
    // and checks of the project do not matter.
    let idea = kind_file("idea", "");
    for note in [None, Some(""), Some("  \t")] {
        for g in [no_gate(), gate(&[("Lid", "current")], &[("fit", true)])] {
            let e = to_review(&idea, note, &g).expect_err(&format!("idea with note {note:?}"));
            assert_mentions(&e, &["T-0001", "note"]);
        }
    }
    let t = to_review(&idea, Some("Compared three hinge ideas, see body."), &no_gate())
        .expect("a note opens the gate of an idea");
    assert_eq!(t.state, TaskState::Review);
    let text = &t.log.last().expect("log entry").text;
    assert!(text.starts_with("in_progress -> review"), "{text:?}");
    assert!(text.contains("Compared three hinge ideas, see body."), "{text:?}");
}

#[test]
fn ag_tk_06_hand_over_log_entry_lists_the_bound_parts_and_checks() {
    // AG-TK-06: "The log entry of a hand-over to review lists the bound
    // parts and checks"; AG-TK-07: it starts with `<old> -> <new>` and
    // carries the note. Whoever reads the log sees what the gate looked at.
    let g = gate(&[("Lid", "current"), ("Box", "current")], &[("fit", true), ("wall", true)]);
    for note in [None, Some("ready, see the render")] {
        let t = to_review(GATED, note, &g).expect("gate is met");
        let entry = t.log.last().expect("log entry");
        assert_eq!((entry.date.as_str(), entry.who.as_str()), (TODAY, "agent"));
        assert!(entry.text.starts_with("in_progress -> review"), "{entry:?}");
        assert_mentions(&entry.text, &["Lid", "Box", "fit", "wall"]);
        if let Some(note) = note {
            assert!(entry.text.contains(note), "{entry:?}");
        }
        assert_eq!(t.log.len(), 1, "one entry for the hand-over: {:?}", t.log);
    }
}

// ---------------------------------------------------------------------------
// AG-TK-07: log text
// ---------------------------------------------------------------------------

#[test]
fn ag_tk_07_state_change_is_logged_as_old_arrow_new_with_the_note() {
    // "A state change is logged as `<old> -> <new>` followed by `: <note>`
    // when a note is given". Checked for transitions that are no hand-over
    // to review (those also list parts and checks, AG-TK-06): the agent's
    // other four and some of the human's.
    let moves = [
        (Role::Agent, TaskState::Open, TaskState::InProgress),
        (Role::Agent, TaskState::InProgress, TaskState::Blocked),
        (Role::Agent, TaskState::Blocked, TaskState::InProgress),
        (Role::Agent, TaskState::Review, TaskState::InProgress),
        (Role::Human, TaskState::Review, TaskState::Done),
        (Role::Human, TaskState::Review, TaskState::Rejected),
        (Role::Human, TaskState::Done, TaskState::Open),
        (Role::Human, TaskState::Blocked, TaskState::InProgress),
    ];
    for (role, from, to) in moves {
        let arrow = format!("{} -> {}", from.as_str(), to.as_str());
        let d = Dir::new("log-text");
        d.write(PROBE_FILE, &bound(from));
        let t = update(d.path(), "T-0001", role, Some(to), Some("M3 or M4?"), TODAY, &green())
            .unwrap_or_else(|e| panic!("{arrow}: {e}"));
        let expected = LogEntry {
            date: TODAY.to_owned(),
            who: who(role).to_owned(),
            text: format!("{arrow}: M3 or M4?"),
        };
        assert_eq!(t.log, [expected], "{arrow} with a note");
        let line = format!("- {TODAY} {}: {arrow}: M3 or M4?", who(role));
        assert_eq!(count_lines(&d.read(PROBE_FILE), &line), 1, "{}", d.read(PROBE_FILE));

        // Without a note (the agent's `blocked` needs one): only the arrow.
        // A blank note is no note.
        if to == TaskState::Blocked && role == Role::Agent {
            continue;
        }
        for note in [None, Some(""), Some("  ")] {
            let d = Dir::new("log-text-bare");
            d.write(PROBE_FILE, &bound(from));
            let t = update(d.path(), "T-0001", role, Some(to), note, TODAY, &green())
                .unwrap_or_else(|e| panic!("{arrow} with note {note:?}: {e}"));
            assert_eq!(t.log.len(), 1);
            assert_eq!(t.log[0].text, arrow, "note {note:?}");
        }
    }
}

#[test]
fn ag_tk_07_note_only_update_logs_exactly_the_note() {
    // "a note-only update logs the note": no arrow is added, and an arrow
    // inside the note is the note's business.
    for role in [Role::Agent, Role::Human] {
        let d = Dir::new("log-note");
        d.write(PROBE_FILE, &bound(TaskState::InProgress));
        let note = Some("see: a -> b");
        let t = update(d.path(), "T-0001", role, None, note, TODAY, &green())
            .expect("a note needs no transition");
        assert_eq!(t.state, TaskState::InProgress);
        assert_eq!(t.log.len(), 1);
        assert_eq!(t.log[0].text, "see: a -> b");
    }
}

#[test]
fn ag_tk_04_a_note_does_not_replace_parts_and_checks() {
    // The note opens the gate of an idea (AG-TK-06); it is no way around a
    // red gate.
    let e = to_review(GATED, Some("trust me, it fits"), &no_gate()).expect_err("gate is red");
    assert_mentions(&e, &["Lid", "Box", "fit", "wall"]);
}

#[test]
fn ag_tk_04_gate_guards_only_the_way_into_review() {
    // The other agent transitions work while nothing is built: taking the
    // task, asking a question, resuming, and taking it back from review.
    let moves = [
        (TaskState::Open, TaskState::InProgress),
        (TaskState::InProgress, TaskState::Blocked),
        (TaskState::Blocked, TaskState::InProgress),
        (TaskState::Review, TaskState::InProgress),
    ];
    for (from, to) in moves {
        let d = Dir::new("gate-other");
        d.write(PROBE_FILE, &GATED.replace("in_progress", from.as_str()));
        let t = update(d.path(), "T-0001", Role::Agent, Some(to), Some("why?"), TODAY, &no_gate())
            .unwrap_or_else(|e| panic!("{} -> {}: {e}", from.as_str(), to.as_str()));
        assert_eq!(t.state, to);
    }
}

// ---------------------------------------------------------------------------
// Points the module decides where the requirements are open (module comment
// of qoob3d_core::tasks and the doc comments of `create` and `Task::body`).
// ---------------------------------------------------------------------------

#[test]
fn ag_tk_01_log_lines_of_another_form_are_no_entries() {
    // AG-TK-01: an entry is `- <YYYY-MM-DD> <who>: <text>`. A date of another
    // shape, an empty `who` or a `who` with a space is not of that form, so
    // the line is no entry (it stays in the file). The text may hold `: `.
    let d = Dir::new("log-form");
    d.write(
        "tasks/0001-log.md",
        "+++\ntitle = \"Log\"\n+++\n\n## Log\n\
         - 2026-11-20 human: kept\n\
         - someday human: no date\n\
         - 2026-11-2 human: date too short\n\
         - 2026-11-201 human: date too long\n\
         - 2026/11/20 human: slashes\n\
         - 2026-1x-20 human: letter in the month\n\
         - 20261120xx human: no dashes\n\
         - 2026-11-20 : no who\n\
         - 2026-11-20 the human: who with a space\n\
         - 2026-11-21 agent: kept too: with a colon\n",
    );
    let log = loaded(&d, "T-0001").log;
    let entry = |date: &str, who: &str, text: &str| LogEntry {
        date: date.to_owned(),
        who: who.to_owned(),
        text: text.to_owned(),
    };
    assert_eq!(
        log,
        [
            entry("2026-11-20", "human", "kept"),
            entry("2026-11-21", "agent", "kept too: with a colon")
        ]
    );
}

#[test]
fn ag_tk_03_a_date_not_shaped_yyyy_mm_dd_is_refused_and_nothing_written() {
    // The date of a log entry must be `YYYY-MM-DD` (AG-TK-01), otherwise
    // the entry written would not load back as one.
    for date in ["2026-11-2", "2026-11-201", "20261120xx", "2026_11_20", "2026-11-2x", "", "today"]
    {
        let d = Dir::new("bad-date");
        let old = minimal(TaskState::Open);
        d.write("tasks/0001-probe.md", &old);
        let e = update(d.path(), "T-0001", Role::Human, None, Some("note"), date, &no_gate())
            .expect_err(date);
        assert!(matches!(e, TaskError::InvalidDate { .. }), "{date:?}: {e}");
        assert_eq!(d.read("tasks/0001-probe.md"), old, "{date:?}: a refused change writes nothing");
    }
}

#[test]
fn ag_tk_03_note_with_the_current_state_leaves_the_header_byte_identical() {
    // "A new state equal to the current one is no state change": a note with
    // state = the current state only appends the note (AG-TK-03), even when
    // the header spells the state unusually or leaves it to the default.
    for old in [
        "+++\ntitle = \"Box\"\nstate = 'open'   # as typed\n+++\nA box.\n\n## Log\n",
        "+++\ntitle = \"Box\"\n+++\nA box.\n\n## Log\n",
    ] {
        let d = Dir::new("same-state");
        d.write("tasks/0001-box.md", old);
        let t = update(
            d.path(),
            "T-0001",
            Role::Human,
            Some(TaskState::Open),
            Some("still open"),
            TODAY,
            &no_gate(),
        )
        .expect("note with the current state");
        assert_eq!(t.state, TaskState::Open);
        // AG-TK-07: a note-only update logs the note; who = human.
        assert_eq!(d.read("tasks/0001-box.md"), format!("{old}- {TODAY} human: still open\n"));
    }
}

#[test]
fn ag_tk_01_body_drops_blank_lines_around_it_but_keeps_indentation() {
    // `Task::body`: the Markdown "without surrounding blank lines"; blank
    // lines may hold spaces. The indentation of the first line is Markdown
    // (a code block) and stays.
    let d = Dir::new("body-edges");
    d.write(
        "tasks/0001-code.md",
        "+++\ntitle = \"Code\"\n+++\n\n  \n\n    indented code\nnext line\n\n \n## Log\n",
    );
    assert_eq!(loaded(&d, "T-0001").body, "    indented code\nnext line");
    let d = Dir::new("body-edges-2");
    d.write("tasks/0001-code.md", "+++\ntitle = \"Code\"\n+++\n\n\n\tTab first\n");
    assert_eq!(loaded(&d, "T-0001").body, "\tTab first");
}

#[test]
fn ag_tk_01_non_ascii_body_right_before_the_log_heading_loads() {
    // Bodies are the human's words, often not ASCII; the last line before
    // `## Log` may end in multi-byte characters (here `€`, 3 bytes, followed
    // by 5 ASCII bytes and the line break, and variants shifted by a byte).
    for line in ["Budget: 5 €/year", "Budget: 5 €/yea", "Budget: 5 €/yearly", "Angle 30° × 2 µm"]
    {
        let d = Dir::new("body-utf8");
        d.write(
            "tasks/0001-budget.md",
            &format!("+++\ntitle = \"Budget\"\n+++\n{line}\n## Log\n- 2026-11-20 human: ok\n"),
        );
        let t = loaded(&d, "T-0001");
        assert_eq!(t.body, line);
        assert_eq!(t.log.len(), 1, "{:?}", t.log);
        assert_eq!(t.log[0].text, "ok");
    }
}

#[test]
fn ag_tk_03_created_file_ends_with_exactly_one_line_break() {
    // The body is written as given and the file ends in one line break (a
    // text file, so an appended log line starts on its own line and Git
    // shows no "No newline at end of file"); no blank line is added.
    for body in ["Hello", "Hello\n"] {
        let d = Dir::new("create-newline");
        let t = create(d.path(), "Hi", TaskKind::Task, body, &[], &[], DATE).expect("create");
        let text = d.read(&slash(&t.file));
        assert!(text.ends_with("\nHello\n"), "{body:?}: {text:?}");
        assert!(!text.ends_with("\n\n"), "{body:?}: {text:?}");
    }
}

#[test]
fn ag_tk_03_slug_collapses_runs_and_trims_dashes() {
    // `create` doc: every character outside `[a-z0-9]` becomes `-`, runs of
    // `-` collapse, `-` at both ends is trimmed.
    for (title, slug) in [
        ("  Hello  World  ", "hello-world"),
        ("(padded)", "padded"),
        ("a  --  b", "a-b"),
        ("\u{d6}l", "l"),
        ("--x--y--", "x-y"),
    ] {
        let d = Dir::new("slug-runs");
        assert_eq!(slug_of(&new_task(&d, title)), slug, "title {title:?}");
    }
}

#[test]
fn ag_tk_03_numbers_end_at_9999() {
    // Four digits (AG-TK-01): 9999 is the last number, the next create is an
    // error and writes nothing.
    let d = Dir::new("numbers");
    d.write("tasks/9998-almost.md", "+++\ntitle = \"Almost\"\n+++\n");
    let t = new_task(&d, "Last");
    assert_eq!(t.id, "T-9999");
    assert_eq!(slash(&t.file), "tasks/9999-last.md");
    let e = create(d.path(), "One more", TaskKind::Task, "", &[], &[], DATE)
        .expect_err("no number after 9999");
    assert!(matches!(e, TaskError::NumbersUsedUp), "{e}");
    assert_mentions(&e.to_string(), &["9999"]);
    assert_eq!(d.task_files(), ["9998-almost.md", "9999-last.md"]);
}

#[test]
fn ag_tk_03_other_files_and_sub_folders_in_tasks_are_ignored() {
    // Module decision: in `tasks/`, sub folders and files not ending in `.md`
    // are ignored (also by create, which picks the next number).
    let d = Dir::new("ignored");
    d.write("tasks/0001-a.md", "+++\ntitle = \"A\"\n+++\n")
        .write("tasks/notes.txt", "not a task")
        .write("tasks/.DS_Store", "")
        .write("tasks/0005-draft.md.bak", "")
        .write("tasks/archive.md/0009-old.md", "+++\ntitle = \"Old\"\n+++\n");
    let ids: Vec<String> = load(&d).all().iter().map(|t| t.id.clone()).collect();
    assert_eq!(ids, ["T-0001"]);
    assert_eq!(new_task(&d, "Next").id, "T-0002");
}
