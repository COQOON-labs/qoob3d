//! The tool `render` (AG-MC-04): `qoob3d render-project` in a child process,
//! so neither the renderer's output nor a GPU failure reaches the protocol
//! on stdout. The PNG lands in `out/renders/` and comes back as an image;
//! several views come back as one contact sheet (AG-MC-09).

use std::path::Path;
use std::process::{Command, Stdio};

use qoob3d_core::project::layout;
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

/// Longest file name of a render before the part list is replaced by its
/// hash (finding 3.2: 17 part names exceeded the file system's limit).
const MAX_NAME: usize = 96;

/// `--region` from `region: {min: [x, y, z], max: [x, y, z]}`.
fn region(args: &Value) -> Result<Option<String>, String> {
    let region = &args["region"];
    if region.is_null() {
        return Ok(None);
    }
    let corner = |key: &str| -> Result<Vec<f64>, String> {
        region[key]
            .as_array()
            .filter(|a| a.len() == 3)
            .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<_>>>())
            .ok_or_else(|| format!("region.{key} must be [x, y, z] in mm"))
    };
    let (min, max) = (corner("min")?, corner("max")?);
    if !(min[0] < max[0] && min[1] < max[1] && min[2] <= max[2]) {
        return Err("region: min must be below max in x and y, not above it in z".to_owned());
    }
    let all: Vec<String> = min.iter().chain(&max).map(f64::to_string).collect();
    Ok(Some(all.join(",")))
}

/// The file name of a render: the views and parts, or a hash of them when
/// that would be too long.
fn file_name(views: &[&str], parts: &[String]) -> String {
    let safe = |name: &str| -> String {
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
    };
    let plain: Vec<String> =
        views.iter().map(|v| (*v).to_owned()).chain(parts.iter().map(|p| safe(p))).collect();
    let name = plain.join("-");
    if name.len() <= MAX_NAME {
        return name;
    }
    // FNV-1a: stable across runs, unlike the std hasher.
    let hash = name
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{}-{}-parts-{hash:016x}", views.join("-"), parts.len())
}

/// `render {view?, views?, parts?, source?, region?, width?, height?}`: the
/// result `{path, views: [{view, looks_along, up}], width, height}` (and
/// `view` for a single one) and the PNG as base64. Several views give one
/// contact sheet; `source` renders the shapes a throwaway script shows,
/// `region` frames a box instead of everything (AG-MC-12).
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

    let source = match &args["source"] {
        Value::Null => None,
        Value::String(s) if !parts.is_empty() => {
            let _ = s;
            return Err("render takes parts or source, not both".to_owned());
        }
        Value::String(s) => Some(s.clone()),
        _ => return Err("source must be a Rhai script".to_owned()),
    };
    let region = region(args)?;
    let names: Vec<&str> = views.iter().map(|v| v.0).collect();
    let owned: Vec<String> = parts.iter().map(|p| (*p).to_owned()).collect();
    let stem = match &source {
        // A script has no name: its text makes it one.
        Some(text) => file_name(&names, &[format!("script-{}", text.len()), text.clone()]),
        None => file_name(&names, &owned),
    };
    let relative = format!("out/renders/{stem}.png");
    let out = dir.join(&relative);
    let io = |e: std::io::Error| format!("{relative}: {e}");
    if let Some(folder) = out.parent() {
        std::fs::create_dir_all(folder).map_err(io)?;
    }
    let exe = std::env::current_exe().map_err(io)?;
    let mut command = Command::new(exe);
    command.arg(crate::RENDER_PROJECT).arg(dir).arg("--out").arg(&out);
    command.args(["--views", &names.join(","), "--size", &format!("{width}x{height}")]);
    if !parts.is_empty() {
        command.arg(format!("--parts={}", parts.join(",")));
    }
    if let Some(region) = &region {
        command.arg(format!("--region={region}"));
    }
    let script = layout::data_dir(dir)
        .join(layout::RUN_DIR)
        .join(format!("render-{}.rhai", std::process::id()));
    if let Some(text) = &source {
        if let Some(folder) = script.parent() {
            std::fs::create_dir_all(folder).map_err(io)?;
        }
        std::fs::write(&script, text).map_err(io)?;
        command.arg("--source").arg(&script);
    }
    let done = command.stdin(Stdio::null()).stdout(Stdio::null()).output();
    if source.is_some() {
        let _ = std::fs::remove_file(&script);
    }
    let done = done.map_err(io)?;
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
