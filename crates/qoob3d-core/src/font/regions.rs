//! From the contours of glyphs to regions with holes (AG-SC-38): cleaning,
//! nesting inside a glyph, and the search for contours that cross or touch.
//! Plain polygon geometry in font units.

use super::{FontError, Region};

/// The contours of one character at its place in the line.
pub(super) struct Glyph {
    pub(super) character: char,
    pub(super) contours: Vec<Vec<[f64; 2]>>,
}

/// Twice the signed area of the triangle `a b c`: positive when it runs
/// counter-clockwise.
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Area of a closed polygon, positive when it runs counter-clockwise.
pub(super) fn signed_area(contour: &[[f64; 2]]) -> f64 {
    let n = contour.len();
    (0..n)
        .map(|i| {
            let (a, b) = (contour[i], contour[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// The contour without points closer than `min_edge` to the one before
/// (the last one to the first); `None` if no area is left.
pub(super) fn cleaned(contour: Vec<[f64; 2]>, min_edge: f64) -> Option<Vec<[f64; 2]>> {
    let apart = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) > min_edge;
    let mut kept: Vec<[f64; 2]> = Vec::with_capacity(contour.len());
    for p in contour {
        if kept.last().is_none_or(|last| apart(*last, p)) {
            kept.push(p);
        }
    }
    while kept.len() > 1 && !apart(kept[0], kept[kept.len() - 1]) {
        kept.pop();
    }
    let area = signed_area(&kept).abs();
    (kept.len() >= 3 && area > min_edge * min_edge && area.is_finite()).then_some(kept)
}

/// Whether `p` lies inside the polygon (even-odd; a point on the boundary
/// counts as either).
fn inside(p: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let n = polygon.len();
    let mut within = false;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            within ^= p[0] < x;
        }
    }
    within
}

/// Smallest and largest corner.
fn bounds<'a>(points: impl IntoIterator<Item = &'a [f64; 2]>) -> ([f64; 2], [f64; 2]) {
    points.into_iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(min, max), p| {
        ([min[0].min(p[0]), min[1].min(p[1])], [max[0].max(p[0]), max[1].max(p[1])])
    })
}

fn boxes_meet(a: &([f64; 2], [f64; 2]), b: &([f64; 2], [f64; 2])) -> bool {
    (0..2).all(|k| a.0[k] <= b.1[k] && b.0[k] <= a.1[k])
}

/// Whether most points of `inner` lie inside `outer`: all or none of them
/// when the two do not cross, and a contour that only touches the other at
/// some points is still on the right side.
fn holds(outer: &[[f64; 2]], inner: &[[f64; 2]]) -> bool {
    let (a, b) = (bounds(outer), bounds(inner));
    let boxed = (0..2).all(|k| a.0[k] <= b.0[k] && b.1[k] <= a.1[k]);
    boxed && 2 * inner.iter().filter(|p| inside(**p, outer)).count() > inner.len()
}

/// The regions of one glyph: a contour inside an even number of the others
/// is an outline, the others are holes of the smallest outline around them.
pub(super) fn regions(contours: Vec<Vec<[f64; 2]>>) -> Vec<Region> {
    let n = contours.len();
    let areas: Vec<f64> = contours.iter().map(|c| signed_area(c)).collect();
    let around: Vec<Vec<usize>> = (0..n)
        .map(|i| (0..n).filter(|&j| j != i && holds(&contours[j], &contours[i])).collect())
        .collect();
    let outline = |i: usize| around[i].len().is_multiple_of(2);
    // Only contours that cross can leave a hole without an outline around
    // it: then it is ink of its own, so that nothing is lost.
    let parent: Vec<Option<usize>> = (0..n)
        .map(|i| {
            let outlines = around[i].iter().copied().filter(|&j| outline(j));
            let smallest = outlines.min_by(|&a, &b| areas[a].abs().total_cmp(&areas[b].abs()));
            smallest.filter(|_| !outline(i))
        })
        .collect();
    let mut slot = vec![usize::MAX; n];
    let mut regions: Vec<Region> = Vec::new();
    for i in (0..n).filter(|&i| parent[i].is_none()) {
        slot[i] = regions.len();
        regions.push(Region { outline: Vec::new(), holes: Vec::new() });
    }
    for (i, mut contour) in contours.into_iter().enumerate() {
        // Outlines counter-clockwise, holes clockwise.
        if (areas[i] > 0.0) != parent[i].is_none() {
            contour.reverse();
        }
        let Some(region) = regions.get_mut(slot[parent[i].unwrap_or(i)]) else { continue };
        if parent[i].is_some() {
            region.holes.push(contour);
        } else {
            region.outline = contour;
        }
    }
    regions
}

