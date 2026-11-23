//! Contact sheet (AG-MC-09): several views of the same size in one image,
//! each with its name in the top left corner. The names are drawn with a
//! built-in 5x7 bitmap font, because nothing else renders text headless.

/// Columns of the 5x7 glyphs.
const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;
/// Label colours (RGBA): light text on a dark plate, readable on any render.
const TEXT: [u8; 4] = [255, 255, 255, 255];
const PLATE: [u8; 4] = [32, 32, 32, 255];

/// The rows of a letter of a view name, top to bottom, bit 4 = left pixel.
/// Characters outside the view names are drawn as blanks.
fn glyph(c: char) -> [u8; GLYPH_H] {
    match c.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        _ => [0; GLYPH_H],
    }
}

/// An RGBA image under construction.
struct Canvas {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Canvas {
    /// Fills the rectangle, clipped to the image.
    fn fill(&mut self, [x, y]: [usize; 2], [w, h]: [usize; 2], colour: [u8; 4]) {
        for row in y..(y + h).min(self.height) {
            for column in x..(x + w).min(self.width) {
                let at = 4 * (row * self.width + column);
                self.rgba[at..at + 4].copy_from_slice(&colour);
            }
        }
    }

    /// `text` on its plate with the top left corner at `origin`; one font
    /// pixel is `scale` image pixels.
    fn label(&mut self, [x, y]: [usize; 2], text: &str, scale: usize) {
        let advance = (GLYPH_W + 1) * scale;
        let plate = [text.chars().count() * advance + scale, (GLYPH_H + 2) * scale];
        self.fill([x, y], plate, PLATE);
        for (i, c) in text.chars().enumerate() {
            for (row, bits) in glyph(c).into_iter().enumerate() {
                for column in (0..GLYPH_W).filter(|column| bits >> (GLYPH_W - 1 - column) & 1 == 1)
                {
                    let at = [x + scale + i * advance + column * scale, y + (row + 1) * scale];
                    self.fill(at, [scale, scale], TEXT);
                }
            }
        }
    }
}

/// The sheet of `images` (RGBA, each `size`) with their `labels`: up to
/// three in a row, four as two rows of two. Returns its size and pixels.
pub(super) fn compose(images: &[Vec<u8>], labels: &[&str], size: [u32; 2]) -> ([u32; 2], Vec<u8>) {
    let [w, h] = size.map(|n| n as usize);
    let columns = if images.len() == 4 { 2 } else { images.len().clamp(1, 3) };
    let rows = images.len().div_ceil(columns);
    let (width, height) = (columns * w, rows * h);
    let mut sheet = Canvas { width, height, rgba: vec![255; 4 * width * height] };
    // Legible from 120 pixel high cells on, growing with the cell.
    let scale = (h / 120).max(1);
    for (i, (image, label)) in images.iter().zip(labels).enumerate() {
        let (x, y) = (i % columns * w, i / columns * h);
        for (row, pixels) in image.chunks_exact(4 * w).take(h).enumerate() {
            let at = 4 * ((y + row) * width + x);
            sheet.rgba[at..at + 4 * w].copy_from_slice(pixels);
        }
        sheet.label([x + 2 * scale, y + 2 * scale], label, scale);
    }
    ([width as u32, height as u32], sheet.rgba)
}
