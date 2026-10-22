//! IngeTrazo documents (`.igz`, see `formats/igz.py` in IngeTrazo).
//!
//! Plain JSON (format 1) or a ZIP holding `document.json`, `textures/` and,
//! for large documents (format 3), binary geometry blocks under `geom/`.
//! The scene is a loose mesh in world coordinates plus a tree of groups;
//! each group's `xform` (column major) composes from the root down, and a
//! component instance takes its mesh from `protos`. IngeTrazo works in
//! meters with Z up, like BlinderKitten.

use crate::i18n::tr;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use bevy::math::{Mat4, Vec2, Vec3};
use serde_json::Value;

use super::{Builder, LoadError, Model, Texture, decode_image, srgb_to_linear};

/// The color IngeTrazo shows unpainted faces in.
const DEFAULT_COLOR: [f32; 3] = [0.85, 0.85, 0.83];

struct Doc {
    scene: Value,
    /// ZIP members (textures, geometry blocks) by name.
    files: HashMap<String, Vec<u8>>,
    /// Directory of the file: textures outside it are looked up from here.
    dir: std::path::PathBuf,
}

fn read(path: &Path) -> Result<Doc, LoadError> {
    let bytes = super::read_limited(path, super::MAX_MODEL_BYTES)?;
    let bad = |e: &dyn std::fmt::Display| LoadError::Format(trf!("IngeTrazo-Datei: {e}", "IngeTrazo file: {e}", e = e));
    let (json, files) = if bytes.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(&e))?;
        let mut files = HashMap::new();
        let mut json = None;
        // A zip bomb (or a lying size field) stops at these limits.
        let mut left = super::MAX_MODEL_BYTES;
        for i in 0..zip.len() {
            let f = zip.by_index(i).map_err(|e| bad(&e))?;
            let name = f.name().map_err(|e| bad(&e))?.into_owned();
            let cap = left.min(super::MAX_SIDE_BYTES);
            let mut data = Vec::with_capacity(f.size().min(cap).min(64 << 20) as usize);
            f.take(cap + 1).read_to_end(&mut data)?;
            if data.len() as u64 > cap {
                return Err(bad(&trf!("{name} ist zu groß", "{name} is too large", name = name)));
            }
            left -= data.len() as u64;
            if name == "document.json" {
                json = Some(data);
            } else {
                files.insert(name, data);
            }
        }
        (json.ok_or_else(|| bad(&tr("document.json fehlt", "document.json is missing")))?, files)
    } else {
        (bytes, HashMap::new())
    };
    let doc: Value = serde_json::from_slice(&json).map_err(|e| bad(&e))?;
    let format = doc.get("igz_format").and_then(Value::as_i64).unwrap_or(1);
    if format > 3 {
        return Err(bad(&trf!("Format {format} ist neuer als unterstützt (bis 3)", "format {format} is newer than supported (up to 3)", format = format)));
    }
    let scene = doc.get("scene").cloned().ok_or_else(|| bad(&tr("keine Szene", "no scene")))?;
    Ok(Doc { scene, files, dir: path.parent().map(Path::to_path_buf).unwrap_or_default() })
}

