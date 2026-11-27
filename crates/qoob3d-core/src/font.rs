//! Lettering (AG-SC-38): one line of text set in a TrueType/OpenType font,
//! as polygons in mm. The baseline is y = 0, the text runs along +x.
//!
//! The size is the cap height: the top of the outline of "H" above the
//! baseline, else the font's capital height metric, else its ascender.
//! Every character is one glyph (no ligatures, no shaping, no right-to-left
//! text); pairs are kerned with the font's `kern` table, GPOS kerning is not
//! read. Curves are flattened to polylines within [`FLATNESS`] × cap height
//! with at most [`MAX_CURVE_SEGMENTS`] segments each, so the same text has
//! the same polygons at every size.
//!
//! Which contour is a hole follows from nesting inside its glyph (a contour
//! inside an odd number of others), not from the winding the font stores.
//! Glyphs are separate pieces of ink: where two of them (or two contours of
//! one glyph) touch or overlap, [`Lettering::overlapping`] says so and the
//! caller unites the regions.
//!
//! Fonts are foreign bytes: nothing here panics, and the limits
//! [`MAX_FONT_BYTES`], [`MAX_CHARS`], [`MAX_GLYPH_POINTS`] and [`MAX_POINTS`]
//! bound what is allocated.

mod regions;
#[cfg(test)]
mod tests;

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use regions::Glyph;

/// Largest font file that is read.
pub const MAX_FONT_BYTES: usize = 16 * 1024 * 1024;
/// Most characters of one text.
pub const MAX_CHARS: usize = 200;
/// Most points of one glyph after flattening.
pub const MAX_GLYPH_POINTS: usize = 4_000;
/// Most points of one text after flattening.
pub const MAX_POINTS: usize = 100_000;
/// Largest distance between a curve and its polyline, as a fraction of the
/// cap height.
pub const FLATNESS: f64 = 0.002;
/// Most line segments one curve segment of a glyph becomes.
pub const MAX_CURVE_SEGMENTS: usize = 24;
/// Points closer than this are one point, mm: no edge is shorter.
const MIN_EDGE_MM: f64 = 1e-6;

/// Where the bounding box of the ink lies relative to x = 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// The left edge at x = 0.
    #[default]
    Left,
    /// The centre at x = 0.
    Center,
    /// The right edge at x = 0.
    Right,
}

/// How a text is set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setting {
    /// Cap height, mm.
    pub height: f64,
    /// Extra advance after every glyph, mm (negative tightens).
    pub spacing: f64,
    pub align: Align,
}

/// One piece of ink: `outline` runs counter-clockwise, `holes` clockwise,
/// points in mm. No contour crosses itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub outline: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
}

/// A set text.
#[derive(Debug, Clone, PartialEq)]
pub struct Lettering {
    /// In the order of the text; the regions of one glyph follow each other.
    pub regions: Vec<Region>,
    /// Some regions touch or overlap: the ink is their union, not their sum.
    pub overlapping: bool,
}

impl Lettering {
    /// Sum of the region areas, mm²: the ink area unless regions overlap.
    #[must_use]
    pub fn area(&self) -> f64 {
        let signed = |c: &Vec<[f64; 2]>| regions::signed_area(c);
        self.regions
            .iter()
            .map(|r| signed(&r.outline) + r.holes.iter().map(signed).sum::<f64>())
            .sum()
    }

    /// Smallest and largest corner of the ink, mm.
    #[must_use]
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let start = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        self.regions.iter().flat_map(|r| &r.outline).fold(start, |(min, max), p| {
            ([min[0].min(p[0]), min[1].min(p[1])], [max[0].max(p[0]), max[1].max(p[1])])
        })
    }
}

/// Why a text cannot be set.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FontError {
    #[error("the font file has {0} bytes, more than the limit of {MAX_FONT_BYTES}")]
    TooLarge(usize),
    #[error("not a TrueType or OpenType font ({0})")]
    NotAFont(String),
    #[error("the font has no height to scale by: no \"H\", no capital height, no ascender")]
    NoHeight,
    #[error("height must be a positive number of mm, got {0}")]
    Height(f64),
    #[error("spacing must be a finite number of mm, got {0}")]
    Spacing(f64),
    #[error("the string is empty")]
    Empty,
    #[error("the string draws nothing: only spaces, or the font has no outlines for it")]
    Blank,
    #[error("the string has {0} characters, more than the limit of {MAX_CHARS}")]
    TooLong(usize),
    #[error("the string has a line break; one text is one line")]
    LineBreak,
    #[error("the font has no glyph for {}", shown(*.0))]
    MissingGlyph(char),
    #[error("the font has no advance width for {}", shown(*.0))]
    NoAdvance(char),
    #[error("the outline of {} has more than {MAX_GLYPH_POINTS} points", shown(*.0))]
    GlyphTooDetailed(char),
    #[error("the outline of {} has coordinates that are not finite", shown(*.0))]
    NotFinite(char),
    #[error("the outline of {} crosses or touches itself", shown(*.0))]
    SelfCrossing(char),
    #[error("the outlines have more than {MAX_POINTS} points together")]
    TooDetailed,
}

