//! The kernel in the cache key (WS-PRE-02): a kernel whose results may
//! change rebuilds every part, a rebuild of the app alone keeps the cache.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use qoob3d_core::project::{Mesh, file_digest};
use serde_json::Value;

use super::{KERNEL_FINGERPRINT, key_version};

/// The files of `qoob3d-core` the fingerprint covers: its manifest and the
/// code the kernel runs while it builds. Written here a second time, so a
/// list in `build.rs` that differs fails the fingerprint test.
const CORE_FILES: [&str; 8] = [
    "Cargo.toml",
    "src/export.rs",
    "src/font.rs",
    "src/font/regions.rs",
    "src/import.rs",
    "src/import/stl.rs",
    "src/import/three_mf.rs",
    "src/mesh.rs",
];

/// What the kernel takes from `qoob3d_core::project` without it changing a
/// build result: part options of the declarations (kept in the service's
/// memory, never in the cache) and the `[mesh]` settings, whose values
/// enter the key through `key_version`.
const DECLARATION_ONLY: [&str; 4] = ["COLOR_NAMES", "Mesh", "PrintPose", "parse_color"];

/// The path packages cargo builds the kernel with, and their folders.
const PATH_PACKAGES: [(&str, &str); 3] = [
    ("qoob3d-kernel", "crates/qoob3d-kernel"),
    ("qoob3d-core", "crates/qoob3d-core"),
    ("opencascade-sys", "third_party/opencascade-sys"),
];

/// The workspace root.
fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Every file under `rel` (relative to the workspace root, with `/`).
fn walk(rel: &str, out: &mut Vec<String>) {
    let path = root().join(rel);
    if path.is_dir() {
        for entry in std::fs::read_dir(&path).expect("read a source folder") {
            let name = entry.expect("entry").file_name().to_string_lossy().into_owned();
            walk(&format!("{rel}/{name}"), out);
        }
    } else {
        out.push(rel.to_owned());
    }
}

/// `name version source` of every package the kernel depends on, normal or
/// build dependency, transitively (the dev dependencies of a workspace
/// member are no input of the kernel), as cargo itself resolves them for
/// building.
fn resolved_kernel_dependencies() -> BTreeSet<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline", "--locked"])
        .current_dir(root())
        .output()
        .expect("run cargo metadata");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: Value = serde_json::from_slice(&out.stdout).expect("cargo metadata JSON");
    let packages: HashMap<&str, &Value> = meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|p| (p["id"].as_str().expect("id"), p))
        .collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| (n["id"].as_str().expect("id"), n))
        .collect();
    let kernel = packages.values().find(|p| p["name"] == "qoob3d-kernel").expect("the kernel");
    let mut todo = vec![kernel["id"].as_str().expect("id")];
    let mut seen = BTreeSet::new();
    let mut lines = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let p = packages[id];
        if id != kernel["id"] {
            lines.insert(format!(
                "{} {} {}",
                p["name"].as_str().expect("name"),
                p["version"].as_str().expect("version"),
                p["source"].as_str().unwrap_or_default()
            ));
        }
        for dep in nodes[id]["deps"].as_array().expect("deps") {
            let kinds = dep["dep_kinds"].as_array().expect("dep_kinds");
            if kinds.iter().any(|k| k["kind"].is_null() || k["kind"] == "build") {
                todo.push(dep["pkg"].as_str().expect("pkg"));
            }
        }
    }
    lines
}

/// The names of the normal and build dependencies in the `Cargo.toml` of
/// `folder` (`[dependencies]`, `[build-dependencies]` and their
/// `[target.….…]` forms), read line by line.
fn manifest_dependencies(folder: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(root().join(folder).join("Cargo.toml")).expect("manifest");
    let mut names = BTreeSet::new();
    let mut listing = false;
    for line in text.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            let kind = header.trim_end_matches(']').rsplit('.').next().unwrap_or_default();
            listing = kind == "dependencies" || kind == "build-dependencies";
        } else if listing && !line.is_empty() && !line.starts_with('#') {
            let name = line.split(['=', '.', ' ']).next().unwrap_or_default();
            names.insert(name.to_owned());
        }
    }
    names
}

