//! Colours and plates of a kit (AG-PJ-07, AG-SV-10): which filament a part
//! prints in, and how the parts of a kit are spread over the plates of a
//! slicer project.

use super::pose::ARRANGE_GAP_MM;

/// The colour names scripts may use instead of `#RRGGBB`: the sixteen basic
/// web colours, `orange` and `grey`.
pub const COLOR_NAMES: &[(&str, [u8; 3])] = &[
    ("black", [0x00, 0x00, 0x00]),
    ("silver", [0xC0, 0xC0, 0xC0]),
    ("gray", [0x80, 0x80, 0x80]),
    ("grey", [0x80, 0x80, 0x80]),
    ("white", [0xFF, 0xFF, 0xFF]),
    ("maroon", [0x80, 0x00, 0x00]),
    ("red", [0xFF, 0x00, 0x00]),
    ("orange", [0xFF, 0xA5, 0x00]),
    ("purple", [0x80, 0x00, 0x80]),
    ("fuchsia", [0xFF, 0x00, 0xFF]),
    ("green", [0x00, 0x80, 0x00]),
    ("lime", [0x00, 0xFF, 0x00]),
    ("olive", [0x80, 0x80, 0x00]),
    ("yellow", [0xFF, 0xFF, 0x00]),
    ("navy", [0x00, 0x00, 0x80]),
    ("blue", [0x00, 0x00, 0xFF]),
    ("teal", [0x00, 0x80, 0x80]),
    ("aqua", [0x00, 0xFF, 0xFF]),
];

/// `#RRGGBB` or one of [`COLOR_NAMES`] (any case) as bytes; `None` for
/// anything else.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    if let Some((_, rgb)) = COLOR_NAMES.iter().find(|(name, _)| name.eq_ignore_ascii_case(text)) {
        return Some(*rgb);
    }
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Where one footprint of a kit lies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Index of the plate, from 0.
    pub plate: usize,
    /// Centre of the footprint from the plate's front left corner, mm.
    pub centre: [f32; 2],
    /// False if the footprint is larger than the bed: it lies alone on its
    /// plate, centred, and has to be split or turned by the user.
    pub fits: bool,
}

/// One row of footprints on a plate.
struct Shelf {
    y: f32,
    depth: f32,
    x: f32,
}

struct Plate {
    group: usize,
    shelves: Vec<Shelf>,
    /// An oversized footprint has the plate for itself.
    full: bool,
}

/// Spreads footprints `sizes` (x, y extents, mm) over plates of `bed` mm
/// (width, depth), with [`ARRANGE_GAP_MM`] between footprints and to the
/// bed's edge. Footprints of different `groups` never share a plate (one
/// group per filament gives plates that print in one colour); plates follow
/// the order in which the groups first appear. Within a group the deepest
/// footprints are placed first, in rows from the front left corner.
/// `corner` (width, depth) is a zone at the front left corner that stays
/// empty, as printers have it for purging; `[0, 0]` for none.
pub fn plates(
    sizes: &[[f32; 2]],
    groups: &[usize],
    bed: [f32; 2],
    corner: [f32; 2],
) -> Vec<Placement> {
    let gap = ARRANGE_GAP_MM;
    let usable = [bed[0] - 2.0 * gap, bed[1] - 2.0 * gap];
    let mut group_order: Vec<usize> = Vec::new();
    for &g in groups {
        if !group_order.contains(&g) {
            group_order.push(g);
        }
    }
    let rank = |g: usize| group_order.iter().position(|&o| o == g).unwrap_or(usize::MAX);
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|&a, &b| {
        rank(groups[a]).cmp(&rank(groups[b])).then(sizes[b][1].total_cmp(&sizes[a][1]))
    });

    let mut plates: Vec<Plate> = Vec::new();
    let mut placed = vec![Placement { plate: 0, centre: [0.0; 2], fits: true }; sizes.len()];
    for i in order {
        let [w, d] = sizes[i];
        let spot = plates
            .iter_mut()
            .enumerate()
            .filter(|(_, p)| p.group == groups[i] && !p.full)
            .find_map(|(n, p)| place(p, w, d, usable, corner).map(|c| (n, c)));
        let spot = spot.or_else(|| {
            let mut plate = Plate { group: groups[i], shelves: Vec::new(), full: false };
            let at = place(&mut plate, w, d, usable, corner)?;
            plates.push(plate);
            Some((plates.len() - 1, at))
        });
        placed[i] = if let Some((plate, at)) = spot {
            Placement { plate, centre: [gap + at[0] + w / 2.0, gap + at[1] + d / 2.0], fits: true }
        } else {
            // No room even on an empty plate.
            plates.push(Plate { group: groups[i], shelves: Vec::new(), full: true });
            Placement { plate: plates.len() - 1, centre: [bed[0] / 2.0, bed[1] / 2.0], fits: false }
        };
    }
    placed
}

/// The front left corner for a footprint `w` × `d` on `plate`, in an
/// existing row or a new one; `None` if the plate has no room. Rows that
/// pass the empty `corner` zone start beside it; a footprint too wide for
/// that starts a row behind the zone.
fn place(
    plate: &mut Plate,
    w: f32,
    d: f32,
    usable: [f32; 2],
    corner: [f32; 2],
) -> Option<[f32; 2]> {
    let gap = ARRANGE_GAP_MM;
    for shelf in &mut plate.shelves {
        if d <= shelf.depth && shelf.x + w <= usable[0] {
            let at = [shelf.x, shelf.y];
            shelf.x += w + gap;
            return Some(at);
        }
    }
    let mut y = plate.shelves.last().map_or(0.0, |s| s.y + s.depth + gap);
    let mut x = if y < corner[1] { corner[0] } else { 0.0 };
    if x + w > usable[0] {
        (x, y) = (0.0, corner[1]);
    }
    if x + w > usable[0] || y + d > usable[1] {
        return None;
    }
    plate.shelves.push(Shelf { y, depth: d, x: x + w + gap });
    Some([x, y])
}

#[cfg(test)]
mod tests;
