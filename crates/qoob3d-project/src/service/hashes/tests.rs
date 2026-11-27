//! The digest memo (WS-PRE-01): remembered by path, size and modification
//! time, never for a file written within [`RACY`] of its reading, and large
//! files hashed in the background for [`Hashes::known`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use qoob3d_core::project::file_digest;

use super::{Hashes, INLINE_BYTES, RACY};

/// A fresh folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qoob3d-hashes-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create the test folder");
    dir
}

/// Writes `bytes` to `path` and sets its modification time to `modified`.
fn write(path: &Path, bytes: &[u8], modified: SystemTime) {
    std::fs::write(path, bytes).expect("write");
    let file = std::fs::File::options().write(true).open(path).expect("open");
    file.set_modified(modified).expect("set the modification time");
}

fn ago(seconds: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(seconds)
}

#[test]
fn ws_pre_01_a_digest_is_remembered_by_size_and_modification_time() {
    let dir = folder("stamp");
    let file = dir.join("a.stl");
    let old = ago(60);
    write(&file, b"first", old);
    let hashes = Hashes::default();
    assert_eq!(hashes.digest(&file), file_digest(b"first"));
    // Same size and time, other bytes: the remembered digest proves that
    // the file was not read again.
    write(&file, b"other", old);
    assert_eq!(hashes.digest(&file), file_digest(b"first"));
    // Another time, or another size, reads it again.
    write(&file, b"other", ago(30));
    assert_eq!(hashes.digest(&file), file_digest(b"other"));
    write(&file, b"longer", ago(30));
    assert_eq!(hashes.digest(&file), file_digest(b"longer"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ws_pre_01_a_file_written_just_before_it_was_read_is_read_again() {
    // A second write within the resolution of the timestamp keeps size and
    // time; such a digest must not be trusted (the racy case).
    let dir = folder("racy");
    let file = dir.join("a.rhai");
    let now = SystemTime::now();
    write(&file, b"first", now);
    let hashes = Hashes::default();
    assert_eq!(hashes.digest(&file), file_digest(b"first"));
    write(&file, b"other", now);
    assert_eq!(hashes.digest(&file), file_digest(b"other"));
    assert!(RACY <= Duration::from_secs(2), "racy window {RACY:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ws_pre_01_a_file_that_is_gone_hashes_as_empty() {
    let dir = folder("gone");
    let hashes = Arc::new(Hashes::default());
    let file = dir.join("missing.step");
    assert_eq!(hashes.digest(&file), file_digest(b""));
    assert_eq!(hashes.known(&file), Some(file_digest(b"")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ws_pre_01_known_hashes_small_files_at_once_and_large_ones_in_the_background() {
    let dir = folder("known");
    let hashes = Arc::new(Hashes::default());
    let small = dir.join("small.rhai");
    write(&small, b"part();", ago(60));
    assert_eq!(hashes.known(&small), Some(file_digest(b"part();")));

    let large = dir.join("large.stl");
    let bytes = vec![7u8; usize::try_from(INLINE_BYTES).expect("fits") + 1];
    write(&large, &bytes, ago(60));
    assert_eq!(hashes.known(&large), None, "a large file is never hashed by the asking request");
    let deadline = Instant::now() + Duration::from_secs(10);
    let digest = loop {
        if let Some(d) = hashes.known(&large) {
            break d;
        }
        assert!(Instant::now() < deadline, "the background thread did not hash it");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(digest, file_digest(&bytes));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ws_pre_01_literal_imports_are_scanned_again_when_a_module_changes() {
    let dir = folder("imports");
    std::fs::create_dir_all(dir.join("parts")).expect("parts");
    std::fs::create_dir_all(dir.join("lib")).expect("lib");
    let part = Path::new("parts/a.rhai");
    write(&dir.join(part), b"import \"lib/x\" as x;\n", ago(60));
    write(&dir.join("lib/x.rhai"), b"fn f() { 1 }\n", ago(60));
    write(&dir.join("lib/y.rhai"), b"fn g() { 2 }\n", ago(60));
    let hashes = Hashes::default();
    assert_eq!(hashes.imports(&dir, part).expect("scan"), vec![PathBuf::from("lib/x.rhai")]);
    // The module now imports another one: its stamp changed, so the scan runs again.
    write(&dir.join("lib/x.rhai"), b"import \"lib/y\" as y;\nfn f() { 1 }\n", ago(30));
    assert_eq!(
        hashes.imports(&dir, part).expect("scan"),
        vec![PathBuf::from("lib/x.rhai"), PathBuf::from("lib/y.rhai")]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ws_pre_01_a_remembered_key_follows_every_change_of_its_inputs() {
    use qoob3d_core::project::{KeyInputs, cache_key};
    let a = KeyInputs {
        program_version: "v1".to_owned(),
        part_file: file_digest(b"part"),
        modules: vec![file_digest(b"module")],
        values_read: vec![("hull.length".to_owned(), serde_json::json!(120.5))],
        dependency_keys: vec!["dep".to_owned()],
    };
    let changes: [fn(&mut KeyInputs); 5] = [
        |k| k.program_version.push('x'),
        |k| k.part_file = file_digest(b"other part"),
        |k| k.modules.push(file_digest(b"another module")),
        |k| k.values_read[0].1 = serde_json::json!(121),
        |k| k.dependency_keys[0].push('x'),
    ];
    let hashes = Hashes::default();
    assert_eq!(hashes.key("hull", &a), cache_key(&a));
    for change in changes {
        let mut b = a.clone();
        change(&mut b);
        assert_ne!(cache_key(&b), cache_key(&a));
        assert_eq!(hashes.key("hull", &b), cache_key(&b), "{b:?}");
        assert_eq!(hashes.key("hull", &a), cache_key(&a));
    }
    // By part: another part with the same inputs has the same key.
    assert_eq!(hashes.key("deck", &a), cache_key(&a));
}
