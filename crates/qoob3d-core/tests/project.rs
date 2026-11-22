//! Project folder, values, build graph and cache keys (R2, pure layer):
//! docs/requirements/agent.md §2 (AG-PJ-01..04) and §2.1, ADR-0053. Module
//! `qoob3d_core::project` must work without OCCT, so everything here runs on
//! plain files in a temporary folder.
//!
//! Part *declarations* need script evaluation (stub `part()`, §2.1); the
//! graph is therefore tested from `PartDecl` values, and declaration
//! discovery end to end in `crates/qoob3d-app/tests/service.rs`.
//!
//! The cache key framing (separators between the hashed inputs) is not
//! specified byte for byte, so no known-answer hash is asserted. Instead the
//! tests check the defining property of AG-PJ-04: the key changes exactly
//! when one of the inputs of §2.1 changes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use qoob3d_core::project::{
    Config, Graph, KeyInputs, PartDecl, ValueStatus, Values, cache_key, part_files, resolve_imports,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique temporary folder `<root>/` with the project in `<root>/project/`
/// (room for files "outside the project" beside it), removed on drop.
struct Dir {
    root: PathBuf,
    project: PathBuf,
}

impl Dir {
    fn new(tag: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("qoob3d-core-project-{tag}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let project = root.join("project");
        std::fs::create_dir_all(&project).expect("create temp project");
        Self { root, project }
    }

    /// Writes `text` to `rel` (relative to the project), creating folders.
    fn write(&self, rel: &str, text: &str) -> &Self {
        let path = self.project.join(rel);
        std::fs::create_dir_all(path.parent().expect("file has a parent"))
            .expect("create parent folder");
        std::fs::write(&path, text).expect("write project file");
        self
    }

    fn path(&self) -> &Path {
        &self.project
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect()
}

fn decl(name: &str, file: &str, depends: &[&str]) -> PartDecl {
    PartDecl {
        name: name.to_owned(),
        file: PathBuf::from(file),
        depends: depends.iter().map(|d| (*d).to_owned()).collect(),
    }
}

/// Asserts that `message` mentions every word of `words`.
fn assert_mentions(message: &str, words: &[&str]) {
    for w in words {
        assert!(message.contains(w), "error must name {w:?}: {message}");
    }
}

// ---------------------------------------------------------------------------
// AG-PJ-01: qoob3d.toml
// ---------------------------------------------------------------------------

#[test]
fn ag_pj_01_config_defaults() {
    // AG-PJ-01: `units = "mm"`, `[limits] time_s = 60, memory_mb = 4096`,
    // `[workers] count = 2` are the defaults when the file only names the project.
    let d = Dir::new("defaults");
    d.write("qoob3d.toml", "[project]\nname = \"crane\"\n");
    let c = Config::load(d.path()).expect("minimal qoob3d.toml loads");
    assert_eq!(c.name, "crane");
    assert_eq!(c.units, "mm");
    assert!((c.limits.time_s - 60.0).abs() < f64::EPSILON, "time_s {}", c.limits.time_s);
    assert_eq!(c.limits.memory_mb, 4096);
    assert_eq!(c.workers.count, 2);
}

#[test]
fn ag_pj_01_config_values_override_defaults() {
    let d = Dir::new("override");
    d.write(
        "qoob3d.toml",
        "[project]\nname = \"box\"\nunits = \"mm\"\n\n\
         [limits]\ntime_s = 2.5\nmemory_mb = 300\n\n[workers]\ncount = 4\n",
    );
    let c = Config::load(d.path()).expect("qoob3d.toml loads");
    assert_eq!(c.name, "box");
    // 2.5 is exactly representable, so exact comparison is valid.
    assert!((c.limits.time_s - 2.5).abs() < f64::EPSILON);
    assert_eq!(c.limits.memory_mb, 300);
    assert_eq!(c.workers.count, 4);
}

#[test]
fn ag_pj_01_integer_time_limit_is_accepted() {
    // `time_s = 3` (TOML integer) is the natural spelling of a whole number of seconds.
    let d = Dir::new("int-time");
    d.write("qoob3d.toml", "[project]\nname = \"p\"\n[limits]\ntime_s = 3\n");
    let c = Config::load(d.path()).expect("integer time_s loads");
    assert!((c.limits.time_s - 3.0).abs() < f64::EPSILON);
    assert_eq!(c.limits.memory_mb, 4096, "unset memory_mb keeps its default");
}

#[test]
fn ag_pj_01_missing_config_is_an_error_naming_the_file() {
    let d = Dir::new("no-config");
    let e = Config::load(d.path()).expect_err("a folder without qoob3d.toml is no project");
    assert_mentions(&e.to_string(), &["qoob3d.toml"]);
}

#[test]
fn ag_pj_01_malformed_config_is_an_error_naming_the_file() {
    let d = Dir::new("bad-config");
    d.write("qoob3d.toml", "[project\nname = \n");
    let e = Config::load(d.path()).expect_err("malformed TOML must not load");
    assert_mentions(&e.to_string(), &["qoob3d.toml"]);
}

#[test]
fn ag_pj_01_non_positive_limits_are_errors() {
    // Interpretation: a time limit <= 0 would kill every build and zero
    // workers would never build anything; both are configuration errors.
    for (body, field) in [
        ("[limits]\ntime_s = 0\n", "time_s"),
        ("[limits]\ntime_s = -1.0\n", "time_s"),
        ("[limits]\nmemory_mb = 0\n", "memory_mb"),
        ("[workers]\ncount = 0\n", "count"),
    ] {
        let d = Dir::new("bad-limit");
        d.write("qoob3d.toml", &format!("[project]\nname = \"p\"\n{body}"));
        let e = Config::load(d.path()).expect_err(body);
        assert_mentions(&e.to_string(), &["qoob3d.toml", field]);
    }
}

// ---------------------------------------------------------------------------
// AG-PJ-01: part files
// ---------------------------------------------------------------------------

#[test]
fn ag_pj_01_part_files_are_parts_star_rhai_sorted() {
    // `parts/*.rhai`: not other extensions, not sub folders, not lib modules.
    let d = Dir::new("part-files");
    d.write("qoob3d.toml", "[project]\nname = \"p\"\n")
        .write("parts/b.rhai", "")
        .write("parts/a.rhai", "")
        .write("parts/notes.txt", "")
        .write("parts/sub/c.rhai", "")
        .write("lib/form.rhai", "")
        .write("checks/fit.rhai", "");
    let files = part_files(d.path()).expect("list part files");
    assert_eq!(names(&files), ["parts/a.rhai", "parts/b.rhai"]);
}

// ---------------------------------------------------------------------------
// AG-PJ-03: values
// ---------------------------------------------------------------------------

const CRANE_VALUES: &str = "\
[crane.column]
value = 120.5
source = \"caliper 2026-11-20\"
status = \"measured\"

[crane.boom]
value = 300
source = \"data sheet\"
status = \"one_source\"

[hook]
value = \"M8\"
source = \"guess from the photo\"
status = \"assumption\"
";

#[test]
fn ag_pj_03_values_load_with_value_source_status_and_file() {
    let d = Dir::new("values");
    d.write("values/crane.toml", CRANE_VALUES).write(
        "values/print.toml",
        "[wall]\nvalue = 1.2\nsource = \"printer profile\"\nstatus = \"assumption\"\n",
    );
    let v = Values::load(d.path()).expect("valid values load");

    // `[crane.column]` is the id `crane.column` (the example id of AG-PJ-03).
    let column = v.get("crane.column").expect("crane.column");
    assert_eq!(column.id, "crane.column");
    assert_eq!(column.value, json!(120.5));
    assert_eq!(column.source, "caliper 2026-11-20");
    assert_eq!(column.status, ValueStatus::Measured);
    assert_eq!(names(std::slice::from_ref(&column.file)), ["values/crane.toml"]);

    let boom = v.get("crane.boom").expect("crane.boom");
    assert_eq!(boom.value, json!(300), "TOML integers stay integers");
    assert_eq!(boom.status, ValueStatus::OneSource);

    let hook = v.get("hook").expect("hook");
    assert_eq!(hook.value, json!("M8"));
    assert_eq!(hook.status, ValueStatus::Assumption);

    let wall = v.get("wall").expect("wall from the second file");
    assert_eq!(names(std::slice::from_ref(&wall.file)), ["values/print.toml"]);
    assert!(v.get("crane").is_none(), "a namespace table is not a value");
    assert!(v.get("missing").is_none());
}

#[test]
fn ag_pj_03_assumption_report_lists_exactly_the_assumptions_sorted() {
    let d = Dir::new("assumptions");
    d.write("values/crane.toml", CRANE_VALUES).write(
        "values/print.toml",
        "[wall]\nvalue = 1.2\nsource = \"printer profile\"\nstatus = \"assumption\"\n",
    );
    let v = Values::load(d.path()).expect("valid values load");
    let ids: Vec<&str> = v.assumptions().iter().map(|a| a.id.as_str()).collect();
    // `hook` and `wall` have status assumption; measured/one_source are not listed.
    assert_eq!(ids, ["hook", "wall"]);
}

#[test]
fn ag_pj_03_no_values_folder_means_no_values() {
    let d = Dir::new("no-values");
    d.write("qoob3d.toml", "[project]\nname = \"p\"\n");
    let v = Values::load(d.path()).expect("a project without values/ is valid");
    assert!(v.get("anything").is_none());
    assert!(v.assumptions().is_empty());
}

/// Loads `values/bad.toml` with `body` and returns the error text.
fn values_error(body: &str) -> String {
    let d = Dir::new("bad-values");
    d.write("values/good.toml", "[ok]\nvalue = 1\nsource = \"s\"\nstatus = \"measured\"\n")
        .write("values/bad.toml", body);
    Values::load(d.path()).expect_err(body).to_string()
}

#[test]
fn ag_pj_03_missing_source_names_file_and_id() {
    let e = values_error("[a.width]\nvalue = 3\nstatus = \"measured\"\n");
    assert_mentions(&e, &["bad.toml", "a.width", "source"]);
}

#[test]
fn ag_pj_03_missing_status_names_file_and_id() {
    let e = values_error("[a.width]\nvalue = 3\nsource = \"drawing\"\n");
    assert_mentions(&e, &["bad.toml", "a.width", "status"]);
}

#[test]
fn ag_pj_03_unknown_status_names_file_and_id() {
    // Only measured | one_source | assumption exist.
    let e = values_error("[a.width]\nvalue = 3\nsource = \"drawing\"\nstatus = \"guess\"\n");
    assert_mentions(&e, &["bad.toml", "a.width", "guess"]);
}

#[test]
fn ag_pj_03_missing_value_names_file_and_id() {
    let e = values_error("[a.width]\nsource = \"drawing\"\nstatus = \"measured\"\n");
    assert_mentions(&e, &["bad.toml", "a.width"]);
}

#[test]
fn ag_pj_03_empty_source_counts_as_missing() {
    // A blank `source` gives no source; the rule exists so every value is traceable.
    let e = values_error("[a.width]\nvalue = 3\nsource = \"  \"\nstatus = \"measured\"\n");
    assert_mentions(&e, &["bad.toml", "a.width", "source"]);
}

#[test]
fn ag_pj_03_malformed_toml_names_the_file() {
    let e = values_error("[a.width\nvalue = \n");
    assert_mentions(&e, &["bad.toml"]);
}

#[test]
fn ag_pj_03_duplicate_id_across_files_names_id_and_both_files() {
    let d = Dir::new("dup-values");
    let entry = "[wall]\nvalue = 1.2\nsource = \"s\"\nstatus = \"measured\"\n";
    d.write("values/one.toml", entry).write("values/two.toml", entry);
    let e = Values::load(d.path()).expect_err("an id defined twice is ambiguous").to_string();
    assert_mentions(&e, &["wall", "one.toml", "two.toml"]);
}

// ---------------------------------------------------------------------------
// AG-PJ-04: module imports
// ---------------------------------------------------------------------------

#[test]
fn ag_pj_04_imports_resolve_transitively_each_module_once() {
    // parts/c imports lib/form and lib/util; lib/form imports lib/util again
    // (diamond) and lib/deep/edge. Expected set: form, util, deep/edge, once each.
    let d = Dir::new("imports");
    d.write(
        "parts/c.rhai",
        "import \"lib/form\" as form;\nimport \"lib/util\" as util;\n\
         part(\"C\", #{}, |deps| form::make());\n",
    )
    .write(
        "lib/form.rhai",
        "import \"lib/util\" as u;\nimport \"lib/deep/edge\" as e;\nfn make() { box(4, 4, 4) }\n",
    )
    .write("lib/util.rhai", "export const SIZE = 4;\n")
    .write("lib/deep/edge.rhai", "fn r() { 1 }\n")
    .write("lib/unused.rhai", "fn x() { 2 }\n");
    let mods = resolve_imports(d.path(), Path::new("parts/c.rhai")).expect("imports resolve");
    let mut got = names(&mods);
    let n = got.len();
    got.sort();
    got.dedup();
    assert_eq!(got.len(), n, "every module appears once: {mods:?}");
    assert_eq!(got, ["lib/deep/edge.rhai", "lib/form.rhai", "lib/util.rhai"]);
    // Deterministic: the key hashes the modules in this order.
    let again = resolve_imports(d.path(), Path::new("parts/c.rhai")).expect("again");
    assert_eq!(mods, again);
}

#[test]
fn ag_pj_04_file_without_imports_has_no_modules() {
    let d = Dir::new("no-imports");
    d.write("parts/d.rhai", "part(\"D\", #{}, |deps| cylinder(10, 5));\n");
    let mods = resolve_imports(d.path(), Path::new("parts/d.rhai")).expect("resolve");
    assert!(mods.is_empty(), "{mods:?}");
}

#[test]
fn ag_pj_04_missing_module_names_module_and_importing_file() {
    let d = Dir::new("missing-module");
    d.write("parts/c.rhai", "import \"lib/missing\" as m;\npart(\"C\", #{}, |deps| m::f());\n");
    let e = resolve_imports(d.path(), Path::new("parts/c.rhai"))
        .expect_err("a missing module is a load error")
        .to_string();
    assert_mentions(&e, &["lib/missing", "parts/c.rhai"]);
}

#[test]
fn ag_pj_04_imports_cannot_escape_the_project() {
    // Same sandbox rule as AG-SC-02 / AG-SC-14: nothing outside the project.
    let d = Dir::new("escape");
    // The module exists, so only the sandbox rule can refuse it.
    std::fs::write(d.root.join("outside-module.rhai"), "fn f() { 1 }\n").expect("write");
    let outside = d.root.join("outside-module");
    for target in ["../outside-module".to_owned(), outside.display().to_string()] {
        d.write("parts/c.rhai", &format!("import \"{target}\" as o;\n"));
        let e = resolve_imports(d.path(), Path::new("parts/c.rhai"))
            .expect_err(&format!("import of {target} must be refused"))
            .to_string();
        assert_mentions(&e, &["parts/c.rhai"]);
    }
}

#[test]
fn ag_pj_04_import_cycle_terminates() {
    // lib/a <-> lib/b. Either both are listed once or the cycle is an error
    // naming a module; it must never loop forever.
    let d = Dir::new("import-cycle");
    d.write("parts/p.rhai", "import \"lib/a\" as a;\n")
        .write("lib/a.rhai", "import \"lib/b\" as b;\n")
        .write("lib/b.rhai", "import \"lib/a\" as a;\n");
    match resolve_imports(d.path(), Path::new("parts/p.rhai")) {
        Ok(mods) => {
            let mods: Vec<PathBuf> = mods;
            let mut got = names(&mods);
            got.sort();
            assert_eq!(got, ["lib/a.rhai", "lib/b.rhai"]);
        }
        Err(e) => assert!(e.to_string().contains("lib/"), "{e}"),
    }
}

// ---------------------------------------------------------------------------
// AG-PJ-02 / AG-PJ-04: build graph
// ---------------------------------------------------------------------------

/// Position of `name` in `order`.
fn pos(order: &[&str], name: &str) -> usize {
    order.iter().position(|n| *n == name).unwrap_or_else(|| panic!("{name} not in {order:?}"))
}

#[test]
fn ag_pj_04_order_puts_dependencies_first() {
    // B → A, C → A and B, D independent; declared in reverse on purpose.
    let g = Graph::new(vec![
        decl("D", "parts/d.rhai", &[]),
        decl("C", "parts/c.rhai", &["B", "A"]),
        decl("B", "parts/b.rhai", &["A"]),
        decl("A", "parts/a.rhai", &[]),
    ])
    .expect("acyclic graph");
    let order = g.order();
    assert_eq!(order.len(), 4, "{order:?}");
    assert!(pos(&order, "A") < pos(&order, "B"), "{order:?}");
    assert!(pos(&order, "B") < pos(&order, "C"), "{order:?}");
    assert!(order.contains(&"D"));
}

#[test]
fn ag_pj_04_dependents_are_transitive() {
    let g = Graph::new(vec![
        decl("A", "parts/a.rhai", &[]),
        decl("B", "parts/b.rhai", &["A"]),
        decl("C", "parts/c.rhai", &["B"]),
        decl("D", "parts/d.rhai", &[]),
    ])
    .expect("acyclic graph");
    let mut a = g.dependents("A");
    a.sort_unstable();
    assert_eq!(a, ["B", "C"], "changing A rebuilds B and, through B, C");
    assert_eq!(g.dependents("C"), Vec::<&str>::new());
    assert_eq!(g.dependents("D"), Vec::<&str>::new());
}

#[test]
fn ag_pj_02_unknown_dependency_names_both_parts() {
    let e = Graph::new(vec![decl("A", "parts/a.rhai", &[]), decl("B", "parts/b.rhai", &["Ghost"])])
        .expect_err("unknown dependency is a load error")
        .to_string();
    assert_mentions(&e, &["B", "Ghost"]);
}

#[test]
fn ag_pj_02_cycle_names_the_parts() {
    let e = Graph::new(vec![
        decl("Alpha", "parts/x.rhai", &["Gamma"]),
        decl("Beta", "parts/x.rhai", &["Alpha"]),
        decl("Gamma", "parts/y.rhai", &["Beta"]),
        decl("Free", "parts/y.rhai", &[]),
    ])
    .expect_err("a cycle is a load error")
    .to_string();
    assert_mentions(&e, &["Alpha", "Beta", "Gamma"]);
}

#[test]
fn ag_pj_02_self_dependency_is_a_cycle() {
    let e = Graph::new(vec![decl("Loop", "parts/l.rhai", &["Loop"])])
        .expect_err("a part cannot depend on itself")
        .to_string();
    assert_mentions(&e, &["Loop"]);
}

#[test]
fn ag_pj_02_duplicate_part_names_are_errors() {
    // Names are unique across the project, also across files.
    let e = Graph::new(vec![decl("Lid", "parts/a.rhai", &[]), decl("Lid", "parts/b.rhai", &[])])
        .expect_err("duplicate names are a load error")
        .to_string();
    assert_mentions(&e, &["Lid", "parts/a.rhai", "parts/b.rhai"]);
}

#[test]
fn ag_pj_02_empty_project_has_an_empty_graph() {
    let g = Graph::new(Vec::new()).expect("no parts is valid");
    assert!(g.order().is_empty());
}

// ---------------------------------------------------------------------------
// AG-PJ-04 / §2.1: cache key
// ---------------------------------------------------------------------------

fn base() -> KeyInputs {
    KeyInputs {
        program_version: "0.1.0".to_owned(),
        part_file: b"part(\"A\", #{}, |deps| box(value(\"a\"), 10, 10));\n".to_vec(),
        modules: vec![b"fn make() { box(4, 4, 4) }\n".to_vec()],
        values_read: vec![("a".to_owned(), json!(20)), ("b.c".to_owned(), json!(1.5))],
        dependency_keys: vec!["0".repeat(64)],
    }
}

#[test]
fn ag_pj_04_key_is_sha256_hex_and_deterministic() {
    let k = cache_key(&base());
    // SHA-256 → 32 bytes → 64 hex digits.
    assert_eq!(k.len(), 64, "{k}");
    assert!(k.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{k}");
    assert_eq!(k, cache_key(&base()), "same inputs, same key");
}

#[test]
fn ag_pj_04_key_changes_with_every_input() {
    let k = cache_key(&base());
    let mut variants: Vec<(&str, KeyInputs)> = Vec::new();
    let mut i = base();
    i.program_version = "0.1.1".to_owned();
    variants.push(("program version", i));
    let mut i = base();
    i.part_file.push(b' ');
    variants.push(("part file bytes", i));
    let mut i = base();
    i.modules[0] = b"fn make() { box(4, 4, 5) }\n".to_vec();
    variants.push(("module bytes", i));
    let mut i = base();
    i.modules.push(b"export const X = 1;\n".to_vec());
    variants.push(("additional module", i));
    let mut i = base();
    i.values_read[0].1 = json!(21);
    variants.push(("read value", i));
    let mut i = base();
    i.values_read.push(("d".to_owned(), json!("M8")));
    variants.push(("additional read value", i));
    let mut i = base();
    i.values_read[1].0 = "b.d".to_owned();
    variants.push(("read value id", i));
    let mut i = base();
    i.dependency_keys[0] = "1".repeat(64);
    variants.push(("dependency key", i));
    let mut i = base();
    i.dependency_keys.push("2".repeat(64));
    variants.push(("additional dependency", i));
    for (what, inputs) in variants {
        assert_ne!(cache_key(&inputs), k, "changing the {what} must change the key");
    }
}

#[test]
fn ag_pj_04_value_reads_are_a_sorted_list() {
    // §2.1: "the sorted list of (value id, canonical value)": the order in
    // which the script read the values does not matter.
    let mut i = base();
    i.values_read.reverse();
    assert_eq!(cache_key(&i), cache_key(&base()));
}

#[test]
fn ag_pj_04_key_before_the_first_build_has_no_values() {
    // §2.1: before the first build the key is computed without values; a
    // key with recorded reads differs from it.
    let mut first = base();
    first.values_read.clear();
    assert_ne!(cache_key(&first), cache_key(&base()));
}

#[test]
fn ag_pj_04_moving_bytes_between_inputs_changes_the_key() {
    // AG-PJ-04 rebuilds when the part file changes; moving the text "b" from
    // the part file into the module changes both files, so the key must
    // change, which a plain concatenation of the inputs would miss.
    let mut a = base();
    a.part_file = b"ab".to_vec();
    a.modules = vec![b"c".to_vec()];
    let mut b = base();
    b.part_file = b"a".to_vec();
    b.modules = vec![b"bc".to_vec()];
    assert_ne!(cache_key(&a), cache_key(&b));
    // The same for the boundary between two modules.
    let mut c = base();
    c.modules = vec![b"x".to_vec(), b"yz".to_vec()];
    let mut d = base();
    d.modules = vec![b"xy".to_vec(), b"z".to_vec()];
    assert_ne!(cache_key(&c), cache_key(&d));
}

#[test]
fn ag_pj_04_value_types_are_distinguished() {
    // The canonical value of the string "20" is not the number 20.
    let mut s = base();
    s.values_read[0].1 = json!("20");
    assert_ne!(cache_key(&s), cache_key(&base()));
}