pub fn load(path: &Path) -> Result<Model, LoadError> {
    let doc = read(path)?;
    let mut ctx = Ctx { doc: &doc, builder: Builder::default(), textures: HashMap::new(), hidden_layers: Vec::new(), in_billboard: false };
    ctx.hidden_layers = doc
        .scene
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|l| l.get("visible").and_then(Value::as_bool) == Some(false))
        .filter_map(|l| l.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    let protos: Vec<Value> = doc.scene.get("protos").and_then(Value::as_array).cloned().unwrap_or_default();
    ctx.builder.object("loose", tr("Lose Geometrie", "Loose geometry"));
    ctx.mesh(&doc.scene, Mat4::IDENTITY, None)?;
    // Every top-level group or component is an object of its own. Its uid
    // survives renaming and reordering in IngeTrazo, so overrides and scenes stick.
    for (i, g) in doc.scene.get("groups").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let key = g.get("uid").and_then(Value::as_str).map_or_else(|| format!("g{i}"), |u| format!("uid:{u}"));
        let kind = if g.get("proto").is_some() { tr("Komponente", "Component") } else { tr("Gruppe", "Group") };
        let name = g.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map_or_else(|| format!("{kind} {}", i + 1), str::to_string);
        ctx.builder.object(&key, &name);
        // Face-me figures (people, trees) come along; the stage turns them to the camera.
        let billboard = g.get("billboard").is_some_and(|b| !b.is_null() && b != &Value::Bool(false));
        if billboard {
            ctx.builder.billboard();
        }
        // A classic one is a single quad its picture covers (`make_billboard_group`).
        ctx.in_billboard = g.get("billboard") == Some(&Value::Bool(true));
        ctx.group(g, Mat4::IDENTITY, &protos, None, 0)?;
        ctx.in_billboard = false;
    }
    Ok(ctx.builder.finish())
}

struct Ctx<'a> {
    doc: &'a Doc,
    builder: Builder,
    textures: HashMap<String, Option<Arc<Texture>>>,
    hidden_layers: Vec<String>,
    /// Inside a face-me figure: its picture spans each face exactly once.
    in_billboard: bool,
}

/// Paint a container passes on to faces without their own.
#[derive(Clone)]
struct Paint {
    color: Option<Vec<f64>>,
    texture: Option<Value>,
}

