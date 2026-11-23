//! Guide and reference for agents (docs/requirements/agent.md §6.3,
//! AG-MC-07..10; ADR-0054): the MCP server `qoob3d mcp [DIR]` over stdio
//! against the real binary. Every test names its requirement ID.
//!
//! What an agent reads is tested by using it: the minimal project of the
//! guide is written, built and checked through the server, and every example
//! of the served reference runs through `run_script`, both in a project made
//! by `qoob3d init`.
//!
//! Transport and client shared with `tests/mcp.rs` (`common/mcp.rs`): one JSON-RPC 2.0 message per
//! line; [`Mcp`] reads stdout in a thread and waits for every answer with a
//! timeout. Expected numbers are derived by hand next to the assertions;
//! `VOL` = 1e-6 relative for volumes of boxes, `POS` = 1e-6 mm for bounding
//! boxes (AG-HD-11: tight to 1e-6 mm).
//!
//! Processes and folders: [`Mcp`] closes stdin and kills the server on drop;
//! `support::Project` stops the service and removes the folder.

#[path = "common/artifacts.rs"]
mod artifacts;
#[path = "common/inspect.rs"]
mod inspect;
#[path = "common/mcp.rs"]
mod mcp_client;
#[path = "common/agent.rs"]
mod support;

use std::time::Duration;

use artifacts::{git, git_init, git_tracked, png_colors, png_size};
use inspect::assert_vec;
use mcp_client::{Mcp, base64_decode, short};
use serde_json::{Value, json};
use support::{Project, Service, VOL, assert_rel, cli};

/// Bounding boxes in results (AG-HD-11).
const POS: f64 = 1e-6;
/// Answers to protocol messages that start no work.
const QUICK: Duration = Duration::from_secs(30);
/// Answers to tool calls (service start, builds, renders on software Vulkan).
const TOOL: Duration = Duration::from_secs(180);
const CLI: Duration = Duration::from_secs(90);
/// The render views (AG-RD-01).
const VIEWS: [&str; 7] = ["iso", "front", "back", "left", "right", "top", "bottom"];

// ===========================================================================
// MCP client (shared in common/mcp.rs)
// ===========================================================================

impl Mcp {
    /// `resources/read` of one text resource.
    fn resource(&mut self, uri: &str) -> String {
        let r = self.request("resources/read", &json!({"uri": uri}), QUICK);
        r["result"]["contents"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{uri}: no text: {}", short(&r.to_string())))
            .to_owned()
    }
}

// ===========================================================================
// Projects and Markdown
// ===========================================================================

/// A project made by `qoob3d init` (AG-SV-08), with nothing in it that the
/// test wrote: what the guide and the reference show must work there.
fn fresh_project(name: &str) -> Project {
    let p = Project::new(name, "");
    // `Project::new` writes a skeleton of its own; `init` starts from an
    // empty folder.
    std::fs::remove_dir_all(&p.dir).expect("remove the test skeleton");
    std::fs::create_dir_all(&p.dir).expect("create the empty folder");
    let out = cli(&p.root, &["init", "project"], CLI);
    assert_eq!(out.code, Some(0), "qoob3d init: {}\n{}", out.stdout, out.stderr);
    assert!(p.path("qoob3d.toml").is_file() && p.path("parts").is_dir(), "init made no project");
    p
}

/// A fenced code block of a Markdown text.
struct Block {
    /// The info string after the fence (`rhai`, `toml`, …).
    lang: String,
    /// The last non-empty line before the block.
    before: String,
    /// 1-based line of the opening fence.
    line: usize,
    body: String,
}

fn fenced_blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut open: Option<(String, String, usize, Vec<&str>)> = None;
    let mut before = "";
    for (i, line) in markdown.lines().enumerate() {
        let trimmed = line.trim_start();
        match open.take() {
            None if trimmed.starts_with("```") => {
                let lang = trimmed.trim_start_matches('`').trim().to_owned();
                open = Some((lang, before.to_owned(), i + 1, Vec::new()));
            }
            None => {
                if !trimmed.is_empty() {
                    before = line;
                }
            }
            Some((lang, before, line, body)) if trimmed.starts_with("```") => {
                blocks.push(Block { lang, before, line, body: body.join("\n") + "\n" });
            }
            Some((lang, before, start, mut body)) => {
                body.push(line);
                open = Some((lang, before, start, body));
            }
        }
    }
    assert!(open.is_none(), "unterminated fenced block");
    blocks
}

