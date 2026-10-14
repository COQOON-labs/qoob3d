//! Mesh export for slicers (ADR-0034): binary STL and 3MF. Pure Rust,
//! no dependencies; 3MF uses a minimal stored (uncompressed) ZIP writer.

use crate::mesh::TriMesh;

/// One printable object: name and mesh (mm).
pub struct ExportObject<'a> {
    pub name: String,
    pub mesh: &'a TriMesh,
}

/// Binary STL with all objects merged (STL has no notion of objects).
pub fn stl(objects: &[ExportObject<'_>]) -> Vec<u8> {
    let triangles: Vec<[[f32; 3]; 3]> = objects
        .iter()
        .flat_map(|o| {
            let (pos, tris) = o.mesh.welded();
            tris.into_iter().map(move |t| t.map(|i| pos[i as usize]))
        })
        .collect();
    let mut out = Vec::with_capacity(84 + 50 * triangles.len());
    let mut header = [0u8; 80];
    let tag = b"QOOB3D binary STL, units mm";
    header[..tag.len()].copy_from_slice(tag);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for [a, b, c] in triangles {
        let n = normal(a, b, c);
        for v in [n, a, b, c] {
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

fn normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 { n.map(|x| x / len) } else { [0.0, 0.0, 0.0] }
}

/// 3MF package (core spec): one object per body, all on the build plate.
pub fn three_mf(objects: &[ExportObject<'_>]) -> Vec<u8> {
    let mut model = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">"#,
        "\n",
        r#"<metadata name="Application">QOOB3D</metadata>"#,
        "\n<resources>\n"
    ));
    for (i, o) in objects.iter().enumerate() {
        let (pos, tris) = o.mesh.welded();
        model.push_str(&format!(
            "<object id=\"{}\" name=\"{}\" type=\"model\"><mesh><vertices>\n",
            i + 1,
            xml_escape(&o.name)
        ));
        for p in pos {
            model.push_str(&format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", p[0], p[1], p[2]));
        }
        model.push_str("</vertices><triangles>\n");
        for t in tris {
            model.push_str(&format!(
                "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n",
                t[0], t[1], t[2]
            ));
        }
        model.push_str("</triangles></mesh></object>\n");
    }
    model.push_str("</resources>\n<build>\n");
    for i in 0..objects.len() {
        model.push_str(&format!("<item objectid=\"{}\"/>\n", i + 1));
    }
    model.push_str("</build>\n</model>\n");

    let content_types = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
        "</Types>\n"
    );
    let rels = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
        "</Relationships>\n"
    );
    zip_store(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ])
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// CRC-32 (IEEE), as used by ZIP.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Minimal ZIP archive with stored (uncompressed) entries.
pub fn zip_store(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name_bytes = name.as_bytes();
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);
        // Central directory entry.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes()); // needed
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra len, comment len, disk, internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let central_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // disk numbers
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// Reads a stored ZIP back (tests and `cargo xtask models`): (name, data).
pub fn unzip_stored(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at =
        |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut files = Vec::new();
    let mut i = 0;
    while u32_at(i) == Some(0x0403_4b50) {
        let bad = || format!("truncated entry at {i}");
        if u16_at(i + 8).ok_or_else(bad)? != 0 {
            return Err("compressed entries are not supported".into());
        }
        let crc = u32_at(i + 14).ok_or_else(bad)?;
        let size = u32_at(i + 18).ok_or_else(bad)? as usize;
        let name_len = u16_at(i + 26).ok_or_else(bad)? as usize;
        let extra = u16_at(i + 28).ok_or_else(bad)? as usize;
        let name_start = i + 30;
        let data_start = name_start + name_len + extra;
        let name = bytes.get(name_start..name_start + name_len).ok_or_else(bad)?;
        let data = bytes.get(data_start..data_start + size).ok_or_else(bad)?;
        if crc32(data) != crc {
            return Err(format!("CRC mismatch in {}", String::from_utf8_lossy(name)));
        }
        files.push((String::from_utf8_lossy(name).into_owned(), data.to_vec()));
        i = data_start + size;
    }
    // Central directory: one entry per file, pointing at its local header.
    let mut offset = 0usize;
    for (n, (name, data)) in files.iter().enumerate() {
        let bad = || format!("central directory entry {n} is broken");
        if u32_at(i) != Some(0x0201_4b50) {
            return Err(bad());
        }
        let name_len = u16_at(i + 28).ok_or_else(bad)? as usize;
        let extra = u16_at(i + 30).ok_or_else(bad)? as usize;
        let comment = u16_at(i + 32).ok_or_else(bad)? as usize;
        let local = u32_at(i + 42).ok_or_else(bad)? as usize;
        let central_name = bytes.get(i + 46..i + 46 + name_len).ok_or_else(bad)?;
        if central_name != name.as_bytes() || local != offset {
            return Err(bad());
        }
        offset += 30 + name.len() + data.len();
        i += 46 + name_len + extra + comment;
    }
    if u32_at(i) != Some(0x0605_4b50) || u16_at(i + 10) != Some(files.len() as u16) {
        return Err("end of central directory is broken".into());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
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
}
