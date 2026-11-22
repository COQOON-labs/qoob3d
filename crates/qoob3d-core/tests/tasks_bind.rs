//! Binding parts and checks to a task (R5, pure layer):
//! docs/requirements/agent.md §6.2 AG-TK-05, ADR-0054. The first fresh-agent
//! run ended with a review gate that proved nothing, because the task had no
//! parts and checks bound and the agent could not bind them.
//!
//! Interface under test (new, in `qoob3d_core::tasks`):
//!
//! ```text
//! bind(dir, id, role, parts: Option<&[String]>, checks: Option<&[String]>, date)
//!     -> Result<Task, TaskError>
//! ```
//!
//! `Some(list)` replaces the list of the header, `None` leaves it alone.
//! This file is separate from `tasks.rs` so that a missing `bind` breaks the
//! build of these tests only.
//!
//! All expected values come from the requirement text. Where it leaves a
//! detail open the tests stay tolerant and say so:
//!
//! - Whether binding appends a log entry is not specified. "The rest of the
//!   file stays byte-identical" is checked like AG-TK-03 in `tasks.rs`: after
//!   removing the header's `parts` and `checks` lines from both versions, the
//!   old file must be a byte prefix of the new one, and whatever follows may
//!   only be the `## Log` heading and well-formed log lines of the role.
//! - Where a missing `parts`/`checks` line is inserted into the header and
//!   how the array is spelled is not specified; the lists are compared as
//!   loaded.
//! - Which states the role human may bind in is not specified (AG-TK-05
//!   speaks of the agent); only `open` is asserted for the human.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use qoob3d_core::tasks::{
    self, GateInput, Role, Task, TaskError, TaskKind, TaskState, Tasks, update,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `None` for a list that `bind` leaves alone.
const KEEP: Option<&[String]> = None;