/// A project file path (`values/x.toml`, `parts/x.rhai`, `checks/x.rhai`)
/// named in `text`; patterns such as `parts/*.rhai` are no file names.
fn file_path_in(text: &str) -> Option<String> {
    let name = |c: char| c.is_ascii_alphanumeric() || "_-./".contains(c);
    text.split(|c: char| !name(c)).map(|t| t.trim_end_matches('.')).find_map(|token| {
        let (folder, file) = token.split_once('/')?;
        let (stem, extension) = file.rsplit_once('.')?;
        let known = match folder {
            "values" => extension == "toml",
            "parts" | "checks" => extension == "rhai",
            _ => false,
        };
        (known && !stem.is_empty() && !stem.contains('/')).then(|| token.to_owned())
    })
}

/// The files of the minimal project in the guide: every `toml`/`rhai` block
/// whose first line (a comment) or whose preceding line names its path.
fn guide_files(guide: &str) -> Vec<(String, String)> {
    fenced_blocks(guide)
        .into_iter()
        .filter(|b| b.lang == "toml" || b.lang == "rhai")
        .filter_map(|b| {
            let first = b.body.lines().next().unwrap_or_default();
            let path = file_path_in(first).or_else(|| file_path_in(&b.before))?;
            Some((path, b.body))
        })
        .collect()
}

#[test]
fn the_markdown_helpers_find_blocks_and_paths() {
    let md = "Text\n\n`parts/hook.rhai`:\n```rhai\nlet a = 1;\n```\n\n```toml\n# values/dims.toml\n\
              [w]\n```\n\n```\nplain\n```\n";
    let files = guide_files(md);
    assert_eq!(
        files,
        [
            ("parts/hook.rhai".to_owned(), "let a = 1;\n".to_owned()),
            ("values/dims.toml".to_owned(), "# values/dims.toml\n[w]\n".to_owned()),
        ]
    );
    assert_eq!(fenced_blocks(md).len(), 3);
    assert_eq!(fenced_blocks(md)[0].line, 4);
    for no_file in ["parts/*.rhai", "values/", "out/a.stl", "lib/a.rhai", "parts/a.toml", ""] {
        assert_eq!(file_path_in(no_file), None, "{no_file}");
    }
    assert_eq!(file_path_in("see checks/fit.rhai."), Some("checks/fit.rhai".to_owned()));
}

// ===========================================================================
// AG-MC-07: the guide
// ===========================================================================

#[test]
fn ag_mc_07_the_guide_fits_120_lines_and_states_the_dimension_policy() {
    let p = fresh_project("guide-policy");
    let mut mcp = Mcp::open(&p);
    let guide = mcp.call("guide", &json!({})).text();
    assert_eq!(guide.trim(), mcp.resource("qoob3d://guide").trim(), "tool and resource agree");
    let lines = guide.trim().lines().count();
    assert!((40..=120).contains(&lines), "the guide has {lines} lines (at most 120)");
    let lower = guide.to_lowercase();
    // Dimension policy: fixed dimensions are `measured` or `one_source`,
    // free design dimensions are `assumption` and go into the review note,
    // a missing interface dimension or a contradiction is a question.
    for word in ["measured", "one_source", "assumption", "blocked", "interface", "review", "note"] {
        assert!(lower.contains(word), "the dimension policy names {word:?}");
    }
    assert!(lower.contains("contradict"), "contradictory text is a question via blocked");
    // TOML rules for values: ids, the keys, the optional `unit`.
    for word in ["value", "source", "status", "unit", "id"] {
        assert!(lower.contains(word), "the TOML rules name {word:?}");
    }
    assert!(lower.contains("optional"), "`unit` is optional");
    // FDM section: print orientation, fillet on top and chamfer on the bed
    // face, the 45 degree overhang rule, holes.
    for word in ["fdm", "orientation", "fillet", "chamfer", "bed", "overhang", "45", "hole"] {
        assert!(lower.contains(word), "the FDM section names {word:?}");
    }
    // Still a workflow guide (AG-MC-06): the loop by its tools.
    for tool in ["task_list", "task_update", "api_reference", "file_write", "build", "check"] {
        assert!(guide.contains(tool), "the guide names {tool}");
    }
}

