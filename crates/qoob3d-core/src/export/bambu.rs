//! A kit as a Bambu Studio project (AG-SV-10, ADR-0034): a 3MF in the
//! layout Bambu Studio writes itself, with plates, objects made of parts
//! and a filament per part. Bambu Studio reads plates and filaments only
//! from files that name it as the application, and takes everything else
//! as bare geometry.
//!
//! Bambu Studio creates plates only for a file with project settings, and
//! takes a printer, process or filament from its own installed presets
//! when the settings name one and list no differences to it. The file
//! therefore names the default presets of the printer ([`Presets`]) and
//! carries none of their values: the slicer's own, current ones apply.

use std::fmt::Write as _;

use super::{xml_escape, zip_store};
use crate::mesh::TriMesh;

/// The version Bambu Studio reads the layout as. A file newer than the
/// running slicer asks the user first, one older than 2.0 has its prime
/// tower settings converted: this is the oldest version without either.
const WRITER_VERSION: &str = "02.00.00.00";

/// The names of the presets Bambu Studio ships for a printer with a 0.4 mm
/// nozzle: the printer, its default process and its default PLA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presets {
    /// Id of the printer in the printer database.
    pub id: &'static str,
    pub printer: &'static str,
    pub process: &'static str,
    pub filament: &'static str,
}

/// The single-nozzle Bambu Lab printers (names as of Bambu Studio 2.9; the
/// names of the process and the filament follow no rule, e.g. the P1S).
/// Listed are the printers whose three presets the installed slicer has
/// and marks as compatible with each other (checked by the service tests
/// where Bambu Studio is installed). Printers with two nozzles need a
/// filament-to-nozzle map on top and are not listed.
pub const PRESETS: &[Presets] = &[
    preset("bambu-lab-a1", "Bambu Lab A1", "@BBL A1", "@BBL A1"),
    preset("bambu-lab-a1-mini", "Bambu Lab A1 mini", "@BBL A1M", "@BBL A1M"),
    preset("bambu-lab-p1s", "Bambu Lab P1S", "@BBL X1C", "@BBL P1S 0.4 nozzle"),
    preset("bambu-lab-p2s", "Bambu Lab P2S", "@BBL P2S", "@BBL P2S"),
    preset("bambu-lab-x1", "Bambu Lab X1", "@BBL X1C", "@BBL X1"),
    preset("bambu-lab-x1-carbon", "Bambu Lab X1 Carbon", "@BBL X1C", "@BBL X1C"),
    preset("bambu-lab-x1e", "Bambu Lab X1E", "@BBL X1C", "@BBL X1C"),
    preset("bambu-lab-h2s", "Bambu Lab H2S", "@BBL H2S", "@BBL H2S"),
];

const fn preset(
    id: &'static str,
    printer: &'static str,
    process: &'static str,
    filament: &'static str,
) -> Presets {
    Presets { id, printer, process, filament }
}

impl Presets {
    pub fn by_id(id: &str) -> Option<&'static Presets> {
        PRESETS.iter().find(|p| p.id == id)
    }

    /// Name of the printer preset.
    pub fn printer_name(&self) -> String {
        format!("{} 0.4 nozzle", self.printer)
    }

    /// Name of the process preset.
    pub fn process_name(&self) -> String {
        format!("0.20mm Standard {}", self.process)
    }

    /// Name of the filament preset.
    pub fn filament_name(&self) -> String {
        format!("Bambu PLA Basic {}", self.filament)
    }
}

/// Gap between plates as a fraction of the bed, as Bambu Studio lays its
/// plates out in the world.
const PLATE_GAP: f32 = 0.2;

/// One mesh of an object and the filament (from 1) it prints in.
pub struct ProjectPart<'a> {
    pub name: String,
    pub mesh: &'a TriMesh,
    pub filament: usize,
}

/// One object on a plate. Its parts share one frame: x and y around the
/// object's centre, z from the bed.
pub struct ProjectObject<'a> {
    pub name: String,
    pub parts: Vec<ProjectPart<'a>>,
    /// Index of the plate, from 0.
    pub plate: usize,
    /// Where the object's centre lies, from the plate's front left corner.
    pub centre: [f32; 2],
}

/// A kit laid out on plates.
pub struct Project<'a> {
    pub objects: Vec<ProjectObject<'a>>,
    /// The colour (sRGB) of each filament, in slot order.
    pub filaments: Vec<[u8; 3]>,
    /// Width and depth of the bed the plates were laid out for, mm.
    pub bed: [f32; 2],
    /// Largest print height of the printer, mm.
    pub height: f32,
    /// The printer the project is for.
    pub presets: &'a Presets,
}

