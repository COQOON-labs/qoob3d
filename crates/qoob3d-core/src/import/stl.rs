//! STL reader (AG-SC-23): binary (80-byte header, u32 count, 50 bytes per
//! triangle) and ASCII (`solid … facet normal … outer loop vertex ×3
//! endloop endfacet … endsolid`). The winding gives the orientation; the
//! stored normals are ignored.

use super::{ImportError, ImportedMesh, MAX_TRIANGLES, Welder, unreadable, within_limit};

/// Header and count of a binary STL.
const HEADER: usize = 84;
/// Normal, three corners (f32) and the attribute word.
const RECORD: usize = 50;

/// Reads a binary or ASCII STL.
///
/// # Errors
/// The bytes are no STL, are truncated, or hold no or more than
/// [`MAX_TRIANGLES`] triangles.
pub fn read_stl(bytes: &[u8]) -> Result<ImportedMesh, ImportError> {
    if bytes.is_empty() {
        return Err(unreadable("the file is empty"));
    }
    let count = bytes.get(80..HEADER).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
    let exact =
        count.is_some_and(|n| HEADER as u64 + RECORD as u64 * u64::from(n) == bytes.len() as u64);
    // Many binary headers start with "solid" too: an exact size says binary.
    let ascii = bytes.trim_ascii_start().starts_with(b"solid") && !exact;
    match count {
        _ if ascii => ascii_stl(bytes),
        Some(n) => binary_stl(bytes, n),
        None => Err(unreadable(format!(
            "{} bytes are neither a binary STL (at least {HEADER} bytes) nor an ASCII STL \
             (starting with \"solid\")",
            bytes.len()
        ))),
    }
}

fn binary_stl(bytes: &[u8], count: u32) -> Result<ImportedMesh, ImportError> {
    let records = (bytes.len() - HEADER) / RECORD;
    if (count as usize) > records {
        return Err(unreadable(format!(
            "the binary STL claims {count} triangles but holds bytes for {records}"
        )));
    }
    within_limit(u64::from(count))?;
    let mut welder = Welder::default();
    for record in bytes[HEADER..].as_chunks::<RECORD>().0.iter().take(count as usize) {
        let coordinate = |k: usize| {
            let at = 12 + 4 * k;
            f64::from(f32::from_le_bytes([
                record[at],
                record[at + 1],
                record[at + 2],
                record[at + 3],
            ]))
        };
        welder.triangle(std::array::from_fn(|v| std::array::from_fn(|c| coordinate(3 * v + c))))?;
    }
    welder.finish()
}

/// The next word must be `keyword`.
fn expect<'a>(
    words: &mut impl Iterator<Item = &'a [u8]>,
    keyword: &str,
    facet: u64,
) -> Result<(), ImportError> {
    match words.next() {
        Some(w) if w == keyword.as_bytes() => Ok(()),
        found => {
            let found = found.map_or_else(
                || "the end of the file".to_owned(),
                |w| format!("{:?}", String::from_utf8_lossy(w)),
            );
            Err(unreadable(format!(
                "ASCII STL, facet {facet}: expected \"{keyword}\", found {found}"
            )))
        }
    }
}

fn number<'a>(words: &mut impl Iterator<Item = &'a [u8]>, facet: u64) -> Result<f64, ImportError> {
    let word = words.next().unwrap_or_default();
    std::str::from_utf8(word).ok().and_then(|w| w.parse().ok()).ok_or_else(|| {
        unreadable(format!(
            "ASCII STL, facet {facet}: {:?} is not a number",
            String::from_utf8_lossy(word)
        ))
    })
}

/// One `facet … endfacet` after its keyword: the three corners.
fn facet<'a>(
    words: &mut impl Iterator<Item = &'a [u8]>,
    facet: u64,
) -> Result<[[f64; 3]; 3], ImportError> {
    expect(words, "normal", facet)?;
    for _ in 0..3 {
        number(words, facet)?;
    }
    expect(words, "outer", facet)?;
    expect(words, "loop", facet)?;
    let mut corners = [[0.0; 3]; 3];
    for corner in &mut corners {
        expect(words, "vertex", facet)?;
        for c in corner.iter_mut() {
            *c = number(words, facet)?;
        }
    }
    expect(words, "endloop", facet)?;
    expect(words, "endfacet", facet)?;
    Ok(corners)
}

fn ascii_stl(bytes: &[u8]) -> Result<ImportedMesh, ImportError> {
    let mut words = bytes.split(u8::is_ascii_whitespace).filter(|w| !w.is_empty());
    let mut welder = Welder::default();
    let mut facets: u64 = 0;
    // `solid name` …: the name is any words up to the first facet.
    let mut in_solid = false;
    while let Some(word) = words.next() {
        match word {
            b"solid" if !in_solid => in_solid = true,
            b"endsolid" if in_solid => in_solid = false,
            b"facet" if in_solid => {
                facets += 1;
                let corners = facet(&mut words, facets)?;
                // Past the limit only count, to name the total.
                if facets <= MAX_TRIANGLES as u64 {
                    welder.triangle(corners)?;
                }
            }
            // Names of solids, and words after the last `endsolid`.
            _ => {}
        }
    }
    if in_solid {
        return Err(unreadable("ASCII STL: the file ends inside a solid (no \"endsolid\")"));
    }
    within_limit(facets)?;
    welder.finish()
}