/// The table row of `view`: a line `| view | … |` (the name may be in
/// backticks).
fn view_row<'a>(guide: &'a str, view: &str) -> Option<&'a str> {
    guide.lines().find(|line| {
        let mut cells = line.trim().strip_prefix('|').unwrap_or_default().split('|');
        cells.next().is_some_and(|cell| cell.trim().trim_matches('`') == view)
    })
}

/// `text` names the signed axis, with ASCII minus or the minus sign U+2212.
fn names_direction(text: &str, sign: char, axis: char) -> bool {
    let signs: &[char] = if sign == '-' { &['-', '\u{2212}'] } else { &['+'] };
    signs.iter().any(|s| text.contains(&format!("{s}{axis}")))
}

#[test]
fn ag_mc_07_the_guide_has_a_table_of_the_seven_render_views() {
    let p = fresh_project("guide-views");
    let mut mcp = Mcp::open(&p);
    let guide = mcp.call("guide", &json!({})).text();
    for view in VIEWS {
        assert!(view_row(&guide, view).is_some(), "no table row `| {view} | … |` in the guide");
    }
    // Direction the camera looks along (ADR-0006: front looks in +Y, right
    // in −X; +Z is up, so top looks down −Z) and the axis that is up.
    let looks = [
        ("front", '+', 'Y'),
        ("back", '-', 'Y'),
        ("right", '-', 'X'),
        ("left", '+', 'X'),
        ("top", '-', 'Z'),
        ("bottom", '+', 'Z'),
    ];
    for (view, sign, axis) in looks {
        let row = view_row(&guide, view).unwrap_or_default();
        assert!(names_direction(row, sign, axis), "{view} looks along {sign}{axis}: {row}");
    }
    // The side views and iso have +Z up (ADR-0006); top and bottom name
    // another axis as up (it cannot be the one they look along).
    for view in ["iso", "front", "back", "left", "right"] {
        let row = view_row(&guide, view).unwrap_or_default();
        assert!(names_direction(row, '+', 'Z') || row.contains("Z up"), "{view} has Z up: {row}");
    }
    for view in ["top", "bottom"] {
        let row = view_row(&guide, view).unwrap_or_default();
        assert!(row.contains('X') || row.contains('Y'), "{view} names its up axis: {row}");
    }
}

/// Names of the `[id]` tables of a values file.
fn toml_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix('[')?.strip_suffix(']'))
        .map(|id| id.trim().trim_matches('"').to_owned())
        .collect()
}

#[test]
fn ag_mc_07_the_minimal_project_of_the_guide_builds_and_passes_as_written() {
    let p = fresh_project("guide-minimal");
    git_init(&p.dir);
    let mut mcp = Mcp::open(&p);
    let files = guide_files(&mcp.call("guide", &json!({})).text());
    let paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    for folder in ["values/", "parts/", "checks/"] {
        let n = paths.iter().filter(|path| path.starts_with(folder)).count();
        assert_eq!(n, 1, "one complete minimal project: one file under {folder}, got {paths:?}");
    }
    // Exactly as an agent would: write the three files, build, check.
    for (path, content) in &files {
        mcp.call("file_write", &json!({"path": path, "content": content})).ok();
        assert_eq!(&p.read(path), content, "{path} is on disk as written");
    }
    let built = mcp.call("build", &json!({"wait_s": 120}));
    let parts = built.ok()["parts"].as_object().cloned().unwrap_or_default();
    assert!(!parts.is_empty(), "the part file declares a part: {}", built.envelope);
    for (name, part) in &parts {
        assert_eq!(part["state"], json!("built"), "part {name}: {part}");
        assert_eq!(part["valid"], json!(true), "part {name}: {part}");
    }
    let checked = mcp.call("check", &json!({"wait_s": 120}));
    let r = checked.ok();
    assert!(r["passed"].as_u64().is_some_and(|n| n >= 1), "a check ran: {r}");
    assert_eq!(r["failed"].as_u64(), Some(0), "the guide's check is green: {r}");
    let expectations = r["checks"][0]["expectations"].as_array().map_or(0, Vec::len);
    assert!(expectations >= 1, "the check measures something: {r}");

    // The values of the guide follow its own TOML rules: the tool lists them.
    let ids = toml_ids(&files.iter().find(|(path, _)| path.starts_with("values/")).expect("v").1);
    assert!(!ids.is_empty(), "the values file declares a value");
    let values = mcp.call("values", &json!({}));
    let listed = values.ok().to_string();
    for id in &ids {
        assert!(listed.contains(&format!("\"{id}\"")), "values lists {id}: {listed}");
    }

    // All green: the state can be saved, with exactly these files.
    let saved = mcp.call("save_state", &json!({"message": "Minimal project of the guide"}));
    let commit = saved.ok()["commit"].as_str().unwrap_or_default().to_owned();
    let head = git(&p.dir, &["rev-parse", "HEAD"]).expect("a commit exists");
    assert!(commit.len() >= 7 && head.starts_with(&commit), "commit {commit:?} is HEAD {head}");
    let tracked = git_tracked(&p.dir);
    for path in &paths {
        assert!(tracked.iter().any(|t| t == path), "{path} is committed: {tracked:?}");
    }
    // The CLI sees the same project: everything current and passed.
    let status = cli(&p.dir, &["status"], CLI).envelope();
    let states = |key: &str| -> Vec<String> {
        let list = status["result"][key].as_array().cloned().unwrap_or_default();
        list.iter().map(|e| e["state"].as_str().unwrap_or_default().to_owned()).collect()
    };
    assert!(states("parts").iter().all(|s| s == "current"), "{status}");
    assert!(states("checks").iter().all(|s| s == "passed"), "{status}");
    assert_eq!(states("parts").len(), parts.len(), "{status}");
}