/// `tasks::bind` (see the file comment for its signature); the lists may be
/// vectors, arrays or slices of `String`.
fn bind<P: AsRef<[String]>, C: AsRef<[String]>>(
    dir: &Path,
    id: &str,
    role: Role,
    parts: Option<P>,
    checks: Option<C>,
    date: &str,
) -> Result<Task, TaskError> {
    let parts = parts.as_ref().map(AsRef::as_ref);
    let checks = checks.as_ref().map(AsRef::as_ref);
    tasks::bind(dir, id, role, parts, checks, date)
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique temporary project folder, removed on drop.
struct Dir {
    root: PathBuf,
}

impl Dir {
    fn new(tag: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("qoob3d-core-bind-{tag}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("tasks")).expect("create temp project");
        Self { root }
    }

    /// A project whose only task `T-0001` has the content `text`.
    fn with_task(tag: &str, text: &str) -> Self {
        let d = Self::new(tag);
        std::fs::write(d.root.join(FILE), text).expect("write task file");
        d
    }

    fn read(&self) -> String {
        std::fs::read_to_string(self.root.join(FILE)).expect("read task file")
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn loaded(&self) -> Task {
        let tasks = Tasks::load(&self.root).expect("tasks load");
        tasks.get(ID).expect("T-0001 is loaded").clone()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

const FILE: &str = "tasks/0001-hook.md";
const ID: &str = "T-0001";
const TODAY: &str = "2026-11-23";

const ALL_STATES: [TaskState; 6] = [
    TaskState::Open,
    TaskState::InProgress,
    TaskState::Blocked,
    TaskState::Review,
    TaskState::Done,
    TaskState::Rejected,
];

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn assert_mentions(message: &str, words: &[&str]) {
    for w in words {
        assert!(message.contains(w), "error must name {w:?}: {message}");
    }
}

/// A hand-written task with comments, uneven spacing, single-quoted
/// strings, decoy `parts`/`checks` lines in the body and a log.
fn hand_written(state: TaskState) -> String {
    format!(
        "\
+++
# written by hand, the comment must survive
title = \"Wall hook\"   # trailing comment
kind = \"task\"
state = \"{}\"
checks = [ \"old_check\",'second' ]   # to be replaced
parts = ['OldPart']
created = \"2026-11-20\"
+++

A hook for a bicycle helmet.

parts = [\"Decoy\"] is a line of the body.
checks = [\"decoy\"] too.

## Log
- 2026-11-20 human: created by hand
",
        state.as_str()
    )
}

/// A free-text task as the human of the R5 run wrote it: nothing bound, no
/// `parts` and `checks` lines at all.
const FREE_TEXT: &str = "\
+++
title = \"Wall hook for a bicycle helmet\"
state = \"in_progress\"
+++

It has to carry 500 g and hang on one screw.
";

fn is_list_line(line: &str) -> bool {
    ["parts", "checks"].iter().any(|key| {
        line.trim_start().strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
    })
}

/// `text` without the `parts` and `checks` lines of its header (lines
/// between the first two `+++`). Such lines in the body are kept.
fn without_list_lines(text: &str) -> String {
    let mut out = String::new();
    let mut fences = 0;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare == "+++" {
            fences += 1;
        } else if fences == 1 && is_list_line(bare) {
            continue;
        }
        out.push_str(line);
    }
    out
}

/// `YYYY-MM-DD`.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter()
            .enumerate()
            .all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// AG-TK-05 "the header lines are rewritten, the rest of the file stays
/// byte-identical": apart from the header's `parts`/`checks` lines, `new` is
/// `old`, optionally followed by log lines of `who` (see the file comment).
fn assert_rest_identical(old: &str, new: &str, who: &str) {
    let (o, n) = (without_list_lines(old), without_list_lines(new));
    assert!(
        n.starts_with(&o),
        "everything but the parts/checks lines must stay byte-identical\n--- old\n{old}\n--- new\n{new}"
    );
    for line in n[o.len()..].lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        let entry = line.strip_prefix("- ").and_then(|rest| rest.split_once(' '));
        let is_log_line = entry
            .is_some_and(|(date, text)| is_date(date) && text.starts_with(&format!("{who}: ")));
        assert!(line == "## Log" || is_log_line, "appended line is no log entry: {line:?}");
    }
}

fn green(parts: &[&str], checks: &[&str]) -> GateInput {
    GateInput {
        parts: parts.iter().map(|n| ((*n).to_owned(), "current".to_owned())).collect(),
        checks: checks.iter().map(|n| ((*n).to_owned(), true)).collect(),
    }
}

// ---------------------------------------------------------------------------
// AG-TK-05: replace the lists, keep the rest
// ---------------------------------------------------------------------------

#[test]
fn ag_tk_05_agent_binds_parts_and_checks_in_open_and_in_progress() {
    // "The agent can bind parts and checks to a task while it is `open` or
    // `in_progress` … (replace the lists; the header lines are rewritten,
    // the rest of the file stays byte-identical)".
    for state in [TaskState::Open, TaskState::InProgress] {
        let old = hand_written(state);
        let d = Dir::with_task("bind", &old);
        let before = d.loaded();
        let (parts, checks) = (strings(&["Hook", "Plate"]), strings(&["load", "screw_fit"]));
        let t = bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY)
            .unwrap_or_else(|e| panic!("agent binds in {}: {e}", state.as_str()));

        // Replaced, not merged: OldPart, old_check and second are gone.
        assert_eq!(t.parts, parts, "state {}", state.as_str());
        assert_eq!(t.checks, checks, "state {}", state.as_str());
        // Binding is no state change.
        assert_eq!(t.state, state);
        assert_eq!(
            (&t.id, &t.file, &t.title, t.kind, &t.created, &t.body),
            (&before.id, &before.file, &before.title, before.kind, &before.created, &before.body)
        );
        assert_eq!(t.log[..before.log.len()], before.log[..], "old log entries stay");
        assert_eq!(d.loaded(), t, "bind returns the task as written");

        let new = d.read();
        assert_rest_identical(&old, &new, "agent");
        // The decoy lines of the body are body, not header.
        assert!(new.contains("parts = [\"Decoy\"] is a line of the body."), "{new}");
        assert!(new.contains("checks = [\"decoy\"] too."), "{new}");
        assert!(new.contains("# written by hand, the comment must survive"), "{new}");
    }
}

#[test]
fn ag_tk_05_binding_adds_the_lists_to_a_header_without_them() {
    // The free-text task of the R5 run has no `parts`/`checks` lines; the
    // binding has to be stored although there is no line to rewrite.
    let d = Dir::with_task("bind-free", FREE_TEXT);
    let (parts, checks) = (strings(&["Hook"]), strings(&["load", "screw_fit"]));
    let t = bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY)
        .expect("agent binds in in_progress");
    assert_eq!((&t.parts, &t.checks), (&parts, &checks));
    assert_eq!(t.title, "Wall hook for a bicycle helmet");
    assert_eq!(t.kind, TaskKind::Task);
    assert_eq!(t.state, TaskState::InProgress);
    assert_eq!(t.body.trim(), "It has to carry 500 g and hang on one screw.");
    assert_eq!(d.loaded(), t);
    assert_rest_identical(FREE_TEXT, &d.read(), "agent");
}

#[test]
fn ag_tk_05_none_leaves_a_list_alone_and_an_empty_list_clears_it() {
    // `Some(list)` replaces, `None` keeps: the service passes `None` for a
    // field the request does not carry.
    let old = hand_written(TaskState::InProgress);
    let d = Dir::with_task("bind-partial", &old);
    let t = bind(d.path(), ID, Role::Agent, Some(&strings(&["Hook"])), KEEP, TODAY)
        .expect("bind only parts");
    assert_eq!(t.parts, ["Hook"]);
    assert_eq!(t.checks, ["old_check", "second"], "checks were not given");
    let t = bind(d.path(), ID, Role::Agent, KEEP, Some(&strings(&["load"])), TODAY)
        .expect("bind only checks");
    assert_eq!(t.parts, ["Hook"], "parts were not given");
    assert_eq!(t.checks, ["load"]);
    // "replace the lists": the empty list is a list.
    let none: &[String] = &[];
    let t = bind(d.path(), ID, Role::Agent, Some(none), Some(none), TODAY).expect("unbind");
    assert!(t.parts.is_empty() && t.checks.is_empty(), "{t:?}");
    assert_eq!(d.loaded(), t);
    assert_rest_identical(&old, &d.read(), "agent");
}

#[test]
fn ag_tk_05_binding_twice_keeps_one_line_per_list() {
    // Rewriting must not leave the old line behind (TOML refuses a key
    // defined twice, so the file would not load any more).
    let d = Dir::with_task("bind-twice", FREE_TEXT);
    for (parts, checks) in [(&["A"][..], &["a"][..]), (&["B", "C"][..], &["b"][..])] {
        let (parts, checks) = (strings(parts), strings(checks));
        let t = bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY).expect("bind");
        assert_eq!((&t.parts, &t.checks), (&parts, &checks));
        assert_eq!(d.loaded(), t);
    }
    assert_rest_identical(FREE_TEXT, &d.read(), "agent");
}

#[test]
fn ag_tk_05_names_that_need_escaping_round_trip() {
    // The header is TOML: quotes, backslashes, `#`, brackets, commas and
    // non-ASCII in names must be written so the file loads back identically
    // (same rule as `create`, AG-TK-03). `Crane column` is the part name of
    // AG-HD-04.
    let old = hand_written(TaskState::Open);
    let d = Dir::with_task("bind-escape", &old);
    let parts = strings(&["Crane column", "Lid \"A\"", "Back\\slash", "a, b]", "Geh\u{e4}use"]);
    let checks = strings(&["fit #1", "it's tight", "parts = [\"x\"]"]);
    let t = bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY).expect("bind");
    assert_eq!((&t.parts, &t.checks), (&parts, &checks));
    assert_eq!(t.title, "Wall hook", "the header still parses as before");
    assert_eq!(d.loaded(), t);
    assert_rest_identical(&old, &d.read(), "agent");
}

#[test]
fn ag_tk_05_multi_line_arrays_are_rewritten_or_refused_never_corrupted() {
    // A human may spread a TOML array over several lines. Whether `bind`
    // can rewrite that is not specified; it must either store exactly the
    // new lists or refuse and leave the file alone (as `update` does for a
    // `state` line it cannot patch).
    let old = "\
+++
title = \"Spread\"
state = \"open\"
parts = [
  \"One\",
  \"Two\",
]
checks = [\"fit\"]
+++

Body.
";
    let d = Dir::with_task("bind-multi-line", old);
    assert_eq!(d.loaded().parts, ["One", "Two"], "test data: the header is valid");
    let (parts, checks) = (strings(&["Three"]), strings(&["fit", "wall"]));
    match bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY) {
        Ok(t) => {
            assert_eq!((&t.parts, &t.checks), (&parts, &checks));
            let reloaded = d.loaded();
            assert_eq!(reloaded, t);
            assert_eq!(reloaded.title, "Spread");
            assert_eq!(reloaded.body.trim(), "Body.");
        }
        Err(_) => assert_eq!(d.read(), old, "a refused binding writes nothing"),
    }
}

// ---------------------------------------------------------------------------
// AG-TK-05: who may bind when
// ---------------------------------------------------------------------------

#[test]
fn ag_tk_05_agent_cannot_bind_outside_open_and_in_progress() {
    // "…while it is `open` or `in_progress`": in `blocked` the human is
    // asked, in `review` the human looks at exactly what the gate checked,
    // `done`/`rejected` are decided. The error names task and state (as the
    // transition errors of AG-TK-02 do); nothing is written.
    let allowed = [TaskState::Open, TaskState::InProgress];
    for state in ALL_STATES.into_iter().filter(|s| !allowed.contains(s)) {
        let old = hand_written(state);
        let d = Dir::with_task("bind-refused", &old);
        let lists = (strings(&["Hook"]), strings(&["load"]));
        for (parts, checks) in [
            (Some(&lists.0[..]), Some(&lists.1[..])),
            (Some(&lists.0[..]), None),
            (None, Some(&lists.1[..])),
            (Some(&[][..]), Some(&[][..])),
        ] {
            let e = bind(d.path(), ID, Role::Agent, parts, checks, TODAY)
                .expect_err(&format!("agent binds in {}", state.as_str()))
                .to_string();
            assert_mentions(&e, &[ID, state.as_str()]);
            assert_eq!(d.read(), old, "a refused binding writes nothing");
        }
    }
}

#[test]
fn ag_tk_05_human_can_bind_too() {
    // The role human may do everything the agent may (AG-TK-02).
    let old = hand_written(TaskState::Open);
    let d = Dir::with_task("bind-human", &old);
    let (parts, checks) = (strings(&["Hook"]), strings(&["load"]));
    let t = bind(d.path(), ID, Role::Human, Some(&parts), Some(&checks), TODAY)
        .expect("human binds in open");
    assert_eq!((&t.parts, &t.checks), (&parts, &checks));
    assert_rest_identical(&old, &d.read(), "human");
}

#[test]
fn ag_tk_05_binding_an_unknown_task_is_an_error_naming_it() {
    let old = hand_written(TaskState::Open);
    let d = Dir::with_task("bind-unknown", &old);
    for id in ["T-0099", "hook", ""] {
        let e = bind(d.path(), id, Role::Agent, Some(&strings(&["Hook"])), KEEP, TODAY)
            .expect_err(&format!("{id:?} is no task"))
            .to_string();
        assert_mentions(&e, &[id]);
    }
    assert_eq!(d.read(), old, "a failed binding writes nothing");
}

#[test]
fn ag_tk_05_binding_touches_only_the_addressed_task() {
    let d = Dir::with_task("bind-other", &hand_written(TaskState::Open));
    let other = "+++\ntitle = \"Other\"\nparts = [\"Keep\"]\n+++\nUntouched.\n";
    let other_file = d.path().join("tasks/0002-other.md");
    std::fs::write(&other_file, other).expect("write the other task");
    bind(d.path(), ID, Role::Agent, Some(&strings(&["Hook"])), Some(&strings(&["load"])), TODAY)
        .expect("bind");
    assert_eq!(std::fs::read_to_string(&other_file).expect("read").as_str(), other);
    let mut names: Vec<String> = std::fs::read_dir(d.path().join("tasks"))
        .expect("tasks/")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["0001-hook.md", "0002-other.md"], "no file added, renamed or left over");
}

// ---------------------------------------------------------------------------
// AG-TK-05 with the gate (AG-TK-04, AG-TK-06)
// ---------------------------------------------------------------------------

#[test]
fn ag_tk_05_unknown_names_are_accepted_at_binding_time_and_fail_the_gate_later() {
    // "Unknown names are accepted at binding time and fail the gate later":
    // the pure layer cannot know the project's parts anyway; the gate input
    // of the service has no `Ghost` and no `no_such_check`.
    let d = Dir::with_task("bind-ghost", FREE_TEXT);
    let (parts, checks) = (strings(&["Ghost"]), strings(&["no_such_check"]));
    bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY)
        .expect("unknown names are accepted");
    let before = d.read();
    let nothing = GateInput { parts: BTreeMap::new(), checks: BTreeMap::new() };
    for gate in [nothing, green(&["Hook"], &["load"])] {
        let review = Some(TaskState::Review);
        let e = update(d.path(), ID, Role::Agent, review, Some("done"), TODAY, &gate)
            .expect_err("Ghost and no_such_check do not exist")
            .to_string();
        assert_mentions(&e, &[ID, "Ghost", "no_such_check"]);
        assert_eq!(d.read(), before, "a refused hand-over writes nothing");
    }
}