/// A character as error messages name it: `'A' (U+0041)`.
fn shown(c: char) -> String {
    format!("{c:?} (U+{:04X})", u32::from(c))
}

/// Sets `text` in the font `font` (the bytes of a `.ttf` or `.otf` file).
///
/// # Errors
/// The bytes are no font, the string is empty, draws nothing, is too long or
/// has a line break, the font lacks a glyph for one of its characters, or an
/// outline is unusable.
pub fn lettering(font: &[u8], text: &str, setting: &Setting) -> Result<Lettering, FontError> {
    if !(setting.height > 0.0 && setting.height.is_finite()) {
        return Err(FontError::Height(setting.height));
    }
    if !setting.spacing.is_finite() {
        return Err(FontError::Spacing(setting.spacing));
    }
    checked(text)?;
    if font.len() > MAX_FONT_BYTES {
        return Err(FontError::TooLarge(font.len()));
    }
    let face = Face::parse(font, 0).map_err(|e| FontError::NotAFont(e.to_string()))?;
    let cap = cap_units(&face).ok_or(FontError::NoHeight)?;
    let scale = setting.height / cap;
    let glyphs = placed(&face, text, cap, setting.spacing / scale, MIN_EDGE_MM / scale)?;
    let (mut regions, overlapping) = regions::assemble(glyphs)?;

    let xs = || regions.iter().flat_map(|r| &r.outline).map(|p| p[0]);
    let (left, right) =
        (xs().fold(f64::INFINITY, f64::min), xs().fold(f64::NEG_INFINITY, f64::max));
    if regions.is_empty() || !(left.is_finite() && right.is_finite()) {
        return Err(FontError::Blank);
    }
    let shift = match setting.align {
        Align::Left => left,
        Align::Center => f64::midpoint(left, right),
        Align::Right => right,
    };
    for region in &mut regions {
        for p in region.outline.iter_mut().chain(region.holes.iter_mut().flatten()) {
            *p = [(p[0] - shift) * scale, p[1] * scale];
        }
    }
    Ok(Lettering { regions, overlapping })
}

/// The rules for the string itself.
fn checked(text: &str) -> Result<(), FontError> {
    if text.is_empty() {
        return Err(FontError::Empty);
    }
    let count = text.chars().count();
    if count > MAX_CHARS {
        return Err(FontError::TooLong(count));
    }
    let breaks = ['\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}'];
    if text.contains(breaks) {
        return Err(FontError::LineBreak);
    }
    if text.chars().all(char::is_whitespace) {
        return Err(FontError::Blank);
    }
    Ok(())
}

/// The cap height in font units: the top of "H", else the capital height
/// of the OS/2 table, else the ascender.
fn cap_units(face: &Face) -> Option<f64> {
    let letter = glyph(face, 'H').and_then(|id| {
        let mut outline = Flattener::new(FLATNESS * f64::from(face.units_per_em()), 0.0);
        face.outline_glyph(id, &mut outline)?;
        outline.contours.iter().flatten().map(|p| p[1]).reduce(f64::max)
    });
    [letter, face.capital_height().map(f64::from), Some(f64::from(face.ascender()))]
        .into_iter()
        .flatten()
        .find(|units| *units > 0.0 && units.is_finite())
}

/// The glyph of a character; glyph 0 is the font's "missing" box.
fn glyph(face: &Face, c: char) -> Option<GlyphId> {
    face.glyph_index(c).filter(|id| id.0 != 0)
}