impl Project<'_> {
    /// Number of plates: up to the last one that holds an object.
    pub fn plate_count(&self) -> usize {
        self.objects.iter().map(|o| o.plate + 1).max().unwrap_or(1)
    }
}

/// Where the front left corner of plate `index` of `count` lies in the
/// world: Bambu Studio lays plates out in rows of `ceil(sqrt(count))`,
/// to the right and then towards the front.
pub fn plate_origin(index: usize, count: usize, bed: [f32; 2]) -> [f32; 2] {
    let columns = (1..).find(|c| c * c >= count.max(1)).unwrap_or(1);
    let (row, column) = (index / columns, index % columns);
    let stride = bed.map(|side| side + side * PLATE_GAP);
    [column as f32 * stride[0], -(row as f32) * stride[1]]
}

const MODEL_HEAD: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    "\n",
    r#"<model unit="millimeter" xml:lang="en-US" "#,
    r#"xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" "#,
    r#"xmlns:BambuStudio="http://schemas.bambulab.com/package/2021" "#,
    r#"xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06" "#,
    r#"requiredextensions="p">"#,
    "\n"
);

const RELS_HEAD: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    "\n",
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    "\n"
);

const MODEL_TYPE: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

/// The project as a 3MF package. Ids: the parts of all objects are numbered
/// from 1 in order, each object follows its parts.
pub fn bambu_project(project: &Project<'_>) -> Vec<u8> {
    let plates = project.plate_count();
    let mut root = format!(
        "{MODEL_HEAD} <metadata name=\"Application\">BambuStudio-{WRITER_VERSION}</metadata>\n \
         <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <resources>\n"
    );
    let mut build = String::from(" <build>\n");
    let mut settings = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    let mut model_rels = String::from(RELS_HEAD);
    let mut plate_items: Vec<String> = vec![String::new(); plates];
    let mut object_files: Vec<(String, String)> = Vec::new();
    let mut next_id = 1usize;

    for (n, object) in project.objects.iter().enumerate() {
        let path = format!("3D/Objects/object_{}.model", n + 1);
        let mut file = format!(
            "{MODEL_HEAD} <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <resources>\n"
        );
        let first_part = next_id;
        let mut triangles = 0;
        for part in &object.parts {
            triangles += write_mesh(&mut file, next_id, part.mesh);
            next_id += 1;
        }
        file.push_str(" </resources>\n</model>\n");
        object_files.push((path.clone(), file));
        let id = next_id;
        next_id += 1;

        let _ = writeln!(root, "  <object id=\"{id}\" p:UUID=\"{}\" type=\"model\">", uuid(id, 1));
        root.push_str("   <components>\n");
        for (k, _) in object.parts.iter().enumerate() {
            let part = first_part + k;
            let _ = writeln!(
                root,
                "    <component p:path=\"/{path}\" objectid=\"{part}\" p:UUID=\"{}\" \
                 transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/>",
                uuid(part, 2)
            );
        }
        root.push_str("   </components>\n  </object>\n");

        let origin = plate_origin(object.plate, plates, project.bed);
        let _ = writeln!(
            build,
            "  <item objectid=\"{id}\" p:UUID=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 {} {} 0\" \
             printable=\"1\"/>",
            uuid(id, 3),
            origin[0] + object.centre[0],
            origin[1] + object.centre[1]
        );

        object_settings(&mut settings, id, first_part, triangles, object);
        let _ = write!(
            plate_items[object.plate],
            "    <model_instance>\n      <metadata key=\"object_id\" value=\"{id}\"/>\n      \
             <metadata key=\"instance_id\" value=\"0\"/>\n      \
             <metadata key=\"identify_id\" value=\"{}\"/>\n    </model_instance>\n",
            1000 + id
        );
        let _ = writeln!(
            model_rels,
            " <Relationship Target=\"/{path}\" Id=\"rel-{}\" Type=\"{MODEL_TYPE}\"/>",
            n + 1
        );
    }
    root.push_str(" </resources>\n");
    root.push_str(&build);
    root.push_str(" </build>\n</model>\n");
    model_rels.push_str("</Relationships>\n");
    for (i, items) in plate_items.iter().enumerate() {
        let _ = write!(
            settings,
            "  <plate>\n    <metadata key=\"plater_id\" value=\"{}\"/>\n    \
             <metadata key=\"plater_name\" value=\"\"/>\n    \
             <metadata key=\"locked\" value=\"false\"/>\n{items}  </plate>\n",
            i + 1
        );
    }
    settings.push_str("</config>\n");

    let settings_files = [
        ("3D/3dmodel.model", root),
        ("3D/_rels/3dmodel.model.rels", model_rels),
        ("Metadata/model_settings.config", settings),
        ("Metadata/project_settings.config", project_settings(project)),
    ];
    package(&settings_files, &object_files)
}

