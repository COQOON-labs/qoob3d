//! Mesh export for slicers (ADR-0034): binary STL and 3MF. Pure Rust,
//! no dependencies; 3MF uses a minimal stored (uncompressed) ZIP writer.
//! The ZIP reader here is the only one: the 3MF import and `unzip_stored`
//! share it.

pub mod bambu;

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

/// One entry of a ZIP archive as its central directory describes it, with
/// the bytes as stored (compressed or not).
pub(crate) struct ZipEntry<'a> {
    pub(crate) name: String,
    /// 0 stored, 8 deflate.
    pub(crate) method: u16,
    pub(crate) data: &'a [u8],
    pub(crate) crc: u32,
    /// Size of the uncompressed data.
    pub(crate) size: u32,
}

/// The entries of a ZIP archive, read through the end record and the
/// central directory (APPNOTE 4.3.12, 4.3.16), each checked against its
/// local header. Slices only: nothing is allocated for a claimed size.
pub(crate) fn zip_entries(bytes: &[u8]) -> Result<Vec<ZipEntry<'_>>, String> {
    let u16_at = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at =
        |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    // The end record is the last 22 bytes plus its comment.
    let end = (0..bytes.len().saturating_sub(21))
        .rev()
        .take(22 + usize::from(u16::MAX))
        .find(|&i| {
            u32_at(i) == Some(0x0605_4b50)
                && u16_at(i + 20).is_some_and(|c| i + 22 + usize::from(c) == bytes.len())
        })
        .ok_or("no ZIP end of central directory record")?;
    let count = u16_at(end + 10).ok_or("broken end record")?;
    let mut at = u32_at(end + 16).ok_or("broken end record")? as usize;
    let mut entries = Vec::new();
    for n in 0..count {
        let bad = || format!("central directory entry {n} is broken");
        if u32_at(at) != Some(0x0201_4b50) {
            return Err(bad());
        }
        let field = |offset: usize| u16_at(at + offset).map(usize::from).ok_or_else(bad);
        let (flags, method) = (field(8)?, field(10)?);
        let crc = u32_at(at + 16).ok_or_else(bad)?;
        let packed = u32_at(at + 20).ok_or_else(bad)?;
        let size = u32_at(at + 24).ok_or_else(bad)?;
        let (name_len, extra, comment) = (field(28)?, field(30)?, field(32)?);
        let local = u32_at(at + 42).ok_or_else(bad)? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(name).into_owned();
        if flags & 1 != 0 {
            return Err(format!("{name} is encrypted"));
        }
        if packed == u32::MAX || size == u32::MAX {
            return Err(format!("{name} needs ZIP64, which is not supported"));
        }
        let data = local_data(bytes, local, &name, flags & 8 != 0, (crc, packed, size))?;
        entries.push(ZipEntry {
            name,
            method: u16::try_from(method).map_err(|_| bad())?,
            data,
            crc,
            size,
        });
        at += 46 + name_len + extra + comment;
    }
    Ok(entries)
}

/// The stored bytes of the entry `name` whose local header is at `at`; the
/// header must repeat name and, without a data descriptor, CRC and sizes.
fn local_data<'a>(
    bytes: &'a [u8],
    at: usize,
    name: &str,
    descriptor: bool,
    (crc, packed, size): (u32, u32, u32),
) -> Result<&'a [u8], String> {
    let u16_at =
        |i: usize| bytes.get(i..i + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let u32_at =
        |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let bad = || format!("the local header of {name} is broken");
    if u32_at(at) != Some(0x0403_4b50) {
        return Err(bad());
    }
    let name_len = u16_at(at + 26).ok_or_else(bad)?;
    let extra = u16_at(at + 28).ok_or_else(bad)?;
    if bytes.get(at + 30..at + 30 + name_len) != Some(name.as_bytes()) {
        return Err(bad());
    }
    let repeated = [u32_at(at + 14), u32_at(at + 18), u32_at(at + 22)];
    if !descriptor && repeated != [Some(crc), Some(packed), Some(size)] {
        return Err(bad());
    }
    let start = at + 30 + name_len + extra;
    bytes.get(start..start + packed as usize).ok_or_else(|| format!("{name} is truncated"))
}

/// Reads a stored ZIP back (tests and `cargo xtask models`): (name, data).
pub fn unzip_stored(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    zip_entries(bytes)?
        .into_iter()
        .map(|e| {
            if e.method != 0 {
                return Err(format!("{}: compressed entries are not supported", e.name));
            }
            if crc32(e.data) != e.crc {
                return Err(format!("CRC mismatch in {}", e.name));
            }
            Ok((e.name, e.data.to_vec()))
        })
        .collect()
}

#[cfg(test)]
mod tests;
