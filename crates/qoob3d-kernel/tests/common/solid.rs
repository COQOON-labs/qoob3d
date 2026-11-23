//! Solid and error checks of `tests/script.rs` and `tests/script_helpers.rs`
//! (every item is used by each of them, see `common/mod.rs`).
//!
//! Included with `#[path = "common/solid.rs"] mod solid;` next to `mod common;`.

use qoob3d_kernel::script::{RunOutput, ScriptError};

use crate::common::{POS, VOL, assert_rel, shape};

/// A valid solid with the volume `expected` to `VOL` relative.
pub(crate) fn assert_volume(out: &RunOutput, name: &str, expected: f64) {
    let s = shape(out, name);
    assert!(s.is_valid(), "{name} is not a valid solid");
    assert_rel(s.volume(), expected, VOL, &format!("volume of {name}"));
}

pub(crate) fn assert_size(out: &RunOutput, name: &str, size: [f64; 3]) {
    let (a, b) = shape(out, name).bbox();
    for i in 0..3 {
        assert!(
            (b[i] - a[i] - size[i]).abs() < POS,
            "bbox size of {name}: got {a:?}..{b:?}, expected size {size:?}"
        );
    }
}

/// `function` is the name of the failing script API function (AG-SC-03),
/// or for an operator the operator token; one of `functions` must match.
pub(crate) fn expect_geometry(e: &ScriptError, want_line: usize, functions: &[&str]) {
    match e {
        ScriptError::Geometry { line, function, message, .. } => {
            assert_eq!(*line, want_line, "{e:?}");
            assert!(
                functions.contains(&function.as_str()),
                "function {function:?} not in {functions:?}: {e:?}"
            );
            assert!(!message.is_empty(), "{e:?}");
        }
        other => panic!("expected a Geometry error on line {want_line}, got {other:?}"),
    }
}