/// One edge of a contour.
struct Edge {
    a: [f64; 2],
    b: [f64; 2],
    glyph: usize,
    contour: usize,
    /// Its number in the contour of `count` edges.
    index: usize,
    count: usize,
}

impl Edge {
    fn min(&self, k: usize) -> f64 {
        self.a[k].min(self.b[k])
    }

    fn max(&self, k: usize) -> f64 {
        self.a[k].max(self.b[k])
    }

    /// Whether the two edges share a point they must not share: edges that
    /// follow each other meet at their corner, unless one folds back onto
    /// the other.
    fn meets(&self, other: &Self) -> bool {
        if self.glyph == other.glyph && self.contour == other.contour {
            for (first, second) in [(self, other), (other, self)] {
                if (first.index + 1) % first.count == second.index {
                    let ahead = (first.b[0] - first.a[0]) * (second.b[0] - second.a[0])
                        + (first.b[1] - first.a[1]) * (second.b[1] - second.a[1]);
                    return orient(first.a, first.b, second.b) == 0.0 && ahead < 0.0;
                }
            }
        }
        intersect(self.a, self.b, other.a, other.b)
    }
}

/// Whether the closed segments `a b` and `c d` share a point.
fn intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (d1, d2, d3, d4) = (orient(c, d, a), orient(c, d, b), orient(a, b, c), orient(a, b, d));
    let opposite = |x: f64, y: f64| (x > 0.0 && y < 0.0) || (x < 0.0 && y > 0.0);
    if opposite(d1, d2) && opposite(d3, d4) {
        return true;
    }
    // On the line of the other segment: within its extent?
    let on = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (0..2).all(|k| p[k].min(q[k]) <= r[k] && r[k] <= p[k].max(q[k]))
    };
    (d1 == 0.0 && on(c, d, a))
        || (d2 == 0.0 && on(c, d, b))
        || (d3 == 0.0 && on(a, b, c))
        || (d4 == 0.0 && on(a, b, d))
}

/// Whether two different contours cross or touch.
///
/// A sweep along x over all edges: a line of text is long and its edges are
/// short, so each one meets few others.
///
/// # Errors
/// A contour crosses or touches itself.
fn crossings(glyphs: &[Glyph]) -> Result<bool, FontError> {
    let mut edges = Vec::new();
    for (glyph, g) in glyphs.iter().enumerate() {
        for (contour, points) in g.contours.iter().enumerate() {
            let count = points.len();
            edges.extend((0..count).map(|index| Edge {
                a: points[index],
                b: points[(index + 1) % count],
                glyph,
                contour,
                index,
                count,
            }));
        }
    }
    edges.sort_by(|s, t| s.min(0).total_cmp(&t.min(0)));
    let mut crossing = false;
    for (i, s) in edges.iter().enumerate() {
        let reach = edges[i + 1..].iter().take_while(|t| t.min(0) <= s.max(0));
        for t in reach.filter(|t| t.min(1) <= s.max(1) && s.min(1) <= t.max(1)) {
            if !s.meets(t) {
                continue;
            }
            if s.glyph == t.glyph && s.contour == t.contour {
                return Err(FontError::SelfCrossing(glyphs[s.glyph].character));
            }
            crossing = true;
        }
    }
    Ok(crossing)
}

/// Whether an outline of `a` starts in the ink of `b`: with contours that do
/// not cross, the whole outline lies there.
fn covered(a: &[Region], b: &[Region]) -> bool {
    a.iter()
        .filter_map(|r| r.outline.first())
        .any(|p| b.iter().any(|r| inside(*p, &r.outline) && !r.holes.iter().any(|h| inside(*p, h))))
}

/// The regions of all glyphs, and whether some of them touch or overlap.
///
/// # Errors
/// A contour crosses or touches itself.
pub(super) fn assemble(glyphs: Vec<Glyph>) -> Result<(Vec<Region>, bool), FontError> {
    let crossing = crossings(&glyphs)?;
    let inks: Vec<Vec<Region>> = glyphs.into_iter().map(|g| regions(g.contours)).collect();
    // Without crossings ink still overlaps where one glyph lies in another.
    let boxes: Vec<_> =
        inks.iter().map(|ink| bounds(ink.iter().flat_map(|r| &r.outline))).collect();
    let nested = || {
        (0..inks.len()).any(|i| {
            (0..i)
                .filter(|&j| boxes_meet(&boxes[i], &boxes[j]))
                .any(|j| covered(&inks[i], &inks[j]) || covered(&inks[j], &inks[i]))
        })
    };
    let overlapping = crossing || nested();
    Ok((inks.concat(), overlapping))
}
