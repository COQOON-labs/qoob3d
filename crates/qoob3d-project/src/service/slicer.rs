//! A kit as a slicer project (AG-SV-10, ADR-0034): the exported parts as
//! objects on the plates of a Bambu Studio project, each in the filament of
//! its colour. Two ways to print in colour:
//!
//! - one colour per plate (default): every part is an object of its own,
//!   and parts of different colours never share a plate, so no plate needs
//!   a filament change;
//! - multi-colour objects (`multicolor`): a part that declares `print_with`
//!   prints inside its host as one object with several filaments. A plate
//!   holds objects that need the same filaments, so only the plates of
//!   multi-colour objects change filaments.

use std::path::Path;

use qoob3d_core::TriMesh;
use qoob3d_core::export::bambu::{self, PRESETS, Presets, Project, ProjectObject, ProjectPart};
use qoob3d_core::fs::write_atomic;
use qoob3d_core::printer::{DEFAULT_PRINTER_ID, PrinterProfile};
use qoob3d_core::project::{Placement, PrintPose, plates};
use serde_json::{Value, json};

use super::mesh_detail;

/// The colour of parts that declare none: a light grey.
const UNDECLARED_COLOR: [u8; 3] = [0xC8, 0xC8, 0xC8];

/// One exported part with what it declares for printing.
pub(super) struct Part {
    pub(super) name: String,
    pub(super) mesh: TriMesh,
    pub(super) pose: Option<PrintPose>,
    pub(super) color: Option<[u8; 3]>,
    pub(super) print_with: Option<String>,
}

/// Why a project was not written.
pub(super) enum Refused {
    /// The request does not make a project; the message says why.
    Request(String),
    Io(String),
}

/// The printer a project is laid out for: bed, height and the names of
/// its presets in the slicer.
pub(super) struct Printer {
    bed: [f32; 2],
    /// The zone at the bed's front left corner that must stay empty.
    corner: [f32; 2],
    height: f32,
    presets: &'static Presets,
}

/// The printer `id` (default: the default printer of the database). Only
/// printers whose presets are known can carry a project.
pub(super) fn printer(id: Option<&str>) -> Result<Printer, String> {
    let id = id.unwrap_or(DEFAULT_PRINTER_ID);
    if let (Some(profile), Some(presets)) = (PrinterProfile::by_id(id), Presets::by_id(id)) {
        // The exclusion zones of these printers are boxes at the origin.
        let corner =
            profile.exclude_area.iter().fold([0.0f32; 2], |c, p| [c[0].max(p[0]), c[1].max(p[1])]);
        Ok(Printer { bed: profile.size(), corner, height: profile.max_height_mm, presets })
    } else {
        let known: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        Err(format!("no slicer project for printer {id:?}; known: {}", known.join(", ")))
    }
}

/// One object before it is placed: posed meshes with their filaments.
struct Posed {
    name: String,
    /// (part name, mesh, filament from 1)
    meshes: Vec<(String, TriMesh, usize)>,
    footprint: [f32; 2],
}

/// Writes `parts` as a Bambu Studio project to `out`; what the file holds.
pub(super) fn export(
    parts: &[Part],
    multicolor: bool,
    printer: &Printer,
    out: &Path,
) -> Result<Value, Refused> {
    let bed = printer.bed;
    let mut filaments: Vec<[u8; 3]> = Vec::new();
    let slots: Vec<usize> = parts
        .iter()
        .map(|p| {
            let color = p.color.unwrap_or(UNDECLARED_COLOR);
            filaments.iter().position(|&c| c == color).unwrap_or_else(|| {
                filaments.push(color);
                filaments.len() - 1
            }) + 1
        })
        .collect();

    let objects = if multicolor { together(parts, &slots)? } else { apart(parts, &slots) };
    let sizes: Vec<[f32; 2]> = objects.iter().map(|o| o.footprint).collect();
    // A plate holds objects that need the same filaments: no plate changes
    // filaments more often than its objects ask for.
    let mut sets: Vec<Vec<usize>> = Vec::new();
    let groups: Vec<usize> = objects
        .iter()
        .map(|o| {
            let mut set: Vec<usize> = o.meshes.iter().map(|m| m.2).collect();
            set.sort_unstable();
            set.dedup();
            sets.iter().position(|s| *s == set).unwrap_or_else(|| {
                sets.push(set);
                sets.len() - 1
            })
        })
        .collect();
    let placed = plates(&sizes, &groups, bed, printer.corner);

    let project = Project {
        objects: objects
            .iter()
            .zip(&placed)
            .map(|(o, place)| ProjectObject {
                name: o.name.clone(),
                parts: o
                    .meshes
                    .iter()
                    .map(|(name, mesh, filament)| ProjectPart {
                        name: name.clone(),
                        mesh,
                        filament: *filament,
                    })
                    .collect(),
                plate: place.plate,
                centre: place.centre,
            })
            .collect(),
        filaments: filaments.clone(),
        bed,
        height: printer.height,
        presets: printer.presets,
    };
    write_atomic(out, &bambu::bambu_project(&project))
        .map_err(|e| Refused::Io(format!("{}: {e}", out.display())))?;

    let kit =
        Kit { parts, slots: &slots, filaments: &filaments, objects: &objects, placed: &placed };
    Ok(report(&kit, multicolor, printer, out))
}

/// A kit laid out on plates: the parts with their filament slots, the
/// filament colours, and the objects with their places.
struct Kit<'a> {
    parts: &'a [Part],
    slots: &'a [usize],
    filaments: &'a [[u8; 3]],
    objects: &'a [Posed],
    placed: &'a [Placement],
}

