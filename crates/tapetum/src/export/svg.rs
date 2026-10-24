//! A page as SVG (millimetres as user units).

use std::fmt::Write;

use super::drawing::{Anchor, Color, Item, Page};

fn num(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn rgb(c: Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.0), b(c.1), b(c.2))
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn write(page: &Page, title: &str) -> String {
    let [w, h] = page.size;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="0 0 {} {}">
<title>{}</title>
<rect width="100%" height="100%" fill="#ffffff"/>"##,
        num(w),
        num(h),
        num(w),
        num(h),
        escape(title)
    );
    for item in &page.items {
        match item {
            Item::Path { points, closed, stroke, fill } => {
                let d: String = points.iter().enumerate().map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { " L" }, num(p[0]), num(p[1]))).collect();
                let mut attrs = format!(r#"d="{d}{}" fill="{}""#, if *closed { " Z" } else { "" }, fill.map_or("none".into(), rgb));
                if let Some(s) = stroke {
                    let _ = write!(attrs, r#" stroke="{}" stroke-width="{}" stroke-linejoin="round""#, rgb(s.color), num(s.width));
                    match s.dash {
                        Some((d, g)) => {
                            let _ = write!(attrs, r#" stroke-dasharray="{} {}""#, num(d), num(g));
                        }
                        None => attrs.push_str(r#" stroke-linecap="round""#),
                    }
                }
                let _ = writeln!(out, "<path {attrs}/>");
            }
            Item::Text { at, size, text, color, anchor, bold } => {
                let anchor = match anchor {
                    Anchor::Left => "start",
                    Anchor::Center => "middle",
                    Anchor::Right => "end",
                };
                let _ = writeln!(
                    out,
                    r#"<text x="{}" y="{}" font-family="Helvetica, Arial, sans-serif" font-size="{}" fill="{}" text-anchor="{anchor}"{}>{}</text>"#,
                    num(at[0]),
                    num(at[1]),
                    num(*size),
                    rgb(*color),
                    if *bold { r#" font-weight="bold""# } else { "" },
                    escape(text)
                );
            }
            Item::Image { image, top_left, top_right, bottom_left } => {
                let mut png = Vec::new();
                if image::DynamicImage::ImageRgba8((**image).clone()).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
                    continue;
                }
                // The unit square onto the corners (y down, like the page).
                let m = [top_right[0] - top_left[0], top_right[1] - top_left[1], bottom_left[0] - top_left[0], bottom_left[1] - top_left[1], top_left[0], top_left[1]];
                let _ = writeln!(
                    out,
                    r#"<image width="1" height="1" preserveAspectRatio="none" transform="matrix({})" href="data:image/png;base64,{}"/>"#,
                    m.map(num).join(" "),
                    base64(&png)
                );
            }
        }
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_like_everyone_else() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
