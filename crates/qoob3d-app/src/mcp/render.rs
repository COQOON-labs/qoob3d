//! The tool `render` (AG-MC-04): `qoob3d render-project` in a child process,
//! so neither the renderer's output nor a GPU failure reaches the protocol
//! on stdout. The PNG lands in `out/renders/` and comes back as an image;
//! several views come back as one contact sheet (AG-MC-09).

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// The view table (ADR-0006, +Z up): `(view, direction the camera looks
/// along, axis that is up in the image)`. The same table is in `guide.md`.
const VIEWS: [(&str, &str, &str); 7] = [
    ("iso", "-X +Y -Z (from the front right top corner)", "+Z"),
    ("front", "+Y", "+Z"),
    ("back", "-Y", "+Z"),
    ("left", "+X", "+Z"),
    ("right", "-X", "+Z"),
    ("top", "-Z", "+Y"),
    ("bottom", "+Z", "-Y"),
];
/// Views of a contact sheet (AG-MC-09).
const SHEET_VIEWS: std::ops::RangeInclusive<usize> = 2..=6;
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

/// The views asked for: `views` (a contact sheet) or the single `view`.
fn views(args: &Value) -> Result<Vec<&'static (&'static str, &'static str, &'static str)>, String> {
    let known = |name: &str| {
        let names: Vec<&str> = VIEWS.iter().map(|v| v.0).collect();
        VIEWS
            .iter()
            .find(|v| v.0 == name)
            .ok_or_else(|| format!("unknown view {name:?}: use one of {}", names.join(", ")))
    };
    let sheet = match &args["views"] {
        Value::Null => return Ok(vec![known(args["view"].as_str().unwrap_or(VIEWS[0].0))?]),
        Value::Array(names) => names,
        _ => return Err("views must be an array of view names".to_owned()),
    };
    if !SHEET_VIEWS.contains(&sheet.len()) {
        return Err(format!(
            "views takes 2 to 6 views, got {}; for one view use `view`",
            sheet.len()
        ));
    }
    sheet
        .iter()
        .map(|name| known(name.as_str().ok_or("views must be an array of view names")?))
        .collect()
}

/// `render {view?, views?, parts?, width?, height?}`: the result `{path,
/// views: [{view, looks_along, up}], width, height}` (and `view` for a single
/// one) and the PNG as base64. Several views give one contact sheet.
pub(super) fn render(dir: &Path, args: &Value) -> Result<(Value, String), String> {
    let views = views(args)?;
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
    let names: Vec<&str> = views.iter().map(|v| v.0).collect();
    let name = names.iter().map(|v| (*v).to_owned()).chain(parts.iter().map(|p| safe(p)));
    let relative = format!("out/renders/{}.png", name.collect::<Vec<_>>().join("-"));
    let out = dir.join(&relative);
    let io = |e: std::io::Error| format!("{relative}: {e}");
    if let Some(folder) = out.parent() {
        std::fs::create_dir_all(folder).map_err(io)?;
    }
    let exe = std::env::current_exe().map_err(io)?;
    let mut command = Command::new(exe);
    command.arg(crate::cli::RENDER_PROJECT).arg(dir).arg("--out").arg(&out);
    command.args(["--views", &names.join(","), "--size", &format!("{width}x{height}")]);
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
    let table: Vec<Value> = views
        .iter()
        .map(|(view, looks_along, up)| json!({"view": view, "looks_along": looks_along, "up": up}))
        .collect();
    let mut result = json!({"path": relative, "views": table, "width": width, "height": height});
    if let [single] = names[..] {
        result["view"] = json!(single);
    }
    Ok((result, base64(&png)))
}
