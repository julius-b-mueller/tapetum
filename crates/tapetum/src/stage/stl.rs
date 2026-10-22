//! STL, binary or ASCII. No colors; everything is one gray part.

use crate::i18n::tr;
use std::path::Path;

use bevy::math::Vec3;

use super::{LoadError, Model, ModelObject, Part};

const GRAY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

pub fn load(path: &Path) -> Result<Model, LoadError> {
    let bytes = super::read_limited(path, super::MAX_MODEL_BYTES)?;
    let triangles = if is_binary(&bytes) { binary(&bytes) } else { ascii(&String::from_utf8_lossy(&bytes))? };
    let mut part = Part { color: GRAY, ..Default::default() };
    for (n, verts) in triangles {
        let n = if n.length_squared() > 1e-12 { n.normalize() } else { (verts[1] - verts[0]).cross(verts[2] - verts[0]).normalize_or_zero() };
        let base = part.positions.len() as u32;
        part.positions.extend(verts);
        part.normals.extend([n; 3]);
        part.indices.extend([base, base + 1, base + 2]);
    }
    Ok(Model { objects: vec![ModelObject { key: String::new(), name: tr("Modell", "Model").into(), parts: vec![part], ..Default::default() }] })
}

/// Binary STL: 80-byte header, triangle count, 50 bytes per triangle. Some
/// binary files start with "solid" too, so the size decides.
fn is_binary(b: &[u8]) -> bool {
    b.len() >= 84 && {
        let n = u32::from_le_bytes([b[80], b[81], b[82], b[83]]) as usize;
        b.len() == 84 + n * 50
    }
}

fn binary(b: &[u8]) -> Vec<(Vec3, [Vec3; 3])> {
    let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let v = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
    (84..b.len()).step_by(50).filter(|o| o + 50 <= b.len()).map(|o| (v(o), [v(o + 12), v(o + 24), v(o + 36)])).collect()
}

fn ascii(text: &str) -> Result<Vec<(Vec3, [Vec3; 3])>, LoadError> {
    let mut out = Vec::new();
    let mut normal = Vec3::ZERO;
    let mut verts = Vec::new();
    let num = |s: Option<&str>| s.and_then(|x| x.parse::<f32>().ok()).ok_or_else(|| LoadError::Format(tr("STL: Zahl erwartet", "STL: number expected").into()));
    for line in text.lines() {
        let mut w = line.split_whitespace();
        match w.next() {
            Some("facet") => {
                w.next(); // "normal"
                normal = Vec3::new(num(w.next())?, num(w.next())?, num(w.next())?);
                verts.clear();
            }
            Some("vertex") => verts.push(Vec3::new(num(w.next())?, num(w.next())?, num(w.next())?)),
            Some("endfacet") if verts.len() >= 3 => out.push((normal, [verts[0], verts[1], verts[2]])),
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_binary() {
        let dir = std::env::temp_dir().join(format!("tapetum-stl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.stl");
        std::fs::write(&a, "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n").unwrap();
        assert_eq!(load(&a).unwrap().triangle_count(), 1);

        let mut b = vec![0u8; 80];
        b.extend(1u32.to_le_bytes());
        for v in [0.0f32, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            b.extend(v.to_le_bytes());
        }
        b.extend([0, 0]);
        let bin = dir.join("b.stl");
        std::fs::write(&bin, b).unwrap();
        let m = load(&bin).unwrap();
        assert_eq!(m.objects[0].parts[0].positions[1], Vec3::X);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
