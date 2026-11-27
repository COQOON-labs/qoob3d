//! Whether a service process is alive (WS-PRE-03).

use super::alive;

#[test]
fn ws_pre_03_a_running_process_is_alive_and_an_ended_one_is_not() {
    assert!(alive(std::process::id()), "this test process");
    let mut child = std::process::Command::new("true").spawn().expect("start true");
    let pid = child.id();
    child.wait().expect("true ends");
    assert!(!alive(pid), "an ended and reaped child");
    assert!(!alive(0), "pid 0 is no process");
    assert!(!alive(u32::MAX), "a pid out of range");
}
