//! The kernel fingerprint (WS-PRE-02): SHA-256 over everything that can
//! change what the kernel builds, in `QOOB3D_KERNEL_FINGERPRINT` for
//! `worker::key_version`:
//!
//! - the files of `qoob3d-kernel` (`Cargo.toml`, `build.rs`, `cpp/`, `src/`)
//!   and of the OCCT fork `third_party/opencascade-sys` (`Cargo.toml` with
//!   the pinned OCCT version, `build.rs`, `include/`, `src/`, `OCCT/`);
//! - the files of `qoob3d-core` whose code the kernel runs while it builds
//!   ([`CORE`]: lettering, mesh readers, mesh welding), not its tasks or
//!   project folder code;
//! - `name version source` of every package the kernel depends on in
//!   `Cargo.lock` (Rhai, OCCT, cxx, the font and 3MF readers, …),
//!   transitively; of a path package only what its `Cargo.toml` lists as
//!   normal or build dependency: `Cargo.lock` lists dev dependencies too,
//!   and an update of a test-only crate must not rebuild every part.
//!
//! A rebuild of the app alone keeps the cache. Computed here rather than in
//! the kernel, whose build script would then compile its C++ wrappers again
//! after every change of a Rust file. `worker/tests.rs` recomputes it, derives
//! [`CORE`] from what the kernel names, and checks the packages against
//! `cargo metadata`.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use sha2::{Digest as _, Sha256};

/// What of the kernel folder can change a build result.
const KERNEL: (&str, [&str; 4]) =
    ("crates/qoob3d-kernel", ["Cargo.toml", "build.rs", "cpp", "src"]);
/// The same of the OCCT fork.
const FORK: (&str, [&str; 5]) =
    ("third_party/opencascade-sys", ["Cargo.toml", "build.rs", "include", "src", "OCCT"]);
/// The files of `qoob3d-core` the kernel runs while it builds: its manifest,
/// the zip reader of 3MF (`export.rs`), lettering, mesh import and welding.
/// What it takes from `project` only declares parts (pose, colour) or enters
/// the key as values (`[mesh]` settings).
const CORE: (&str, [&str; 8]) = (
    "crates/qoob3d-core",
    [
        "Cargo.toml",
        "src/export.rs",
        "src/font.rs",
        "src/font/regions.rs",
        "src/import.rs",
        "src/import/stl.rs",
        "src/import/three_mf.rs",
        "src/mesh.rs",
    ],
);
/// The path packages cargo builds the kernel with, and their folders.
const PATH_PACKAGES: [(&str, &str); 3] =
    [("qoob3d-kernel", KERNEL.0), ("qoob3d-core", CORE.0), ("opencascade-sys", FORK.0)];

/// Every file under `rel` (relative to `root`, with `/`).
fn walk(root: &Path, rel: &str, out: &mut Vec<String>) {
    let path = root.join(rel);
    if path.is_dir() {
        let entries =
            std::fs::read_dir(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            walk(root, &format!("{rel}/{}", entry.file_name().to_string_lossy()), out);
        }
    } else {
        out.push(rel.to_owned());
    }
}

/// One `[[package]]` of `Cargo.lock`.
struct Locked {
    name: String,
    version: String,
    source: String,
    dependencies: Vec<String>,
}

/// The names of the normal and build dependencies in the manifest of
/// `folder`, also those under `[target.….…]`.
fn manifest_dependencies(root: &Path, folder: &str) -> BTreeSet<String> {
    let path = root.join(folder).join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let manifest: toml::Table = text.parse().unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let targets = manifest.get("target").and_then(|t| t.as_table()).into_iter().flatten();
    let tables = std::iter::once(&manifest).chain(targets.filter_map(|(_, t)| t.as_table()));
    let mut names = BTreeSet::new();
    for table in tables {
        for kind in ["dependencies", "build-dependencies"] {
            let listed = table.get(kind).and_then(|d| d.as_table()).into_iter().flatten();
            names.extend(listed.map(|(name, _)| name.clone()));
        }
    }
    names
}

/// `name version source` of the packages `qoob3d-kernel` depends on.
fn locked_kernel_dependencies(root: &Path, lock: &str) -> BTreeSet<String> {
    let lock: toml::Table = lock.parse().unwrap_or_else(|e| panic!("Cargo.lock: {e}"));
    let text = |p: &toml::Table, key: &str| {
        p.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_owned()
    };
    let packages: Vec<Locked> = lock
        .get("package")
        .and_then(|p| p.as_array())
        .expect("Cargo.lock has packages")
        .iter()
        .filter_map(|p| p.as_table())
        .map(|p| Locked {
            name: text(p, "name"),
            version: text(p, "version"),
            source: text(p, "source"),
            dependencies: p
                .get("dependencies")
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_str().map(str::to_owned))
                .collect(),
        })
        .collect();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, p) in packages.iter().enumerate() {
        by_name.entry(&p.name).or_default().push(i);
    }
    // A dependency is `name` or, when several versions are locked,
    // `name version` (or `name version (source)`).
    let find = |dep: &str| -> usize {
        let mut words = dep.split(' ');
        let name = words.next().unwrap_or_default();
        let version = words.next();
        let found = by_name.get(name).into_iter().flatten().copied();
        let found: Vec<usize> =
            found.filter(|i| version.is_none_or(|v| packages[*i].version == v)).collect();
        assert_eq!(found.len(), 1, "Cargo.lock: dependency {dep:?} is not one package");
        found[0]
    };
    let kernel = find("qoob3d-kernel");
    let mut todo = vec![kernel];
    let mut seen = BTreeSet::new();
    let mut lines = BTreeSet::new();
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
                .unwrap_or_else(|| panic!("path package {} has no folder in build.rs", p.name));
            manifest_dependencies(root, folder)
        });
        let followed = p.dependencies.iter().filter(|d| {
            let name = d.split(' ').next().unwrap_or_default();
            wanted.as_ref().is_none_or(|w| w.contains(name))
        });
        todo.extend(followed.map(|d| find(d)));
    }
    lines
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let root = Path::new(&manifest).join("../..");
    let mut files = Vec::new();
    for (folder, parts) in [(KERNEL.0, &KERNEL.1[..]), (FORK.0, &FORK.1[..]), (CORE.0, &CORE.1[..])]
    {
        for part in parts {
            let rel = format!("{folder}/{part}");
            println!("cargo:rerun-if-changed={}", root.join(&rel).display());
            walk(&root, &rel, &mut files);
        }
    }
    files.sort();
    let mut hash = Sha256::new();
    hash.update((files.len() as u64).to_le_bytes());
    for rel in &files {
        let path = root.join(rel);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        field(&mut hash, rel.as_bytes());
        field(&mut hash, &bytes);
    }
    let lock_path = root.join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock = std::fs::read_to_string(&lock_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", lock_path.display()));
    let locked = locked_kernel_dependencies(&root, &lock);
    hash.update((locked.len() as u64).to_le_bytes());
    for line in &locked {
        field(&mut hash, line.as_bytes());
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=QOOB3D_KERNEL_FINGERPRINT={hex}");
}
