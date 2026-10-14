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
