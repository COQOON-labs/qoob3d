//! The tool `render` (AG-MC-04): `qoob3d render-project` in a child process,
//! so neither the renderer's output nor a GPU failure reaches the protocol
//! on stdout. The PNG lands in `out/renders/` and comes back as an image.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const VIEWS: [&str; 7] = ["iso", "front", "back", "left", "right", "top", "bottom"];
/// Default image size (AG-RD-01).
const SIZE: [u64; 2] = [1024, 768];
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (RFC 4648 §4).
fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            let index = (n >> (18 - 6 * i)) & 63;
            out.push(if i <= chunk.len() { char::from(ALPHABET[index as usize]) } else { '=' });
        }
    }
    out
}

/// `render {view?, parts?, width?, height?}`: the result `{path, view,
/// width, height}` and the PNG as base64.
pub(super) fn render(dir: &Path, args: &Value) -> Result<(Value, String), String> {
    let view = args["view"].as_str().unwrap_or(VIEWS[0]);
    if !VIEWS.contains(&view) {
        return Err(format!("unknown view {view:?}: use one of {}", VIEWS.join(", ")));
    }
    let parts: Vec<&str> = match &args["parts"] {
        Value::Null => Vec::new(),
        Value::Array(names) => names
            .iter()
            .map(|n| n.as_str().ok_or("parts must be an array of part names"))
            .collect::<Result<_, _>>()?,
        _ => return Err("parts must be an array of part names".to_owned()),
    };
    let size = |key: &str, default: u64| match &args[key] {
        Value::Null => Ok(default),
        v => v
            .as_u64()
            .filter(|n| (16..=8192).contains(n))
            .ok_or_else(|| format!("{key} must be an integer between 16 and 8192")),
    };
    let (width, height) = (size("width", SIZE[0])?, size("height", SIZE[1])?);

    let safe = |name: &str| -> String {
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
    };
    let name = std::iter::once(view.to_owned()).chain(parts.iter().map(|p| safe(p)));
    let relative = format!("out/renders/{}.png", name.collect::<Vec<_>>().join("-"));
    let out = dir.join(&relative);
    let io = |e: std::io::Error| format!("{relative}: {e}");
    if let Some(folder) = out.parent() {
        std::fs::create_dir_all(folder).map_err(io)?;
    }
    let exe = std::env::current_exe().map_err(io)?;
    let mut command = Command::new(exe);
    command.arg(crate::cli::RENDER_PROJECT).arg(dir).arg("--out").arg(&out);
    command.args(["--view", view, "--size", &format!("{width}x{height}")]);
    if !parts.is_empty() {
        command.arg(format!("--parts={}", parts.join(",")));
    }
    let done = command.stdin(Stdio::null()).stdout(Stdio::null()).output().map_err(io)?;
    if !done.status.success() {
        let stderr = String::from_utf8_lossy(&done.stderr);
        // Log lines come before the error of the process.
        let message = stderr.rfind("Error: ").map_or(&*stderr, |i| &stderr[i + 7..]);
        return Err(format!("render failed: {}", message.trim()));
    }
    let png = std::fs::read(&out).map_err(io)?;
    let result = json!({"path": relative, "view": view, "width": width, "height": height});
    Ok((result, base64(&png)))
}