/// `name version source` of the packages `build.rs` fingerprints: from
/// `qoob3d-kernel` along the `dependencies` of `Cargo.lock`; of a path
/// package (no source) only those its `Cargo.toml` lists as normal or build
/// dependencies, since `Cargo.lock` lists its dev dependencies too. Read line
/// by line here, so a slip in the `toml` walk of `build.rs` fails.
fn locked_kernel_dependencies() -> BTreeSet<String> {
    struct Package {
        name: String,
        version: String,
        source: String,
        deps: Vec<String>,
    }
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).expect("read Cargo.lock");
    let quoted = |line: &str| line.split('"').nth(1).unwrap_or_default().to_owned();
    let mut packages = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut p = Package {
            name: String::new(),
            version: String::new(),
            source: String::new(),
            deps: Vec::new(),
        };
        let mut in_deps = false;
        for line in block.lines() {
            if in_deps {
                if line.starts_with(']') {
                    in_deps = false;
                } else {
                    p.deps.push(quoted(line));
                }
            } else if line.starts_with("name = ") {
                p.name = quoted(line);
            } else if line.starts_with("version = ") {
                p.version = quoted(line);
            } else if line.starts_with("source = ") {
                p.source = quoted(line);
            } else if line.starts_with("dependencies = [") {
                in_deps = true;
            }
        }
        packages.push(p);
    }
    let index = |dep: &str| {
        let (name, version) = dep
            .split_once(' ')
            .map_or((dep, None), |(n, v)| (n, Some(v.split(' ').next().unwrap_or(v))));
        let hits: Vec<usize> = (0..packages.len())
            .filter(|i| {
                packages[*i].name == name && version.is_none_or(|v| packages[*i].version == v)
            })
            .collect();
        assert_eq!(hits.len(), 1, "{dep}");
        hits[0]
    };
    let kernel = index("qoob3d-kernel");
    let (mut todo, mut seen, mut lines) = (vec![kernel], BTreeSet::new(), BTreeSet::new());
    while let Some(i) = todo.pop() {
        if !seen.insert(i) {
            continue;
        }
        let p = &packages[i];
        if i != kernel {
            lines.insert(format!("{} {} {}", p.name, p.version, p.source));
        }
        let wanted = p.source.is_empty().then(|| {
            let (_, folder) = PATH_PACKAGES
                .iter()
                .find(|(name, _)| *name == p.name)
                .unwrap_or_else(|| panic!("path package {} without a folder here", p.name));
            manifest_dependencies(folder)
        });
        for dep in &p.deps {
            let name = dep.split(' ').next().unwrap_or_default();
            if wanted.as_ref().is_none_or(|w| w.contains(name)) {
                todo.push(index(dep));
            }
        }
    }
    lines
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The items a path at the start of `s` names, below `module`: `a::b::f`
/// gives `([a, b], f)`, `a::{B, c::D}` gives `([a], B)` and `([a, c], D)`.
/// A capitalised segment is an item (`Mesh::default` names `Mesh`).
fn paths(s: &str, module: &[String], out: &mut Vec<(Vec<String>, String)>) {
    let s = s.trim_start();
    if let Some(group) = s.strip_prefix('{') {
        let (mut depth, mut start) = (0, 0);
        for (i, c) in group.char_indices() {
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => {
                    paths(&group[start..i], module, out);
                    return;
                }
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    paths(&group[start..i], module, out);
                    start = i + 1;
                }
                _ => {}
            }
        }
        return;
    }
    let ident = &s[..s.find(|c| !is_ident(c)).unwrap_or(s.len())];
    let capital = ident.starts_with(|c: char| c.is_ascii_uppercase());
    match s[ident.len()..].strip_prefix("::") {
        Some(rest) if !capital => {
            let mut deeper = module.to_vec();
            deeper.push(ident.to_owned());
            paths(rest, &deeper, out);
        }
        _ if !ident.is_empty() && ident != "self" => out.push((module.to_vec(), ident.to_owned())),
        _ => {}
    }
}

/// Every item `text` names through `prefix` (`qoob3d_core::`, `crate::`,
/// `super::`), outside comments, below `module`.
fn references(text: &str, prefix: &str, module: &[String]) -> Vec<(Vec<String>, String)> {
    let code: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect();
    let code = code.join("\n");
    let mut out = Vec::new();
    for (at, _) in code.match_indices(prefix) {
        if !code[..at].ends_with(is_ident) {
            paths(&code[at + prefix.len()..], module, &mut out);
        }
    }
    out
}