/// The entry of one object in `model_settings.config`: its name, and the
/// filament of each part. The object's own filament is the first part's:
/// Bambu Studio drops the part filament of a single-part object.
fn object_settings(
    out: &mut String,
    id: usize,
    first_part: usize,
    triangles: usize,
    object: &ProjectObject<'_>,
) {
    let name = xml_escape(&object.name);
    let filament = object.parts.first().map_or(1, |p| p.filament);
    let _ = write!(
        out,
        "  <object id=\"{id}\">\n    <metadata key=\"name\" value=\"{name}\"/>\n    \
         <metadata key=\"extruder\" value=\"{filament}\"/>\n    \
         <metadata face_count=\"{triangles}\"/>\n"
    );
    for (k, part) in object.parts.iter().enumerate() {
        let _ = write!(
            out,
            "    <part id=\"{}\" subtype=\"normal_part\">\n      \
             <metadata key=\"name\" value=\"{}\"/>\n      \
             <metadata key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\n      \
             <metadata key=\"extruder\" value=\"{}\"/>\n    </part>\n",
            first_part + k,
            xml_escape(&part.name),
            part.filament
        );
    }
    out.push_str("  </object>\n");
}

/// The ZIP package: the fixed entries, `named` and the model file of each
/// object.
fn package(named: &[(&str, String)], object_files: &[(String, String)]) -> Vec<u8> {
    let content_types = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
        "</Types>\n"
    );
    let rels = format!(
        "{RELS_HEAD} <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\" \
         Type=\"{MODEL_TYPE}\"/>\n</Relationships>\n"
    );
    let mut files: Vec<(&str, &[u8])> =
        vec![("[Content_Types].xml", content_types.as_bytes()), ("_rels/.rels", rels.as_bytes())];
    files.extend(named.iter().map(|(path, text)| (*path, text.as_bytes())));
    files.extend(object_files.iter().map(|(path, text)| (path.as_str(), text.as_bytes())));
    zip_store(&files)
}

/// The welded mesh as a 3MF object; the number of triangles written.
fn write_mesh(out: &mut String, id: usize, mesh: &TriMesh) -> usize {
    let (positions, triangles) = mesh.welded();
    let _ = writeln!(out, "  <object id=\"{id}\" p:UUID=\"{}\" type=\"model\">", uuid(id, 0));
    out.push_str("   <mesh>\n    <vertices>\n");
    for p in &positions {
        let _ = writeln!(out, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", p[0], p[1], p[2]);
    }
    out.push_str("    </vertices>\n    <triangles>\n");
    for t in &triangles {
        let _ = writeln!(out, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]);
    }
    out.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    triangles.len()
}

/// A stable UUID for the element `id` of kind `kind`: the same kit gives
/// the same bytes.
fn uuid(id: usize, kind: u8) -> String {
    format!("{id:08x}-71cb-4c03-9d28-{kind:02x}fed5dfa1dc")
}

/// The project settings: the presets by name, and one filament per colour.
/// Bambu Studio counts the filaments by the colours and wants every
/// per-filament list as long. Nozzle, bed and height are not taken from
/// here by the app (a preset's own values win), but its command line needs
/// them, and lays the plates out by this bed. All values are strings.
fn project_settings(project: &Project<'_>) -> String {
    let n = project.filaments.len().max(1);
    let each = |item: &dyn Fn(usize) -> String| -> Vec<String> { (0..n).map(item).collect() };
    let color = |i: usize| {
        let [r, g, b] = project.filaments.get(i).copied().unwrap_or([0xC8; 3]);
        format!("#{r:02X}{g:02X}{b:02X}")
    };
    let presets = project.presets;
    let [w, d] = project.bed;
    let settings = serde_json::json!({
        "name": "project_settings",
        "from": "project",
        "version": WRITER_VERSION,
        "printer_settings_id": presets.printer_name(),
        "print_settings_id": presets.process_name(),
        "filament_settings_id": each(&|_| presets.filament_name()),
        "filament_colour": each(&color),
        "filament_diameter": each(&|_| "1.75".to_owned()),
        "filament_self_index": each(&|i| (i + 1).to_string()),
        "filament_extruder_variant": each(&|_| "Direct Drive Standard".to_owned()),
        "nozzle_diameter": ["0.4"],
        "printable_area": ["0x0".to_owned(), format!("{w}x0"), format!("{w}x{d}"), format!("0x{d}")],
        "printable_height": project.height.to_string(),
    });
    format!("{settings:#}\n")
}