impl Ctx<'_> {
    fn layer_hidden(&self, v: &Value) -> bool {
        v.get("layer").and_then(Value::as_str).is_some_and(|l| self.hidden_layers.iter().any(|h| h == l))
    }

    fn group(&mut self, g: &Value, parent: Mat4, protos: &[Value], inherited: Option<&Paint>, depth: u32) -> Result<(), LoadError> {
        // Hidden groups stay out of the stage.
        if depth > 32 || g.get("hidden").and_then(Value::as_bool) == Some(true) || self.layer_hidden(g) {
            return Ok(());
        }
        let world = match g.get("xform").and_then(Value::as_array) {
            Some(m) if m.len() == 16 => {
                let mut cols = [0.0f32; 16];
                for (c, v) in cols.iter_mut().zip(m) {
                    *c = v.as_f64().unwrap_or(0.0) as f32;
                }
                parent * Mat4::from_cols_array(&cols)
            }
            _ => parent,
        };
        let own = g.get("material").map(|m| Paint { color: m.get("color").and_then(as_floats), texture: m.get("texture").cloned() });
        let paint = own.as_ref().or(inherited).cloned();
        match g.get("proto").and_then(Value::as_u64) {
            Some(p) => {
                if let Some(mesh) = protos.get(p as usize) {
                    self.mesh(mesh, world, paint.as_ref())?;
                }
            }
            None => self.mesh(g, world, paint.as_ref())?,
        }
        for c in g.get("children").and_then(Value::as_array).into_iter().flatten() {
            self.group(c, world, protos, paint.as_ref(), depth + 1)?;
        }
        Ok(())
    }

    /// The faces of a mesh block: JSON lists, or a format-3 binary block.
    fn mesh(&mut self, block: &Value, m: Mat4, paint: Option<&Paint>) -> Result<(), LoadError> {
        if let Some(geom) = block.get("geom") {
            for face in binary_faces(geom, &self.doc.files)? {
                self.face(&face, m, paint);
            }
            return Ok(());
        }
        for face in block.get("faces").and_then(Value::as_array).into_iter().flatten() {
            self.face(face, m, paint);
        }
        Ok(())
    }

    fn face(&mut self, f: &Value, m: Mat4, paint: Option<&Paint>) {
        if f.get("hidden").and_then(Value::as_bool) == Some(true) || self.layer_hidden(f) {
            return;
        }
        let ring = |v: &Value| -> Vec<Vec3> {
            v.as_array().into_iter().flatten().filter_map(as_floats).filter(|p| p.len() >= 3).map(|p| m.transform_point3(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))).collect()
        };
        let Some(outer) = f.get("vertices").map(ring) else { return };
        let holes: Vec<Vec<Vec3>> = f.get("holes").and_then(Value::as_array).into_iter().flatten().map(ring).collect();

        let texture_json = f.get("texture").or(if f.get("color").is_none() { paint.and_then(|p| p.texture.as_ref()) } else { None });
        let texture = texture_json.and_then(|t| self.texture(t));
        let color_raw = f.get("color").and_then(as_floats).or_else(|| paint.and_then(|p| p.color.clone()));
        let srgb = color_raw.as_deref().map_or(DEFAULT_COLOR, |c| [c.first().copied().unwrap_or(0.85) as f32, c.get(1).copied().unwrap_or(0.85) as f32, c.get(2).copied().unwrap_or(0.85) as f32]);
        let alpha = f.get("opacity").and_then(Value::as_f64).unwrap_or(1.0) as f32;
        let color = if texture.is_some() { [1.0, 1.0, 1.0, alpha] } else { [srgb_to_linear(srgb[0]), srgb_to_linear(srgb[1]), srgb_to_linear(srgb[2]), alpha] };

        match (texture, texture_json) {
            (Some(tex), Some(t)) => {
                // UVs in the document's own world frame: map back through `m`.
                let inv = m.inverse();
                let normal = super::triangulate::newell_normal(&outer.iter().map(|&p| inv.transform_point3(p)).collect::<Vec<_>>());
                let (gu, cu, gv, cv) = if self.in_billboard && t.get("uvw").is_none() {
                    fit_to_face(&outer.iter().map(|&p| inv.transform_point3(p)).collect::<Vec<_>>(), normal)
                } else {
                    uv_axes(t, normal)
                };
                let uv = move |p: Vec3| {
                    let q = inv.transform_point3(p);
                    // Image rows run downwards, texture v upwards.
                    Vec2::new(gu.dot(q) + cu, 1.0 - (gv.dot(q) + cv))
                };
                self.builder.polygon(&outer, &holes, color, Some(tex), Some(&uv));
            }
            _ => self.builder.polygon(&outer, &holes, color, None, None),
        }
    }

    fn texture(&mut self, t: &Value) -> Option<Arc<Texture>> {
        let key = t.get("embed").or_else(|| t.get("path")).and_then(Value::as_str)?.to_string();
        let files = &self.doc.files;
        self.textures
            .entry(key.clone())
            .or_insert_with(|| {
                let bytes = files.get(&key).cloned().or_else(|| super::read_limited(&self.doc.dir.join(&key), super::MAX_SIDE_BYTES).ok())?;
                decode_image(format!("igz:{key}"), &bytes)
            })
            .clone()
    }
}

fn as_floats(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?.iter().map(Value::as_f64).collect()
}

/// IngeTrazo's world-to-UV map: `u = gu·p + cu`, `v = gv·p + cv`; a fitted
/// `uvw` when the face has one, else the SketchUp-style planar projection
/// (`core/texture.py: face_uv_axes`).
fn uv_axes(t: &Value, normal: Vec3) -> (Vec3, f32, Vec3, f32) {
    if let Some(u) = t.get("uvw").and_then(as_floats).filter(|u| u.len() == 8) {
        let f = |i: usize| u[i] as f32;
        return (Vec3::new(f(0), f(1), f(2)), f(3), Vec3::new(f(4), f(5), f(6)), f(7));
    }
    let (mut u_axis, mut v_axis) = projection_basis(normal);
    let rot = t.get("rot").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    if rot != 0.0 {
        let (s, c) = rot.to_radians().sin_cos();
        (u_axis, v_axis) = (u_axis * c + v_axis * s, v_axis * c - u_axis * s);
    }
    let size = |k: &str| t.get(k).and_then(Value::as_f64).filter(|v| v.abs() > 1e-9).unwrap_or(1.0) as f32;
    (u_axis / size("sw"), 0.0, v_axis / size("sh"), 0.0)
}

