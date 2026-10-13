//! `cargo xtask printers <orca-profiles-dir>`: regenerates the printer
//! database `crates/qoob3d-core/src/printer/data.rs` from OrcaSlicer's machine
//! profiles (ADR-0021). Only factual geometry is taken: printable area,
//! non-printable zones, per-nozzle reachable areas and build height.
//!
//! Get the profiles with a sparse checkout, e.g.
//! ```text
//! git clone --depth 1 --filter=blob:none --no-checkout https://github.com/SoftFever/OrcaSlicer orca
//! git -C orca sparse-checkout set --no-cone '/resources/profiles/*.json' '/resources/profiles/*/machine/**/*.json'
//! git -C orca checkout
//! cargo xtask printers orca/resources/profiles
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Vendors that are not real printers (generic templates, test arenas).
const SKIPPED_VENDORS: &[&str] = &["Custom", "OrcaArena"];
/// Nicer display names for some vendor files.
const VENDOR_NAMES: &[(&str, &str)] = &[("Bambulab", "Bambu Lab")];
/// Must match `MAX_BED_POINTS` in the renderer (uniform array size).
const MAX_POINTS: usize = 192;
const OUTPUT: &str = "crates/qoob3d-core/src/printer/data.rs";

type Polygon = Vec<[f32; 2]>;

struct Geometry {
    printable: Polygon,
    exclude: Polygon,
    extruders: Vec<Polygon>,
    height: f32,
}

struct Printer {
    id: String,
    vendor: String,
    name: String,
    geometry: Geometry,
}

pub fn generate(root: &Path, profiles: &Path) -> Result<(), Vec<String>> {
    let mut printers = Vec::new();
    let mut skipped = Vec::new();
    let mut vendor_files: Vec<_> = std::fs::read_dir(profiles)
        .map_err(|e| vec![format!("{}: {e}", profiles.display())])?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    vendor_files.sort();

    for vendor_file in vendor_files {
        let stem = vendor_file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if SKIPPED_VENDORS.contains(&stem.as_str()) {
            continue;
        }
        let Some(vendor) = read_json(&vendor_file) else { continue };
        let Some(models) = vendor["machine_model_list"].as_array() else { continue };
        let raw_vendor = vendor["name"].as_str().unwrap_or(&stem);
        let vendor_name = VENDOR_NAMES
            .iter()
            .find(|(raw, _)| *raw == raw_vendor)
            .map_or(raw_vendor, |(_, nice)| nice)
            .to_string();

        let machines = load_machines(&profiles.join(&stem).join("machine"));
        for model in models.iter().filter_map(|m| m["name"].as_str()) {
            match build(&machines, model) {
                Ok(geometry) => printers.push(Printer {
                    id: String::new(),
                    vendor: vendor_name.clone(),
                    name: model.to_string(),
                    geometry,
                }),
                Err(reason) => skipped.push(format!("{vendor_name} / {model}: {reason}")),
            }
        }
    }

    printers.sort_by(|a, b| (&a.vendor, &a.name).cmp(&(&b.vendor, &b.name)));
    let mut used = BTreeSet::new();
    for p in &mut printers {
        let mut id = slug(&p.name);
        if !used.insert(id.clone()) {
            id = slug(&format!("{} {}", p.vendor, p.name));
            used.insert(id.clone());
        }
        p.id = id;
    }

    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(profiles)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let path = root.join(OUTPUT);
    std::fs::write(&path, render(&printers, &commit)).map_err(|e| vec![e.to_string()])?;
    let fmt = Command::new("rustfmt").args(["--edition", "2024"]).arg(&path).status();
    if !fmt.is_ok_and(|s| s.success()) {
        return Err(vec![format!("rustfmt failed on {}", path.display())]);
    }
    println!("printers: {} models written to {OUTPUT} (OrcaSlicer {commit})", printers.len());
    for s in &skipped {
        println!("printers: skipped {s}");
    }
    Ok(())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// All machine profiles of a vendor, keyed by name. Some vendors use
/// subdirectories (`machine/ECC/…`), so this recurses.
fn load_machines(dir: &Path) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            map.extend(load_machines(&path));
        } else if let Some(v) = read_json(&path)
            && let Some(name) = v["name"].as_str()
        {
            map.insert(name.to_string(), v);
        }
    }
    map
}

/// Looks up `key`, following the `inherits` chain inside the vendor.
fn resolve<'a>(
    machines: &'a BTreeMap<String, Value>,
    mut d: &'a Value,
    key: &str,
) -> Option<&'a Value> {
    for _ in 0..20 {
        if let Some(v) = d.get(key) {
            return Some(v);
        }
        d = machines.get(d.get("inherits")?.as_str()?)?;
    }
    None
}