/// What the project holds: filaments, plates, objects and warnings.
fn report(kit: &Kit<'_>, multicolor: bool, printer: &Printer, out: &Path) -> Value {
    let Kit { parts, slots, filaments, objects, placed } = *kit;
    let bed = printer.bed;
    let plate_count = placed.iter().map(|p| p.plate + 1).max().unwrap_or(1);
    let mut warnings = Vec::new();
    let details: Vec<Value> = objects
        .iter()
        .zip(placed)
        .map(|(o, place)| {
            let meshes: Vec<Value> = o
                .meshes
                .iter()
                .map(|(name, mesh, filament)| {
                    let mut d = mesh_detail(mesh);
                    d["part"] = json!(name);
                    d["filament"] = json!(filament);
                    d
                })
                .collect();
            if !place.fits {
                warnings.push(format!(
                    "{} ({:.0} x {:.0} mm) is larger than the bed of {:.0} x {:.0} mm",
                    o.name, o.footprint[0], o.footprint[1], bed[0], bed[1]
                ));
            }
            json!({
                "object": o.name,
                "plate": place.plate + 1,
                "size": o.footprint,
                "fits": place.fits,
                "watertight": meshes.iter().all(|m| m["watertight"] == json!(true)),
                "parts": meshes,
            })
        })
        .collect();
    let plate_list: Vec<Value> = (0..plate_count)
        .map(|plate| {
            let on_plate = || objects.iter().zip(placed).filter(move |(_, p)| p.plate == plate);
            let mut used: Vec<usize> =
                on_plate().flat_map(|(o, _)| o.meshes.iter().map(|m| m.2)).collect();
            used.sort_unstable();
            used.dedup();
            json!({
                "plate": plate + 1,
                "filaments": used,
                "objects": on_plate().map(|(o, _)| o.name.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let filament_list: Vec<Value> = filaments
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "filament": i + 1,
                "color": format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
                "parts": parts
                    .iter()
                    .zip(slots)
                    .filter(|(_, slot)| **slot == i + 1)
                    .map(|(p, _)| p.name.as_str())
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    if parts.iter().any(|p| p.color.is_none()) {
        warnings.push("parts without color print in a light grey filament".to_owned());
    }
    json!({
        "file": out.file_name().map(|f| f.to_string_lossy().into_owned()),
        "slicer": "Bambu Studio",
        "multicolor": multicolor,
        "printer": printer.presets.id,
        "presets": {
            "printer": printer.presets.printer_name(),
            "process": printer.presets.process_name(),
            "filament": printer.presets.filament_name(),
        },
        "bed": bed,
        "filaments": filament_list,
        "plates": plate_list,
        "objects": details,
        "warnings": warnings,
    })
}

fn footprint(meshes: &[TriMesh]) -> [f32; 2] {
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for b in meshes.iter().filter_map(TriMesh::bounds) {
        for k in 0..2 {
            lo[k] = lo[k].min(b.min[k]);
            hi[k] = hi[k].max(b.max[k]);
        }
    }
    if lo[0] > hi[0] { [0.0; 2] } else { [hi[0] - lo[0], hi[1] - lo[1]] }
}

fn copies(name: &str, count: u32) -> impl Iterator<Item = String> + '_ {
    (1..=count).map(move |i| if count > 1 { format!("{name} {i}") } else { name.to_owned() })
}

/// Every part an object of its own, in its own pose.
fn apart(parts: &[Part], slots: &[usize]) -> Vec<Posed> {
    parts
        .iter()
        .zip(slots)
        .flat_map(|(part, &slot)| {
            let pose = part.pose.clone().unwrap_or_default();
            let mesh = pose.apply(&part.mesh);
            let size = footprint(std::slice::from_ref(&mesh));
            copies(&part.name, pose.count)
                .map(move |name| Posed {
                    name,
                    meshes: vec![(part.name.clone(), mesh.clone(), slot)],
                    footprint: size,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Parts with `print_with` inside their host, posed with it.
fn together(parts: &[Part], slots: &[usize]) -> Result<Vec<Posed>, Refused> {
    for part in parts {
        let Some(host) = &part.print_with else { continue };
        let refuse = |why: &str| {
            Err(Refused::Request(format!("{} prints with {host}, which {why}", part.name)))
        };
        match parts.iter().find(|p| p.name == *host) {
            None => return refuse("is not among the exported parts"),
            Some(h) if h.print_with.is_some() => return refuse("prints with another part itself"),
            Some(_) => {}
        }
    }
    Ok(parts
        .iter()
        .zip(slots)
        .filter(|(part, _)| part.print_with.is_none())
        .flat_map(|(host, &slot)| {
            let guests: Vec<(&Part, usize)> = parts
                .iter()
                .zip(slots)
                .filter(|(p, _)| p.print_with.as_deref() == Some(host.name.as_str()))
                .map(|(p, &s)| (p, s))
                .collect();
            let pose = host.pose.clone().unwrap_or_default();
            let mut together: Vec<&TriMesh> = vec![&host.mesh];
            together.extend(guests.iter().map(|(p, _)| &p.mesh));
            let posed = pose.apply_all(&together);
            let size = footprint(&posed);
            let names = std::iter::once((host.name.clone(), slot))
                .chain(guests.iter().map(|(p, s)| (p.name.clone(), *s)));
            let meshes: Vec<(String, TriMesh, usize)> =
                names.zip(posed).map(|((name, slot), mesh)| (name, mesh, slot)).collect();
            copies(&host.name, pose.count)
                .map(|name| Posed { name, meshes: meshes.clone(), footprint: size })
                .collect::<Vec<_>>()
        })
        .collect())
}