/// The module of a source file of a crate: `src/import/stl.rs` is
/// `import::stl`, `src/project/mod.rs` is `project`, `src/lib.rs` the root.
fn module_of(rel: &str) -> Vec<String> {
    let path = rel.trim_start_matches("src/").trim_end_matches(".rs");
    let mut parts: Vec<String> = path.split('/').map(str::to_owned).collect();
    if matches!(parts.last().map(String::as_str), Some("lib" | "mod")) {
        parts.pop();
    }
    parts
}

/// The source files under `crate_dir/src` without unit tests, relative to
/// `crate_dir`, with their texts.
fn sources(crate_dir: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk(&format!("{crate_dir}/src"), &mut files);
    files
        .into_iter()
        .filter(|f| f.ends_with(".rs") && !f.ends_with("/tests.rs") && !f.contains("/tests/"))
        .map(|f| {
            let text = std::fs::read_to_string(root().join(&f)).expect("read a source");
            (f[crate_dir.len() + 1..].to_owned(), text)
        })
        .collect()
}

/// Whether `text` defines `name` as a function, type, constant or module.
fn defines(text: &str, name: &str) -> bool {
    const KINDS: [&str; 8] = ["fn", "struct", "enum", "const", "static", "type", "trait", "mod"];
    text.lines().filter(|l| !l.trim_start().starts_with("//")).any(|line| {
        let words: Vec<&str> = line.split(|c| !is_ident(c)).filter(|w| !w.is_empty()).collect();
        words.windows(2).any(|w| KINDS.contains(&w[0]) && w[1] == name)
    })
}

/// The files of `qoob3d-core` whose code the kernel runs: those defining
/// what the kernel names (but [`DECLARATION_ONLY`]), then what they name
/// through `crate::` and `super::` and the child modules they use.
fn core_files_the_kernel_runs() -> BTreeSet<String> {
    let core = sources("crates/qoob3d-core");
    let mut todo: Vec<(Vec<String>, String)> = sources("crates/qoob3d-kernel")
        .iter()
        .flat_map(|(_, text)| references(text, "qoob3d_core::", &[]))
        .filter(|(module, name)| !(module == &["project"] && DECLARATION_ONLY.contains(&&**name)))
        .collect();
    let mut needed = BTreeSet::new();
    let mut added = Vec::new();
    loop {
        for (module, name) in todo.drain(..) {
            let defining: Vec<&String> = core
                .iter()
                .filter(|(rel, text)| module_of(rel).starts_with(&module) && defines(text, &name))
                .map(|(rel, _)| rel)
                .collect();
            assert!(!defining.is_empty(), "no file of qoob3d-core defines {module:?} {name}");
            added.extend(defining.into_iter().filter(|rel| needed.insert((*rel).clone())).cloned());
        }
        let Some(rel) = added.pop() else { break };
        let text = &core.iter().find(|(r, _)| *r == rel).expect("a source").1;
        let module = module_of(&rel);
        todo.extend(references(text, "crate::", &[]));
        todo.extend(references(text, "super::", &module[..module.len().saturating_sub(1)]));
        // Child modules (`mod regions;`) the file names (`regions::…`).
        let mut test_only = false;
        for line in text.lines().map(str::trim) {
            let child = line.strip_prefix("pub ").unwrap_or(line).strip_prefix("mod ");
            if let Some(child) = child.and_then(|c| c.strip_suffix(';'))
                && !test_only
                && text.contains(&format!("{child}::"))
            {
                let mut path = module.clone();
                path.push(child.to_owned());
                let file = core.iter().find(|(r, _)| module_of(r) == path).expect("child module");
                if needed.insert(file.0.clone()) {
                    added.push(file.0.clone());
                }
            }
            test_only = line == "#[cfg(test)]";
        }
    }
    needed
}