#[test]
fn ag_tk_05_the_r5_flow_bind_then_hand_over() {
    // The repeated fresh-agent run (acceptance R5): a free-text task, the
    // agent binds what it built, the gate looks at exactly that, and the
    // hand-over entry lists it (AG-TK-06) after `in_progress -> review`
    // (AG-TK-07).
    let d = Dir::with_task("bind-flow", FREE_TEXT);
    let gate = green(&["Hook"], &["load", "screw_fit"]);
    let review = Some(TaskState::Review);
    // Nothing bound: a note does not open the gate of a task (AG-TK-06).
    let e = update(d.path(), ID, Role::Agent, review, Some("all green"), TODAY, &gate)
        .expect_err("nothing is bound")
        .to_string();
    assert_mentions(&e, &[ID, "part", "check"]);

    let (parts, checks) = (strings(&["Hook"]), strings(&["load", "screw_fit"]));
    bind(d.path(), ID, Role::Agent, Some(&parts), Some(&checks), TODAY).expect("bind");
    let t = update(d.path(), ID, Role::Agent, review, Some("all green"), TODAY, &gate)
        .expect("a part and two checks, all green");
    assert_eq!(t.state, TaskState::Review);
    assert_eq!((&t.parts, &t.checks), (&parts, &checks));
    let entry = t.log.last().expect("hand-over entry");
    assert_eq!((entry.date.as_str(), entry.who.as_str()), (TODAY, "agent"));
    assert!(entry.text.starts_with("in_progress -> review"), "{entry:?}");
    assert_mentions(&entry.text, &["Hook", "load", "screw_fit", "all green"]);

    // In review the lists are frozen for the agent.
    let none: &[String] = &[];
    let e = bind(d.path(), ID, Role::Agent, Some(none), Some(none), TODAY)
        .expect_err("agent binds in review")
        .to_string();
    assert_mentions(&e, &[ID, "review"]);
    assert_eq!(d.loaded().parts, parts);
}