// ===========================================================================
// AG-MC-08: the agent edition of the reference
// ===========================================================================

/// What an agent cannot use: repository paths, requirement IDs, ADR
/// numbers, Rust notes.
const NOT_FOR_AGENTS: [&str; 6] = ["crates/", "docs/", "ADR-", "AG-SC", "From Rust", ".rs"];

#[test]
fn ag_mc_08_the_served_reference_has_no_repository_internals() {
    let p = fresh_project("reference-edition");
    let mut mcp = Mcp::open(&p);
    let reference = mcp.call("api_reference", &json!({})).text();
    // It is the reference: the entries an agent searches for are there.
    for entry in ["### `box(x, y, z) -> Shape`", "### `fillet(s, edges, r) -> Shape`", "```rhai"] {
        assert!(reference.contains(entry), "the reference has {entry:?}");
    }
    for (i, line) in reference.lines().enumerate() {
        for word in NOT_FOR_AGENTS {
            assert!(!line.contains(word), "line {}: {word:?} in the reference: {line}", i + 1);
        }
        // No other requirement ID either (AG-PF-02, SK-PL-01, …).
        let id = line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).find(|t| {
            let mut parts = t.split('-');
            let area = |s: &str| s.len() == 2 && s.chars().all(|c| c.is_ascii_uppercase());
            let number = |s: &str| s.len() == 2 && s.chars().all(|c| c.is_ascii_digit());
            let (a, b, c) = (parts.next(), parts.next(), parts.next());
            parts.next().is_none()
                && a.is_some_and(area)
                && b.is_some_and(area)
                && c.is_some_and(number)
        });
        assert!(id.is_none(), "line {}: requirement ID {id:?} in the reference: {line}", i + 1);
    }
    // The resource is the same edition; a search serves parts of it.
    assert_eq!(mcp.resource("qoob3d://script-api").trim(), reference.trim());
    let searched = mcp.call("api_reference", &json!({"search": "project"})).text();
    for word in NOT_FOR_AGENTS {
        assert!(!searched.contains(word), "{word:?} in the searched reference");
    }
}

