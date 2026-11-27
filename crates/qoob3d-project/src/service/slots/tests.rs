//! The runtime folder of the worker budget (WS-PRE-06).

use std::path::PathBuf;

use super::usable;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qoob3d-slots-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create a scratch folder");
    dir
}

#[test]
fn ws_pre_06_only_a_folder_of_this_user_holds_the_slots() {
    // In a shared temporary folder another user could make the folder
    // first and hold every slot.
    let me = rustix::process::geteuid().as_raw();
    let dir = scratch("own");
    assert_eq!(usable(&dir, me), Ok(()));
    let foreign = usable(&dir, me + 1).expect_err("a folder of another user");
    assert!(foreign.contains("owned by"), "{foreign}");

    let link = dir.with_extension("link");
    std::fs::remove_file(&link).ok();
    std::os::unix::fs::symlink(&dir, &link).expect("symlink");
    let linked = usable(&link, me).expect_err("a symlink");
    assert!(linked.contains("not a folder"), "{linked}");

    let file = dir.join("file");
    std::fs::write(&file, b"").expect("write a file");
    assert!(usable(&file, me).is_err(), "a file");
}
