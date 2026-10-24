//! Pages as a PDF: vector paths, Helvetica text, pictures with transparency.

use std::io::Write;

use super::drawing::{Anchor, Color, Item, Page, Stroke, text_width, win_ansi};

/// Points per millimetre.
const PT: f32 = 72.0 / 25.4;

fn num(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// A PDF string literal: bytes with `(`, `)` and `\` escaped.
fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![b'('];
    for &b in bytes {
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    out
}

struct Writer {
    out: Vec<u8>,
    offsets: Vec<usize>,
}

impl Writer {
    /// Reserves the next object number.
    fn reserve(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn object(&mut self, id: usize, body: &[u8]) {
        self.offsets[id - 1] = self.out.len();
        self.out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    fn stream(&mut self, id: usize, dict: &str, data: &[u8]) {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.object(id, &body);
    }
}

fn color(c: Color, op: &str) -> String {
    format!("{} {} {} {op}\n", num(c.0), num(c.1), num(c.2))
}

/// The pages as a PDF file.
pub fn write(pages: &[Page], title: &str) -> Vec<u8> {
    let mut w = Writer { out: b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec(), offsets: Vec::new() };
    let (catalog, pages_id, font, bold, info) = (w.reserve(), w.reserve(), w.reserve(), w.reserve(), w.reserve());
    w.object(font, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    w.object(bold, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>");
    let mut kids = Vec::new();
    for page in pages {
        let height = page.size[1];
        let p = |q: [f32; 2]| [q[0] * PT, (height - q[1]) * PT];
        let mut content = Vec::new();
        let mut images = Vec::new();
        for item in &page.items {
            match item {
                Item::Path { points, closed, stroke, fill } => {
                    content.extend_from_slice(b"q\n");
                    if let Some(f) = fill {
                        content.extend_from_slice(color(*f, "rg").as_bytes());
                    }
                    if let Some(Stroke { color: c, width, dash }) = stroke {
                        content.extend_from_slice(color(*c, "RG").as_bytes());
                        content.extend_from_slice(format!("{} w 1 J 1 j\n", num(width * PT)).as_bytes());
                        if let Some((d, g)) = dash {
                            content.extend_from_slice(format!("[{} {}] 0 d 0 J\n", num(d * PT), num(g * PT)).as_bytes());
                        }
                    }
                    for (i, q) in points.iter().enumerate() {
                        let [x, y] = p(*q);
                        content.extend_from_slice(format!("{} {} {}\n", num(x), num(y), if i == 0 { "m" } else { "l" }).as_bytes());
                    }
                    let op: &[u8] = match (fill.is_some(), stroke.is_some(), closed) {
                        (true, true, _) => b"b\n",
                        (true, false, _) => b"f\n",
                        (false, true, true) => b"s\n",
                        (false, true, false) => b"S\n",
                        (false, false, _) => b"n\n",
                    };
                    content.extend_from_slice(op);
                    content.extend_from_slice(b"Q\n");
                }
                Item::Text { at, size, text, color: c, anchor, bold: b } => {
                    let shift = match anchor {
                        Anchor::Left => 0.0,
                        Anchor::Center => text_width(text, *size, *b) / 2.0,
                        Anchor::Right => text_width(text, *size, *b),
                    };
                    let [x, y] = p([at[0] - shift, at[1]]);
                    content.extend_from_slice(b"BT\n");
                    content.extend_from_slice(color(*c, "rg").as_bytes());
                    content.extend_from_slice(format!("/{} {} Tf {} {} Td ", if *b { "F2" } else { "F1" }, num(size * PT), num(x), num(y)).as_bytes());
                    content.extend_from_slice(&literal(&win_ansi(text)));
                    content.extend_from_slice(b" Tj\nET\n");
                }
                Item::Image { image, top_left, top_right, bottom_left } => {
                    let (iw, ih) = image.dimensions();
                    let mut rgb = Vec::with_capacity((iw * ih * 3) as usize);
                    let mut alpha = Vec::with_capacity((iw * ih) as usize);
                    for px in image.pixels() {
                        rgb.extend_from_slice(&px.0[..3]);
                        alpha.push(px.0[3]);
                    }
                    let (img, mask) = (w.reserve(), w.reserve());
                    w.stream(mask, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode"), &deflate(&alpha));
                    w.stream(img, &format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask {mask} 0 R /Filter /FlateDecode"), &deflate(&rgb));
                    let name = format!("Im{}", images.len() + 1);
                    // The unit square: x along the top edge, y up from the bottom left corner.
                    let (tl, tr, bl) = (p(*top_left), p(*top_right), p(*bottom_left));
                    let m = [tr[0] - tl[0], tr[1] - tl[1], tl[0] - bl[0], tl[1] - bl[1], bl[0], bl[1]];
                    content.extend_from_slice(format!("q {} cm /{name} Do Q\n", m.map(num).join(" ")).as_bytes());
                    images.push((name, img));
                }
            }
        }
        let contents = w.reserve();
        w.stream(contents, "/Filter /FlateDecode", &deflate(&content));
        let xobjects: String = images.iter().map(|(n, id)| format!("/{n} {id} 0 R ")).collect();
        let page_id = w.reserve();
        let body = format!(
            "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Contents {contents} 0 R /Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> /XObject << {xobjects}>> >> >>",
            num(page.size[0] * PT),
            num(page.size[1] * PT)
        );
        w.object(page_id, body.as_bytes());
        kids.push(page_id);
    }
    let kids_list: String = kids.iter().map(|k| format!("{k} 0 R ")).collect();
    w.object(pages_id, format!("<< /Type /Pages /Kids [{kids_list}] /Count {} >>", kids.len()).as_bytes());
    w.object(catalog, format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").as_bytes());
    let mut info_body = b"<< /Producer (Tapetum) /Title ".to_vec();
    info_body.extend_from_slice(&literal(&win_ansi(title)));
    info_body.extend_from_slice(b" >>");
    w.object(info, &info_body);
    let xref = w.out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", w.offsets.len() + 1);
    for o in &w.offsets {
        tail.push_str(&format!("{o:010} 00000 n \n"));
    }
    tail.push_str(&format!("trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n", w.offsets.len() + 1));
    w.out.extend_from_slice(tail.as_bytes());
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_with_everything() {
        let mut page = Page::new([297.0, 210.0]);
        page.line([10.0, 10.0], [100.0, 10.0], Stroke::dashed(Color::BLACK, 0.3, 2.0, 1.0));
        page.circle([50.0, 50.0], 5.0, Some(Stroke::solid(Color::BLACK, 0.25)), Some(Color::WHITE));
        page.text([20.0, 30.0], 4.0, "Größe (1:50)", Anchor::Left, true);
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 10, 10, 128]));
        page.items.push(Item::Image { image: std::sync::Arc::new(img), top_left: [60.0, 60.0], top_right: [80.0, 60.0], bottom_left: [60.0, 90.0] });
        let pdf = write(&[page.clone(), page], "Test");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4") && text.ends_with("%%EOF\n"));
        assert!(text.contains("/Count 2") && text.contains("/SMask"));
        // Every object the cross-reference table names starts where it says.
        let xref = text[text.rfind("startxref\n").unwrap() + 10..].lines().next().unwrap().parse::<usize>().unwrap();
        for (i, line) in text[xref..].lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
            let at: usize = line[..10].parse().unwrap();
            assert!(pdf[at..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
        }
        assert_eq!(literal(b"a(b)\\"), b"(a\\(b\\)\\\\)");
    }
}
