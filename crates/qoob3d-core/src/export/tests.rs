use super::*;

fn tetra() -> TriMesh {
    // Duplicated vertices per face, like kernel meshes.
    let p = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]];
    let faces = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]];
    let mut m = TriMesh::default();
    for f in faces {
        for i in f {
            m.indices.push(m.positions.len() as u32);
            m.positions.push(p[i]);
            m.normals.push([0.0; 3]);
        }
    }
    m
}

#[test]
fn crc_matches_the_reference_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn stl_has_header_count_and_50_bytes_per_triangle() {
    let m = tetra();
    let bytes = stl(&[ExportObject { name: "a".into(), mesh: &m }]);
    assert_eq!(bytes.len(), 84 + 4 * 50);
    assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 4);
}

#[test]
fn three_mf_is_a_valid_package_with_shared_vertices() {
    let m = tetra();
    let bytes = three_mf(&[ExportObject { name: "Body <1>".into(), mesh: &m }]);
    let files = unzip_stored(&bytes).unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["[Content_Types].xml", "_rels/.rels", "3D/3dmodel.model"]);
    let model = String::from_utf8(files[2].1.clone()).unwrap();
    assert_eq!(model.matches("<vertex ").count(), 4, "welded: 4 corners");
    assert_eq!(model.matches("<triangle ").count(), 4);
    assert!(model.contains("unit=\"millimeter\""));
    assert!(model.contains("name=\"Body &lt;1&gt;\""), "names are escaped");
}

// ---- AG-SV-10: a kit as a Bambu Studio project

use super::bambu::{
    PRESETS, Presets, Project, ProjectObject, ProjectPart, bambu_project, plate_origin,
};

fn file<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
    let bytes = &files.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}")).1;
    std::str::from_utf8(bytes).expect("utf-8")
}

/// A two-colour object on plate 1 and a single one on plate 2.
fn two_plates(mesh: &TriMesh) -> Project<'_> {
    let part = |name: &str, filament| ProjectPart { name: name.into(), mesh, filament };
    Project {
        objects: vec![
            ProjectObject {
                name: "House & <garden>".into(),
                parts: vec![part("Walls", 1), part("Windows", 2)],
                plate: 0,
                centre: [20.0, 30.0],
            },
            ProjectObject {
                name: "Roof".into(),
                parts: vec![part("Roof", 3)],
                plate: 1,
                centre: [40.0, 50.0],
            },
        ],
        filaments: vec![[0xF4, 0xF4, 0xF4], [0x20, 0x20, 0x20], [0xC0, 0x39, 0x2B]],
        bed: [256.0, 256.0],
        height: 250.0,
        presets: Presets::by_id("bambu-lab-p1s").expect("the P1S"),
    }
}

#[test]
fn plates_lie_in_rows_of_the_square_root_of_their_number() {
    let bed = [256.0, 200.0];
    assert_eq!(plate_origin(0, 1, bed), [0.0, 0.0]);
    // Two plates: two columns. Five: three columns, the fourth starts row 2.
    assert_eq!(plate_origin(1, 2, bed), [307.2, 0.0]);
    assert_eq!(plate_origin(2, 5, bed), [614.4, 0.0]);
    assert_eq!(plate_origin(3, 5, bed), [0.0, -240.0]);
    assert_eq!(plate_origin(2, 4, bed), [0.0, -240.0], "four plates: two columns");
}

