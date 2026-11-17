//! Screen text with a tiny built-in 3x5 pixel font, drawn as quads (no textures needed yet).

use crate::gfx::MeshBuilder;

/// Each glyph is five rows of three pixels, top row first, as a 15-character string of 0/1.
const GLYPHS: &[(char, &str)] = &[
    ('A', "010101111101101"),
    ('B', "110101110101110"),
    ('C', "011100100100011"),
    ('D', "110101101101110"),
    ('E', "111100110100111"),
    ('F', "111100110100100"),
    ('G', "011100101101011"),
    ('H', "101101111101101"),
    ('I', "111010010010111"),
    ('J', "001001001101010"),
    ('K', "101101110101101"),
    ('L', "100100100100111"),
    ('M', "101111111101101"),
    ('N', "110101101101101"),
    ('O', "010101101101010"),
    ('P', "110101110100100"),
    ('Q', "010101101110011"),
    ('R', "110101110101101"),
    ('S', "011100010001110"),
    ('T', "111010010010010"),
    ('U', "101101101101111"),
    ('V', "101101101101010"),
    ('W', "101101111111101"),
    ('X', "101101010101101"),
    ('Y', "101101010010010"),
    ('Z', "111001010100111"),
    ('0', "111101101101111"),
    ('1', "010110010010111"),
    ('2', "110001010100111"),
    ('3', "110001010001110"),
    ('4', "101101111001001"),
    ('5', "111100110001110"),
    ('6', "011100111101111"),
    ('7', "111001010010010"),
    ('8', "111101111101111"),
    ('9', "111101111001110"),
    ('-', "000000111000000"),
    (':', "000010000010000"),
    ('/', "001001010100100"),
    ('.', "000000000000010"),
    ('!', "010010010000010"),
    ('+', "000010111010000"),
];

fn glyph(c: char) -> Option<&'static str> {
    let c = c.to_ascii_uppercase();
    GLYPHS.iter().find(|(g, _)| *g == c).map(|(_, bits)| *bits)
}

/// Width in screen units of `text` at pixel size `px`.
pub fn text_width(text: &str, px: f32) -> f32 {
    let n = text.chars().count() as f32;
    if n == 0.0 { 0.0 } else { n * 4.0 * px - px }
}

/// Appends `text` with its top-left at (x, y). Draws a 1-pixel drop shadow first so it reads on any
/// background.
pub fn text(m: &mut MeshBuilder, x: f32, y: f32, px: f32, text: &str, rgba: u32) {
    for (dx, dy, col) in [(px * 0.5, px * 0.5, 0x0000_00a0), (0.0, 0.0, rgba)] {
        for (i, c) in text.chars().enumerate() {
            let Some(bits) = glyph(c) else { continue };
            let gx = x + dx + i as f32 * 4.0 * px;
            for (k, b) in bits.bytes().enumerate() {
                if b == b'1' {
                    let (cx, cy) = ((k % 3) as f32, (k / 3) as f32);
                    m.rect_2d(gx + cx * px, y + dy + cy * px, px, px, col);
                }
            }
        }
    }
}

/// Centred horizontally on `cx`.
pub fn text_centered(m: &mut MeshBuilder, cx: f32, y: f32, px: f32, s: &str, rgba: u32) {
    text(m, cx - text_width(s, px) / 2.0, y, px, s, rgba);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_are_well_formed_and_unique() {
        for (i, (c, bits)) in GLYPHS.iter().enumerate() {
            assert_eq!(bits.len(), 15, "{c}");
            assert!(bits.bytes().all(|b| b == b'0' || b == b'1'), "{c}");
            assert!(GLYPHS[i + 1..].iter().all(|(d, _)| d != c), "duplicate {c}");
        }
    }

    #[test]
    fn spaces_draw_nothing() {
        let mut m = MeshBuilder::new();
        text(&mut m, 0.0, 0.0, 2.0, "   ", 0xffffffff);
        assert!(m.verts.is_empty());
        text(&mut m, 0.0, 0.0, 2.0, "I", 0xffffffff);
        assert_eq!(
            m.verts.len(),
            2 * 9 * 6,
            "shadow + glyph, 9 pixels, 6 vertices each"
        );
    }
}