/// The flattened glyphs of `text` at their pen positions, in font units.
/// `spacing` and `min_edge` are in font units too.
fn placed(
    face: &Face,
    text: &str,
    cap: f64,
    spacing: f64,
    min_edge: f64,
) -> Result<Vec<Glyph>, FontError> {
    let mut glyphs = Vec::new();
    let (mut pen, mut points) = (0.0, 0);
    let mut previous = None;
    for c in text.chars() {
        let id = glyph(face, c).ok_or(FontError::MissingGlyph(c))?;
        if let Some(left) = previous {
            pen += kerning(face, left, id);
        }
        let mut outline = Flattener::new(FLATNESS * cap, pen);
        // A glyph without an outline (a space, a blank Braille cell) only
        // advances.
        let _ = face.outline_glyph(id, &mut outline);
        if outline.overflow {
            return Err(FontError::GlyphTooDetailed(c));
        }
        if outline.broken {
            return Err(FontError::NotFinite(c));
        }
        let contours: Vec<_> =
            outline.contours.into_iter().filter_map(|k| regions::cleaned(k, min_edge)).collect();
        points += contours.iter().map(Vec::len).sum::<usize>();
        if points > MAX_POINTS {
            return Err(FontError::TooDetailed);
        }
        glyphs.push(Glyph { character: c, contours });
        pen += f64::from(face.glyph_hor_advance(id).ok_or(FontError::NoAdvance(c))?) + spacing;
        previous = Some(id);
    }
    Ok(glyphs)
}

/// The pair kerning of the `kern` table in font units (horizontal subtables
/// without state machines; their values add up).
fn kerning(face: &Face, left: GlyphId, right: GlyphId) -> f64 {
    let Some(table) = face.tables().kern else {
        return 0.0;
    };
    table
        .subtables
        .into_iter()
        .filter(|s| s.horizontal && !s.variable && !s.has_cross_stream && !s.has_state_machine)
        .filter_map(|s| s.glyphs_kerning(left, right))
        .map(f64::from)
        .sum()
}

/// Line segments for a curve whose second differences are at most `bend`
/// long: `factor` is 1/4 for a quadratic and 3/4 for a cubic Bézier curve.
fn segments(bend: f64, factor: f64, tolerance: f64) -> usize {
    // NaN becomes 0 and infinity the largest value: both end in the range.
    ((factor * bend / tolerance).sqrt().ceil() as usize).clamp(1, MAX_CURVE_SEGMENTS)
}

fn second_difference(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (a[0] - 2.0 * b[0] + c[0]).hypot(a[1] - 2.0 * b[1] + c[1])
}

/// Collects the contours of one glyph as polylines in font units, shifted
/// by the pen position.
struct Flattener {
    tolerance: f64,
    pen: f64,
    contours: Vec<Vec<[f64; 2]>>,
    points: usize,
    /// More than [`MAX_GLYPH_POINTS`] points: the rest was dropped.
    overflow: bool,
    /// A coordinate was not finite.
    broken: bool,
}

impl Flattener {
    fn new(tolerance: f64, pen: f64) -> Self {
        Self { tolerance, pen, contours: Vec::new(), points: 0, overflow: false, broken: false }
    }

    fn at(&mut self, x: f32, y: f32) -> [f64; 2] {
        self.broken |= !(x.is_finite() && y.is_finite());
        [f64::from(x) + self.pen, f64::from(y)]
    }

    fn push(&mut self, p: [f64; 2]) {
        if self.points >= MAX_GLYPH_POINTS {
            self.overflow = true;
        } else if let Some(contour) = self.contours.last_mut() {
            contour.push(p);
            self.points += 1;
        }
    }

    /// `n` points of the curve `point(t)`, t in (0, 1].
    fn curve(&mut self, n: usize, point: impl Fn(f64) -> [f64; 2]) {
        for i in 1..=n {
            self.push(point(i as f64 / n as f64));
        }
    }

    fn last(&self) -> Option<[f64; 2]> {
        self.contours.last().and_then(|c| c.last()).copied()
    }
}

impl OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        let start = self.at(x, y);
        if !self.overflow {
            self.contours.push(Vec::new());
            self.push(start);
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let end = self.at(x, y);
        self.push(end);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (b, c) = (self.at(x1, y1), self.at(x, y));
        let Some(a) = self.last() else { return };
        let n = segments(second_difference(a, b, c), 0.25, self.tolerance);
        self.curve(n, |t| {
            let s = 1.0 - t;
            [0, 1].map(|k| s * s * a[k] + 2.0 * s * t * b[k] + t * t * c[k])
        });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (b, c, d) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        let Some(a) = self.last() else { return };
        let bend = second_difference(a, b, c).max(second_difference(b, c, d));
        let n = segments(bend, 0.75, self.tolerance);
        self.curve(n, |t| {
            let s = 1.0 - t;
            [0, 1].map(|k| {
                s * s * s * a[k]
                    + 3.0 * s * s * t * b[k]
                    + 3.0 * s * t * t * c[k]
                    + t * t * t * d[k]
            })
        });
    }

    fn close(&mut self) {}
}