#[test]
fn a_bambu_project_names_plates_parts_and_filaments() {
    let mesh = tetra();
    let project = two_plates(&mesh);
    let files = unzip_stored(&bambu_project(&project)).unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for wanted in [
        "[Content_Types].xml",
        "_rels/.rels",
        "3D/3dmodel.model",
        "3D/_rels/3dmodel.model.rels",
        "3D/Objects/object_1.model",
        "3D/Objects/object_2.model",
        "Metadata/model_settings.config",
        "Metadata/project_settings.config",
    ] {
        assert!(names.contains(&wanted), "{wanted} is missing: {names:?}");
    }

    // Bambu Studio reads plates and filaments only from its own files.
    let root = file(&files, "3D/3dmodel.model");
    assert!(root.contains("<metadata name=\"Application\">BambuStudio-"), "{root}");
    // Parts 1 and 2 form object 3, part 4 forms object 5.
    assert!(root.contains("<object id=\"3\""), "{root}");
    assert!(root.contains("p:path=\"/3D/Objects/object_1.model\" objectid=\"2\""), "{root}");
    assert!(root.contains("<object id=\"5\""), "{root}");
    // Object 1 at its centre on plate 1, object 2 on plate 2 (one bed and a fifth further).
    assert!(root.contains("objectid=\"3\"") && root.contains("0 0 1 20 30 0\""), "{root}");
    assert!(root.contains("0 0 1 347.2 50 0\""), "{root}");

    let first = file(&files, "3D/Objects/object_1.model");
    assert_eq!(first.matches("<object ").count(), 2, "both parts of the object");
    assert_eq!(first.matches("<vertex ").count(), 8, "welded: 4 corners per part");

    let settings = file(&files, "Metadata/model_settings.config");
    assert!(settings.contains("value=\"House &amp; &lt;garden&gt;\""), "names are escaped");
    assert_eq!(settings.matches("<plate>").count(), 2, "{settings}");
    assert_eq!(settings.matches("<model_instance>").count(), 2, "{settings}");
    for (part, filament) in [(1, 1), (2, 2), (4, 3)] {
        let at = settings.find(&format!("<part id=\"{part}\"")).expect("the part");
        let rest = &settings[at..];
        let end = rest.find("</part>").expect("its end");
        assert!(
            rest[..end].contains(&format!("key=\"extruder\" value=\"{filament}\"")),
            "part {part} prints in filament {filament}: {settings}"
        );
    }
}

/// The independent reader: our own 3MF import follows the components and
/// item transforms of the production extension, as slicers do.
#[test]
fn a_bambu_project_reads_back_as_all_parts_in_their_places() {
    let mesh = tetra();
    let read = crate::import::read_3mf(&bambu_project(&two_plates(&mesh))).expect("readable");
    assert_eq!(read.triangles.len(), 3 * 4, "three parts of four triangles");
    let xs = read.positions.iter().map(|p| p[0]);
    let (lo, hi) =
        xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
    // The tetrahedron spans x 0…10 around each object's centre.
    assert!((lo - 20.0).abs() < 1e-6 && (hi - 357.2).abs() < 1e-3, "x from {lo} to {hi}");
}

/// Bambu Studio makes plates only with project settings; they name its own
/// presets and hold one entry per filament in every per-filament list.
#[test]
fn a_bambu_project_names_the_presets_of_its_printer_and_its_filament_colours() {
    let mesh = tetra();
    let files = unzip_stored(&bambu_project(&two_plates(&mesh))).unwrap();
    let settings: serde_json::Value =
        serde_json::from_str(file(&files, "Metadata/project_settings.config")).expect("JSON");
    assert_eq!(settings["printer_settings_id"], "Bambu Lab P1S 0.4 nozzle");
    assert_eq!(settings["print_settings_id"], "0.20mm Standard @BBL X1C");
    assert_eq!(settings["filament_colour"], serde_json::json!(["#F4F4F4", "#202020", "#C0392B"]));
    assert_eq!(settings["filament_self_index"], serde_json::json!(["1", "2", "3"]));
    for key in ["filament_settings_id", "filament_diameter", "filament_extruder_variant"] {
        assert_eq!(settings[key].as_array().map(Vec::len), Some(3), "{key}: {settings}");
    }
    assert_eq!(settings["filament_settings_id"][2], "Bambu PLA Basic @BBL P1S 0.4 nozzle");
    // No values of the presets themselves: the slicer's own apply.
    assert!(settings.get("different_settings_to_system").is_none(), "{settings}");
    assert_eq!(settings["printable_area"], serde_json::json!(["0x0", "256x0", "256x256", "0x256"]));
    assert_eq!(settings.as_object().map(serde_json::Map::len), Some(13), "{settings}");
}

#[test]
fn every_listed_bambu_printer_is_in_the_printer_database() {
    for presets in PRESETS {
        let printer = crate::printer::PrinterProfile::by_id(presets.id);
        assert_eq!(printer.map(|p| p.name), Some(presets.printer), "{}", presets.id);
    }
}