/// The fingerprint as `build.rs` must compute it: the number of files, then
/// every file of the kernel (`Cargo.toml`, `build.rs`, `cpp/`, `src/`), of
/// the OCCT fork (`Cargo.toml`, `build.rs`, `include/`, `src/`, `OCCT/`) and
/// the [`CORE_FILES`] of `qoob3d-core`,
/// sorted by its path from the workspace root, as length-prefixed path and
/// bytes; then the number of locked dependencies and each `name version
/// source` line, sorted and length-prefixed. Written here a second time, so
/// a fingerprint that `build.rs` computed from other inputs, or did not
/// compute again after a change, fails.
fn kernel_fingerprint_now() -> String {
    fn field(stream: &mut Vec<u8>, bytes: &[u8]) {
        stream.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        stream.extend_from_slice(bytes);
    }
    let mut files = Vec::new();
    for part in ["Cargo.toml", "build.rs", "cpp", "src"] {
        walk(&format!("crates/qoob3d-kernel/{part}"), &mut files);
    }
    for part in ["Cargo.toml", "build.rs", "include", "src", "OCCT"] {
        walk(&format!("third_party/opencascade-sys/{part}"), &mut files);
    }
    for file in CORE_FILES {
        walk(&format!("crates/qoob3d-core/{file}"), &mut files);
    }
    files.sort();
    let mut stream = Vec::new();
    stream.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for rel in &files {
        field(&mut stream, rel.as_bytes());
        field(&mut stream, &std::fs::read(root().join(rel)).expect("read a kernel source"));
    }
    let locked = locked_kernel_dependencies();
    stream.extend_from_slice(&(locked.len() as u64).to_le_bytes());
    for line in &locked {
        field(&mut stream, line.as_bytes());
    }
    file_digest(&stream).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn ws_pre_02_the_kernel_fingerprint_is_that_of_the_current_kernel_sources() {
    assert_eq!(KERNEL_FINGERPRINT, kernel_fingerprint_now());
}

#[test]
fn ws_pre_02_the_fingerprint_covers_every_package_cargo_builds_the_kernel_with() {
    // `Cargo.lock` lists a few more (cxx pins `cxxbridge-cmd` and its clap
    // tree as a dependency for no platform): harmless, an update of them
    // rebuilds once.
    let locked = locked_kernel_dependencies();
    let missing: Vec<_> =
        resolved_kernel_dependencies().into_iter().filter(|p| !locked.contains(p)).collect();
    assert!(missing.is_empty(), "packages of the kernel the fingerprint misses: {missing:?}");
}

#[test]
fn ws_pre_02_the_fingerprint_follows_rhai_and_occt_but_not_the_app() {
    // An OCCT or Rhai update through `Cargo.lock` changes build results, so
    // does one of the font or 3MF readers of `qoob3d-core`; an update of the
    // UI crates or of a test-only crate must not rebuild.
    let names: BTreeSet<String> = locked_kernel_dependencies()
        .iter()
        .map(|line| line.split(' ').next().unwrap_or_default().to_owned())
        .collect();
    let wanted = ["rhai", "occt-sys", "opencascade-sys", "cxx", "cxx-build", "qoob3d-core"];
    for wanted in wanted.into_iter().chain(["skrifa", "read-fonts", "quick-xml", "miniz_oxide"]) {
        assert!(names.contains(wanted), "{wanted} missing: {names:?}");
    }
    for unwanted in ["egui", "eframe", "wgpu", "proptest", "qoob3d-project", "qoob3d-app"] {
        assert!(!names.contains(unwanted), "{unwanted} must not rebuild every part");
    }
}

#[test]
fn ws_pre_02_the_fingerprint_covers_the_core_code_the_kernel_runs() {
    // A change of the lettering, the mesh readers or the mesh welding in
    // `qoob3d-core` changes build results like a change of the kernel; the
    // tasks or the project folder code of `qoob3d-core` must not rebuild.
    let mut needed = core_files_the_kernel_runs();
    needed.insert("Cargo.toml".to_owned());
    let listed: BTreeSet<String> = CORE_FILES.iter().map(|f| (*f).to_owned()).collect();
    assert_eq!(listed, needed, "the core files in the fingerprint");
}

#[test]
fn ws_pre_02_the_kernel_fingerprint_enters_every_cache_key() {
    let version = key_version(&Mesh::default());
    assert!(version.contains(KERNEL_FINGERPRINT), "{version}");
    assert_eq!(KERNEL_FINGERPRINT.len(), 64, "SHA-256 as hex: {KERNEL_FINGERPRINT}");
}