#[test]
fn ag_mc_08_every_example_of_the_served_reference_runs_in_a_fresh_project() {
    let p = fresh_project("reference-examples");
    let mut mcp = Mcp::open(&p);
    let reference = mcp.call("api_reference", &json!({})).text();
    let examples: Vec<Block> =
        fenced_blocks(&reference).into_iter().filter(|b| b.lang == "rhai").collect();
    // The reference of R1–R4 has more than 40 examples; R5 adds some.
    assert!(examples.len() >= 40, "only {} rhai examples in the reference", examples.len());
    let mut failures = Vec::new();
    for example in &examples {
        // As written: nothing is prepared in the project but `qoob3d init`.
        let run = mcp.call("run_script", &json!({"source": example.body, "wait_s": 120}));
        if run.envelope["ok"] != json!(true) {
            let first = example.body.lines().next().unwrap_or_default();
            failures.push(format!("line {} ({first}): {}", example.line, run.envelope["error"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} examples fail in a fresh project:\n{}",
        failures.len(),
        examples.len(),
        failures.join("\n")
    );
    // An example may import only what `init` created (lib/examples/).
    for example in &examples {
        for line in example.body.lines().filter(|l| l.trim_start().starts_with("import ")) {
            let module = line.split('"').nth(1).unwrap_or_default();
            assert!(
                p.path(&format!("{module}.rhai")).is_file(),
                "line {}: {line} imports a file `qoob3d init` did not create",
                example.line
            );
        }
    }
}

// ===========================================================================
// AG-MC-09: contact sheet
// ===========================================================================

/// A = box(20, 10, 10): x ∈ [−10, 10], y ∈ [−5, 5], z ∈ [0, 10], 2000 mm³.
const PART_A: &str = "part(\"A\", #{}, |deps| box(20, 10, 10));\n";
/// B = A moved by 30 in x: x ∈ [20, 40].
const PART_B: &str = "part(\"B\", #{ depends: [\"A\"] }, |deps| deps[\"A\"].move(30, 0, 0));\n";
const CHECK_A: &str = "\
check(\"a_volume\", || {
    expect_near(part(\"A\").volume(), 2000, 0.001, \"volume of A\");
});
";
const TASK: &str = "\
+++
title = \"First\"
kind = \"task\"
state = \"open\"
parts = []
checks = []
created = \"2026-11-22\"
+++

The words of the human.
";

/// A, B, the check `a_volume` and the open task T-0001.
fn small_project(name: &str) -> Project {
    let p = Project::new(name, "");
    p.write("parts/a.rhai", PART_A)
        .write("parts/b.rhai", PART_B)
        .write("checks/geometry.rhai", CHECK_A)
        .write("tasks/0001-first.md", TASK);
    p
}

/// A successful `render`: the result and the PNG, which is the only image
/// content and the file `result.path` under `out/renders/` (AG-MC-04).
fn rendered(mcp: &mut Mcp, p: &Project, arguments: &Value) -> (Value, Vec<u8>) {
    let tool = mcp.call("render", arguments);
    let result = tool.ok().clone();
    assert_eq!(tool.content.len(), 2, "render {arguments}: the envelope and ONE image");
    let image = &tool.content[1];
    assert_eq!(image["type"], json!("image"), "{}", short(&image.to_string()));
    assert_eq!(image["mimeType"], json!("image/png"));
    let png = base64_decode(image["data"].as_str().expect("image data"));
    let path = result["path"].as_str().unwrap_or_else(|| panic!("result.path: {result}"));
    assert!(path.contains("out/renders/") && path.ends_with(".png"), "{path}");
    assert!(std::fs::read(p.dir.join(path)).ok().as_deref() == Some(&png[..]), "{path}");
    assert!(png_colors(&png) > 1, "render {arguments}: the image is one flat colour");
    (result, png)
}

/// The entry of `view` in `result.views`: an object that names the view and
/// carries its row of the view table (looking direction, up axis).
fn view_entry<'a>(result: &'a Value, index: usize, view: &str) -> &'a Value {
    let entry = &result["views"][index];
    assert_eq!(entry["view"], json!(view), "views[{index}] of {result}");
    let other = entry.as_object().map_or(0, |o| o.iter().filter(|(_, v)| !v.is_null()).count());
    assert!(other >= 3, "views[{index}] has the view, where it looks and what is up: {entry}");
    entry
}

#[test]
fn ag_mc_09_render_with_views_returns_one_contact_sheet() {
    assert_eq!(base64_decode("Zm9vYmE="), b"fooba", "RFC 4648 §10 test vector");
    let p = small_project("sheet");
    let _service = Service::start(&p);
    let mut mcp = Mcp::open(&p);
    let (single, one) =
        rendered(&mut mcp, &p, &json!({"view": "iso", "width": 320, "height": 240}));
    assert_eq!(png_size(&one), (320, 240));

    let three = json!({"views": ["iso", "top", "right"], "width": 320, "height": 240});
    let (result, sheet) = rendered(&mut mcp, &p, &three);
    // One image that holds three views of 320 x 240 (width and height are
    // the size of one view): no side is smaller than one view, and the area
    // is at least three of them, however the sheet is laid out.
    let (w, h) = png_size(&sheet);
    assert!(w >= 320 && h >= 240, "the sheet is {w} x {h}");
    assert!(u64::from(w) * u64::from(h) >= 3 * 320 * 240, "three views do not fit {w} x {h}");
    assert_ne!(result["path"], single["path"], "the sheet does not overwrite the single view");

    // bbox of the rendered selection, here all parts: A x ∈ [−10, 10] and
    // B x ∈ [20, 40], both y ∈ [−5, 5], z ∈ [0, 10].
    assert_vec(&result["bbox"]["min"], [-10.0, -5.0, 0.0], POS, "bbox.min");
    assert_vec(&result["bbox"]["max"], [40.0, 5.0, 10.0], POS, "bbox.max");
    assert_vec(&result["bbox"]["size"], [50.0, 10.0, 10.0], POS, "bbox.size");
    // One entry per view, in the order asked for, with its table row
    // (ADR-0006: top looks along −Z, right along −X).
    assert_eq!(result["views"].as_array().map(Vec::len), Some(3), "{result}");
    view_entry(&result, 0, "iso");
    let top = view_entry(&result, 1, "top").to_string();
    assert!(names_direction(&top, '-', 'Z'), "top looks along -Z: {top}");
    let right = view_entry(&result, 2, "right").to_string();
    assert!(names_direction(&right, '-', 'X'), "right looks along -X: {right}");
    assert!(right.contains('Z'), "right has Z up: {right}");
}

#[test]
fn ag_mc_09_the_contact_sheet_follows_the_selection_and_the_views() {
    let p = small_project("sheet-selection");
    let mut mcp = Mcp::open(&p);
    let small = |views: &[&str], parts: Value| json!({"views": views, "parts": parts, "width": 160, "height": 120});
    // Only A: x ∈ [−10, 10], y ∈ [−5, 5], z ∈ [0, 10].
    let (a, sheet_a) = rendered(&mut mcp, &p, &small(&["front", "top"], json!(["A"])));
    assert_vec(&a["bbox"]["min"], [-10.0, -5.0, 0.0], POS, "bbox.min of A");
    assert_vec(&a["bbox"]["max"], [10.0, 5.0, 10.0], POS, "bbox.max of A");
    assert_eq!(a["views"].as_array().map(Vec::len), Some(2), "two views are the minimum: {a}");
    view_entry(&a, 0, "front");
    view_entry(&a, 1, "top");
    // The build that the render started is the usual one: A is 2000 mm³.
    let built = mcp.call("build", &json!({"parts": ["A"], "wait_s": 60}));
    assert_rel(&built.ok()["parts"]["A"]["volume"], 2000.0, VOL, "volume of A");

    // Other views of the same selection are another image.
    let (_, other) = rendered(&mut mcp, &p, &small(&["back", "bottom"], json!(["A"])));
    assert!(sheet_a != other, "front/top and back/bottom look different");
    // Six views are the maximum; the sheet grows with the number of views.
    let six = ["front", "back", "left", "right", "top", "bottom"];
    let (all, sheet_six) = rendered(&mut mcp, &p, &small(&six, json!(["A"])));
    assert_eq!(all["views"].as_array().map(Vec::len), Some(6), "{all}");
    for (i, view) in six.iter().enumerate() {
        view_entry(&all, i, view);
    }
    let area = |png: &[u8]| u64::from(png_size(png).0) * u64::from(png_size(png).1);
    assert!(area(&sheet_six) >= 6 * 160 * 120, "six views of 160 x 120");
    assert!(area(&sheet_six) > area(&sheet_a), "six views need more room than two");
}

#[test]
fn ag_mc_09_render_refuses_fewer_than_two_and_more_than_six_views() {
    let p = small_project("sheet-errors");
    let mut mcp = Mcp::open(&p);
    mcp.call("render", &json!({"views": ["iso"]})).refused(&[], "one view in views");
    mcp.call("render", &json!({"views": []})).refused(&[], "no view in views");
    mcp.call("render", &json!({"views": VIEWS})).refused(&[], "seven views");
    mcp.call("render", &json!({"views": ["iso", "sideways"]}))
        .refused(&["sideways"], "an unknown view in views");
    mcp.call("render", &json!({"views": "iso"})).refused(&[], "views that is no array");
    mcp.call("render", &json!({"views": ["iso", 7]})).refused(&[], "a view that is no name");
    mcp.call("render", &json!({"views": ["iso", "top"], "parts": ["Nope"]}))
        .refused(&["Nope"], "a contact sheet of an unknown part");
    // The schema tells the agent about it.
    let listed = mcp.request("tools/list", &json!({}), QUICK);
    let tools = listed["result"]["tools"].as_array().cloned().unwrap_or_default();
    let render = tools.iter().find(|t| t["name"] == json!("render")).expect("tool render");
    let views = &render["inputSchema"]["properties"]["views"];
    assert_eq!(views["type"], json!("array"), "render.views in the schema: {render}");
    assert!(views["description"].as_str().is_some_and(|d| !d.is_empty()), "{views}");
}

// ===========================================================================
// AG-MC-10: task_get with the project state
// ===========================================================================

/// `[(name, state)]` of `list` (`[{name, state, …}]`), sorted by name.
fn states(list: &Value) -> Vec<(String, String)> {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let entries = list.as_array().unwrap_or_else(|| panic!("no array of names and states: {list}"));
    let mut out: Vec<_> = entries.iter().map(|e| (text(&e["name"]), text(&e["state"]))).collect();
    assert!(out.iter().all(|(n, s)| !n.is_empty() && !s.is_empty()), "name and state: {list}");
    out.sort();
    out
}

#[test]
fn ag_mc_10_task_get_returns_the_parts_and_checks_with_their_states() {
    let p = small_project("task-project");
    // A running service, so the first task_get can wait for discovery
    // (ADR-0055); the MCP server uses it (AG-MC-01).
    let s = Service::start(&p);
    s.wait_discovery();
    let mut mcp = Mcp::open(&p);
    // Nothing built, nothing run.
    let got = mcp.call("task_get", &json!({"id": "T-0001"}));
    let task = got.ok();
    // Besides the task …
    assert_eq!(task["id"], json!("T-0001"), "{task}");
    assert_eq!(task["title"], json!("First"), "{task}");
    assert_eq!(task["state"], json!("open"), "{task}");
    assert!(task["body"].as_str().is_some_and(|b| b.contains("The words of the human.")));
    // … the project as `project_status` reports it.
    let status = mcp.call("project_status", &json!({})).ok().clone();
    let parts = states(&task["project"]["parts"]);
    let names: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["A", "B"], "{task}");
    assert_eq!(parts, states(&status["parts"]), "part states as in project_status");
    assert!(parts.iter().all(|(_, s)| s != "current"), "nothing is built yet: {parts:?}");
    let never = [("a_volume".to_owned(), "never_run".to_owned())];
    assert_eq!(states(&task["project"]["checks"]), never, "{task}");

    // After a check run (which builds first): current and passed.
    let checked = mcp.call("check", &json!({"wait_s": 120}));
    assert_eq!(checked.ok()["failed"].as_u64(), Some(0), "{}", checked.envelope);
    let after = mcp.call("task_get", &json!({"id": "T-0001"})).ok().clone();
    let current = |n: &str| (n.to_owned(), "current".to_owned());
    assert_eq!(states(&after["project"]["parts"]), [current("A"), current("B")], "{after}");
    let passed = [("a_volume".to_owned(), "passed".to_owned())];
    assert_eq!(states(&after["project"]["checks"]), passed, "{after}");
    // The same states as the service reports over HTTP.
    let http = Service::attach(&p).request("GET", "/status", None).ok();
    assert_eq!(states(&after["project"]["parts"]), states(&http["parts"]));
    assert_eq!(states(&after["project"]["checks"]), states(&http["checks"]));

    // A changed part file shows up without a status call: A and its
    // dependent B are no longer current, the check that read A is stale.
    p.write("parts/a.rhai", "part(\"A\", #{}, |deps| box(20, 10, 12));\n");
    // Discovery of the changed file runs in a worker (ADR-0055).
    s.wait_discovery();
    let changed = mcp.call("task_get", &json!({"id": "T-0001"})).ok().clone();
    let parts = states(&changed["project"]["parts"]);
    assert!(parts.iter().all(|(_, s)| s != "current"), "A changed, B depends on it: {parts:?}");

    // An unknown task has no project attached to an error.
    mcp.call("task_get", &json!({"id": "T-0099"})).refused(&["T-0099"], "an unknown task");
}