/// UV map that stretches the picture once over the face's own extent.
fn fit_to_face(corners: &[Vec3], normal: Vec3) -> (Vec3, f32, Vec3, f32) {
    let (u, v) = projection_basis(normal);
    let span = |a: Vec3| {
        let d: Vec<f32> = corners.iter().map(|p| a.dot(*p)).collect();
        let (lo, hi) = d.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        (lo, (hi - lo).max(1e-6))
    };
    let ((u0, uw), (v0, vh)) = (span(u), span(v));
    (u / uw, -u0 / uw, v / vh, -v0 / vh)
}

/// `core/texture.py: projection_basis`: `x = normalize(Z × n)`, `y = n × x`;
/// horizontal faces use (X, Y) looking up and (-X, Y) looking down.
fn projection_basis(n: Vec3) -> (Vec3, Vec3) {
    let n = n.normalize_or_zero();
    let (xx, xy) = (-n.y, n.x);
    let lx = (xx * xx + xy * xy).sqrt();
    if lx < 1e-3 {
        return if n.z > 0.0 { (Vec3::X, Vec3::Y) } else { (Vec3::NEG_X, Vec3::Y) };
    }
    let (xx, xy) = (xx / lx, xy / lx);
    (Vec3::new(xx, xy, 0.0), Vec3::new(-n.z * xy, n.z * xx, n.x * xy - n.y * xx))
}

