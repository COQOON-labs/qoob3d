//! PNG and Git checks for `tests/agent.rs`, `tests/mcp.rs` and
//! `tests/mcp_guide.rs` (every item is used by each of them, see
//! `common/agent.rs`): renders (AG-RD-01) and `save` (AG-SV-07).
//!
//! Included with `#[path = "common/artifacts.rs"] mod artifacts;`.

use std::path::Path;
use std::process::{Command, Stdio};

// ------------------------------------------------------------------ PNG

/// Width and height from the PNG header, read by hand (independent of the
/// writer): 8 signature bytes `89 50 4E 47 0D 0A 1A 0A`, then the first
/// chunk, which must be `IHDR` with length 13: u32 BE width at byte 16,
/// u32 BE height at byte 20 (PNG specification §5.2, §11.2.2).
pub(crate) fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() > 33, "PNG shorter than signature + IHDR: {} bytes", bytes.len());
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&bytes[8..12], &[0, 0, 0, 13], "IHDR length");
    assert_eq!(&bytes[12..16], b"IHDR", "first chunk is IHDR");
    let be = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    (be(16), be(20))
}

/// Number of distinct pixel values (capped at 64) of a PNG, decoded with
/// the `png` crate: a render that shows a shaded part on a background has
/// more than a handful, an empty or failed render has one.
pub(crate) fn png_colors(bytes: &[u8]) -> usize {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("valid PNG");
    let size = reader.output_buffer_size().expect("PNG size fits");
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).expect("PNG pixels decode");
    let px = (info.line_size / info.width as usize).max(1);
    let mut seen = std::collections::BTreeSet::new();
    for pixel in buf[..info.buffer_size()].chunks_exact(px) {
        seen.insert(pixel.to_vec());
        if seen.len() >= 64 {
            break;
        }
    }
    seen.len()
}

// ------------------------------------------------------------------ Git

/// `git <args>` in `dir`; trimmed stdout, or `Err` with stderr.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git not runnable: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// A fresh repository in `dir` with a local identity (no global config
/// needed, nothing outside the temp project is touched) and no signing.
pub(crate) fn git_init(dir: &Path) {
    for args in [
        &["init", "--quiet"][..],
        &["config", "user.name", "QOOB3D Test"][..],
        &["config", "user.email", "test@qoob3d.invalid"][..],
        &["config", "commit.gpgsign", "false"][..],
    ] {
        git(dir, args).unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    }
}

/// Files tracked at `HEAD` (`git ls-tree`), relative to the repository.
pub(crate) fn git_tracked(dir: &Path) -> Vec<String> {
    let out = git(dir, &["ls-tree", "-r", "--name-only", "HEAD"]).expect("git ls-tree HEAD");
    out.lines().map(str::to_owned).collect()
}