fn build(machines: &BTreeMap<String, Value>, model: &str) -> Result<Geometry, String> {
    let mut candidates: Vec<&Value> = machines
        .values()
        .filter(|d| {
            d["type"] == "machine"
                && d["printer_model"] == model
                && d["instantiation"].as_str() != Some("false")
        })
        .collect();
    candidates
        .sort_by_key(|d| (d["printer_variant"].as_str() != Some("0.4"), d["name"].to_string()));
    let d = candidates.first().ok_or("no machine profile")?;

    let printable = polygon(resolve(machines, d, "printable_area").ok_or("no printable_area")?)?;
    if printable.len() < 3 || area(&printable) <= 1.0 {
        return Err("degenerate printable_area".into());
    }
    let exclude = match resolve(machines, d, "bed_exclude_area") {
        Some(v) => polygon(v).unwrap_or_default(),
        None => Vec::new(),
    };
    let exclude =
        if exclude.len() >= 3 && bbox_area(&exclude) > 0.0 { exclude } else { Vec::new() };

    let mut extruders: Vec<Polygon> = match resolve(machines, d, "extruder_printable_area") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.as_str())
            .filter_map(|s| parse_points(s).ok())
            .filter(|p| p.len() >= 3)
            .collect(),
        _ => Vec::new(),
    };
    if extruders.len() < 2 || extruders.iter().all(|e| *e == extruders[0]) {
        extruders.clear();
    }

    let height = match resolve(machines, d, "printable_height") {
        Some(Value::String(s)) => s.trim().parse::<f32>().map_err(|e| e.to_string())?,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) as f32,
        _ => return Err("no printable_height".into()),
    };
    if height <= 0.0 {
        return Err("non-positive printable_height".into());
    }
    let total = printable.len() + exclude.len() + extruders.iter().map(Vec::len).sum::<usize>();
    if total > MAX_POINTS {
        return Err(format!("{total} polygon points > {MAX_POINTS}"));
    }
    Ok(Geometry { printable, exclude, extruders, height })
}

/// Accepts `["0x0","256x0",…]`, `"0x0,256x0,…"` or `["0x0,11x0,…"]`.
fn polygon(v: &Value) -> Result<Polygon, String> {
    let joined = match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            items.iter().filter_map(|i| i.as_str()).collect::<Vec<_>>().join(",")
        }
        _ => return Err("unexpected polygon value".into()),
    };
    parse_points(&joined)
}

fn parse_points(s: &str) -> Result<Polygon, String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (x, y) = p.split_once('x').ok_or_else(|| format!("bad point {p:?}"))?;
            Ok([
                x.trim().parse().map_err(|_| format!("bad x in {p:?}"))?,
                y.trim().parse().map_err(|_| format!("bad y in {p:?}"))?,
            ])
        })
        .collect()
}

fn area(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    0.5 * (0..n)
        .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
        .sum::<f32>()
        .abs()
}

fn bbox_area(p: &[[f32; 2]]) -> f32 {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for q in p {
        for k in 0..2 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    (hi[0] - lo[0]).max(0.0) * (hi[1] - lo[1]).max(0.0)
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn render(printers: &[Printer], commit: &str) -> String {
    let poly = |p: &Polygon| {
        let pts: Vec<String> = p.iter().map(|[x, y]| format!("[{x:?}, {y:?}]")).collect();
        format!("&[{}]", pts.join(", "))
    };
    let mut out = format!(
        "//! @generated by `cargo xtask printers` from OrcaSlicer ({commit}) machine\n\
         //! profiles – do not edit by hand (ADR-0021). Coordinates: printer\n\
         //! coordinates in mm (X right, Y towards the back).\n\n\
         use super::PrinterProfile;\n\n\
         pub static PRINTERS: &[PrinterProfile] = &[\n"
    );
    for p in printers {
        let g = &p.geometry;
        let extruders: Vec<String> = g.extruders.iter().map(poly).collect();
        let _ = writeln!(
            out,
            "PrinterProfile {{ id: {:?}, vendor: {:?}, name: {:?}, printable_area: {}, \
             exclude_area: {}, extruder_areas: &[{}], max_height_mm: {:?} }},",
            p.id,
            p.vendor,
            p.name,
            poly(&g.printable),
            poly(&g.exclude),
            extruders.join(", "),
            g.height
        );
    }
    out.push_str("];\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_point_formats() {
        let a = polygon(&serde_json::json!(["0x0", "256x0", "256x256"])).unwrap();
        let b = polygon(&serde_json::json!("0x0,256x0,256x256")).unwrap();
        let c = polygon(&serde_json::json!(["0x0,256x0", "256x256"])).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a[2], [256.0, 256.0]);
        assert!(parse_points("1x").is_err());
    }

    #[test]
    fn slugs_are_stable() {
        assert_eq!(slug("Bambu Lab X1 Carbon"), "bambu-lab-x1-carbon");
        assert_eq!(slug("Troodon 2.0 - Klipper"), "troodon-2-0-klipper");
    }

    #[test]
    fn polygon_area() {
        assert_eq!(area(&[[0.0, 0.0], [2.0, 0.0], [2.0, 3.0], [0.0, 3.0]]), 6.0);
    }
}