/// A format-3 geometry block as JSON-like faces (`vertices`, `holes`, attrs).
fn binary_faces(meta: &Value, files: &HashMap<String, Vec<u8>>) -> Result<Vec<Value>, LoadError> {
    let bad = |why: &str| LoadError::Format(trf!("IngeTrazo-Geometrieblock beschädigt ({why})", "IngeTrazo geometry block damaged ({why})", why = why));
    let member = meta.get("member").and_then(Value::as_str).ok_or_else(|| bad("member"))?;
    let blob = files.get(member).ok_or_else(|| bad("fehlt"))?;
    let layout = meta.get("arrays").and_then(Value::as_object).ok_or_else(|| bad("layout"))?;
    // Every array as i64/f64, checked against the blob's bounds.
    let array = |name: &str| -> Result<Vec<f64>, LoadError> {
        let spec = layout.get(name).and_then(Value::as_array).ok_or_else(|| bad(name))?;
        let dtype = spec.first().and_then(Value::as_str).ok_or_else(|| bad(name))?;
        let count: usize = spec.get(1).and_then(Value::as_array).ok_or_else(|| bad(name))?.iter().map(|n| n.as_u64().unwrap_or(0) as usize).product();
        let offset = spec.get(2).and_then(Value::as_u64).ok_or_else(|| bad(name))? as usize;
        let size = match dtype {
            "<f4" | "<i4" => 4,
            "<i8" => 8,
            "|u1" => 1,
            _ => return Err(bad(name)),
        };
        let bytes = blob.get(offset..offset + count * size).ok_or_else(|| bad(name))?;
        Ok(bytes
            .chunks_exact(size)
            .map(|c| match dtype {
                "<f4" => f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64,
                "<i4" => i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64,
                "<i8" => i64::from_le_bytes(c.try_into().unwrap()) as f64,
                _ => c[0] as f64,
            })
            .collect())
    };
    let pos = array("pos")?;
    let rings = array("rings")?;
    let sizes = array("ring_sizes")?;
    let counts = array("ring_counts")?;
    let face_attr = array("face_attr")?;
    let table = meta.get("face_attrs").and_then(Value::as_array).cloned().unwrap_or_default();
    let point = |i: f64| -> Result<Value, LoadError> {
        let i = i as usize * 3;
        let p = pos.get(i..i + 3).ok_or_else(|| bad("pos"))?;
        Ok(Value::from(vec![p[0], p[1], p[2]]))
    };
    let (mut c, mut r) = (0usize, 0usize);
    let mut faces = Vec::with_capacity(counts.len());
    for (k, &n) in counts.iter().enumerate() {
        let mut loops = Vec::new();
        for _ in 0..n as usize {
            let len = *sizes.get(r).ok_or_else(|| bad("rings"))? as usize;
            let idx = rings.get(c..c + len).ok_or_else(|| bad("rings"))?;
            loops.push(Value::from(idx.iter().map(|&i| point(i)).collect::<Result<Vec<_>, _>>()?));
            c += len;
            r += 1;
        }
        let attr = face_attr.get(k).copied().unwrap_or(-1.0);
        let mut face = if attr >= 0.0 { table.get(attr as usize).cloned().unwrap_or_default() } else { Value::Object(Default::default()) };
        if !face.is_object() {
            face = Value::Object(Default::default());
        }
        let mut loops = loops.into_iter();
        face["vertices"] = loops.next().unwrap_or_default();
        let holes: Vec<Value> = loops.collect();
        if !holes.is_empty() {
            face["holes"] = Value::from(holes);
        }
        faces.push(face);
    }
    Ok(faces)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/banca-pergola-yanque.igz");

    #[test]
    fn ingetrazo_example() {
        let m = super::super::load(Path::new(EXAMPLE), 1.0, super::super::UpAxis::Z).unwrap();
        assert!(m.triangle_count() > 100, "{}", m.triangle_count());
        let parts: Vec<_> = m.objects.iter().flat_map(|o| &o.parts).collect();
        assert!(parts.iter().any(|p| p.texture.is_some()), "textures embedded in the document");
        // Its groups and components become objects, the face-me figure too.
        assert!(m.objects.len() >= 5, "{}", m.objects.len());
        let figure = m.objects.iter().find(|o| o.billboard).expect("a face-me figure");
        // Its picture spans the quad once: UVs from 0 to 1, not cut by tile borders.
        let uvs: Vec<_> = figure.parts.iter().flat_map(|p| &p.uvs).collect();
        assert!(uvs.iter().all(|uv| (-1e-3..=1.001).contains(&uv.x) && (-1e-3..=1.001).contains(&uv.y)), "{uvs:?}");
        assert!(figure.parts.iter().any(|p| p.texture.as_ref().is_some_and(|t| t.cutout)));
        assert!(m.objects.iter().all(|o| !o.edges.is_empty()));
        let (lo, hi) = m.bounds().unwrap();
        // A bench with a pergola, 2.3 m high; its footing reaches 0.5 m below ground.
        assert!(hi.z > 1.5 && hi.z < 5.0 && lo.z > -1.0, "{lo} {hi}");
        assert!(parts.iter().all(|p| p.positions.len() == p.normals.len() && (p.uvs.is_empty() || p.uvs.len() == p.positions.len())));
    }

    #[test]
    fn plain_json_with_group_transform() {
        let dir = std::env::temp_dir().join(format!("tapetum-igz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.igz");
        // A unit square as component, placed 5 m up; a hidden group is skipped.
        let doc = serde_json::json!({
            "igz_format": 1,
            "scene": {
                "faces": [],
                "protos": [{"faces": [{"vertices": [[0,0,0],[1,0,0],[1,1,0],[0,1,0]], "color": [1.0, 0.0, 0.0]}]}],
                "groups": [
                    {"proto": 0, "xform": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,5,1]},
                    {"proto": 0, "xform": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,9,1], "hidden": true}
                ]
            }
        });
        std::fs::write(&path, doc.to_string()).unwrap();
        let m = load(&path).unwrap();
        assert_eq!(m.triangle_count(), 2);
        assert_eq!(m.objects.len(), 1, "the hidden group is no object");
        assert_eq!(m.objects[0].name, "Komponente 1");
        let part = &m.objects[0].parts[0];
        assert!(part.positions.iter().all(|p| p.z == 5.0));
        assert_eq!(part.color, [1.0, 0.0, 0.0, 1.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn planar_basis_matches_ingetrazo() {
        // A wall facing -Y: x along +X, y up.
        let (u, v) = projection_basis(Vec3::NEG_Y);
        assert!((u - Vec3::X).length() < 1e-6 && (v - Vec3::Z).length() < 1e-6, "{u} {v}");
    }
}
