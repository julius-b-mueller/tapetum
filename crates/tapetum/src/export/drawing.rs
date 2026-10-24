//! A page of vector drawing, written as PDF or SVG: paths, text in Helvetica
//! (a PDF standard font, nothing to embed) and pictures. Millimetres, the
//! origin at the top left, y pointing down.

use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32);

impl Color {
    pub const BLACK: Color = Color(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color(1.0, 1.0, 1.0);

    pub fn gray(v: f32) -> Color {
        Color(v, v, v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub color: Color,
    /// Line width in mm.
    pub width: f32,
    /// Dash and gap in mm, if dashed.
    pub dash: Option<(f32, f32)>,
}

impl Stroke {
    pub fn solid(color: Color, width: f32) -> Stroke {
        Stroke { color, width, dash: None }
    }

    pub fn dashed(color: Color, width: f32, dash: f32, gap: f32) -> Stroke {
        Stroke { color, width, dash: Some((dash, gap)) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug)]
pub enum Item {
    Path { points: Vec<[f32; 2]>, closed: bool, stroke: Option<Stroke>, fill: Option<Color> },
    /// `at` is the left/centre/right end of the baseline.
    Text { at: [f32; 2], size: f32, text: String, color: Color, anchor: Anchor, bold: bool },
    /// A picture (straight alpha) on the parallelogram from its top left,
    /// top right and bottom left corner.
    Image { image: Arc<image::RgbaImage>, top_left: [f32; 2], top_right: [f32; 2], bottom_left: [f32; 2] },
}

#[derive(Clone, Debug)]
pub struct Page {
    /// Width and height in mm.
    pub size: [f32; 2],
    pub items: Vec<Item>,
}

impl Page {
    pub fn new(size: [f32; 2]) -> Page {
        Page { size, items: Vec::new() }
    }

    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], stroke: Stroke) {
        self.items.push(Item::Path { points: vec![a, b], closed: false, stroke: Some(stroke), fill: None });
    }

    pub fn polyline(&mut self, points: Vec<[f32; 2]>, closed: bool, stroke: Option<Stroke>, fill: Option<Color>) {
        if points.len() >= 2 {
            self.items.push(Item::Path { points, closed, stroke, fill });
        }
    }

    pub fn rect(&mut self, min: [f32; 2], max: [f32; 2], stroke: Option<Stroke>, fill: Option<Color>) {
        self.polyline(vec![min, [max[0], min[1]], max, [min[0], max[1]]], true, stroke, fill);
    }

    pub fn circle(&mut self, c: [f32; 2], r: f32, stroke: Option<Stroke>, fill: Option<Color>) {
        let n = 28;
        let points = (0..n).map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        });
        self.polyline(points.collect(), true, stroke, fill);
    }

    pub fn text(&mut self, at: [f32; 2], size: f32, text: impl Into<String>, anchor: Anchor, bold: bool) {
        self.items.push(Item::Text { at, size, text: text.into(), color: Color::BLACK, anchor, bold });
    }
}

/// Paper sizes in mm, landscape.
pub const A3: [f32; 2] = [420.0, 297.0];

/// Advance widths of Helvetica (and Helvetica-Bold) for ASCII 32–126 in
/// 1/1000 of the font size, from the standard font metrics.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
    667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
    667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Width of a character, letters with accents as their base letter.
fn char_width(c: char, bold: bool) -> u16 {
    let table = if bold { &HELVETICA_BOLD } else { &HELVETICA };
    let base = match c {
        'Ä' | 'Á' | 'À' | 'Â' | 'Å' => 'A',
        'Ö' | 'Ó' | 'Ò' | 'Ô' => 'O',
        'Ü' | 'Ú' | 'Ù' | 'Û' => 'U',
        'É' | 'È' | 'Ê' => 'E',
        'ä' | 'á' | 'à' | 'â' | 'å' => 'a',
        'ö' | 'ó' | 'ò' | 'ô' => 'o',
        'ü' | 'ú' | 'ù' | 'û' => 'u',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'ß' => return 611,
        '–' => return 556,
        '—' | '…' => return 1000,
        '·' | '‘' | '’' => return 278,
        '„' | '“' | '”' => return 333,
        c => c,
    };
    let i = base as usize;
    if (32..127).contains(&i) { table[i - 32] } else { 556 }
}

/// Width of a text in mm at `size` mm.
pub fn text_width(text: &str, size: f32, bold: bool) -> f32 {
    text.chars().map(|c| f32::from(char_width(c, bold))).sum::<f32>() / 1000.0 * size
}

/// A text in the PDF standard fonts' encoding (WinAnsi); what it lacks becomes `?`.
pub fn win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c {
            ' '..='~' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            '€' => 0x80,
            '‚' => 0x82,
            '„' => 0x84,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            _ => b'?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_encoding() {
        assert!((text_width("Hi", 10.0, false) - (7.22 + 2.22)).abs() < 1e-4);
        assert!(text_width("Übung", 10.0, true) > text_width("Übung", 10.0, false));
        assert_eq!(win_ansi("Größe – 5 €"), b"Gr\xf6\xdfe \x96 5 \x80");
        assert_eq!(win_ansi("✓"), b"?");
    }
}
