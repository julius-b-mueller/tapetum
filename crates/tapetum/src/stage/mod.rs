//! Loading stage models (IngeTrazo `.igz`, glTF/GLB, OBJ, STL) into plain
//! triangle lists in BK coordinates (meters, Z up). No Bevy types here except
//! math, so the loaders run on a background thread and are easy to test.

mod gltf;
mod igz;
mod obj;
mod stl;
mod triangulate;

use std::path::Path;

use bevy::math::{Vec2, Vec3};
pub use tapetum_project::UpAxis;

/// Texture pixels, RGBA8 (sRGB).
#[derive(Clone, Debug)]
pub struct Texture {
    /// Identifies the picture so parts can share it.
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Average linear color, for the reflectance of textured surfaces.
    pub average: [f32; 3],
    /// Has transparent pixels (cut-out figures, foliage).
    pub cutout: bool,
}

/// Triangles with one material.
#[derive(Clone, Debug, Default)]
pub struct Part {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// Empty without a texture.
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
    /// Linear RGBA.
    pub color: [f32; 4],
    pub texture: Option<std::sync::Arc<Texture>>,
}

/// One object of a file: an IngeTrazo group or component, a glTF node, an
/// OBJ object. Overrides and scenes address it by `key`.
#[derive(Clone, Debug, Default)]
pub struct ModelObject {
    pub key: String,
    pub name: String,
    pub parts: Vec<Part>,
    /// Turns around the vertical axis to face the camera (IngeTrazo's face-me figures).
    pub billboard: bool,
    /// Outline for showing the selection: creases and open borders.
    pub edges: Vec<[Vec3; 2]>,
}

impl ModelObject {
    /// Horizontal direction the object's surfaces face (area-weighted), for billboards.
    pub fn facing(&self) -> Vec3 {
        let mut n = Vec3::ZERO;
        for p in &self.parts {
            for t in p.indices.as_chunks::<3>().0 {
                let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
                let f = (b - a).cross(c - a);
                // Both sides of a figure count the same way.
                n += if f.dot(n) < 0.0 { -f } else { f };
            }
        }
        let flat = Vec3::new(n.x, n.y, 0.0);
        if flat.length_squared() > 1e-12 { flat.normalize() } else { Vec3::NEG_Y }
    }

    /// Creases (faces meeting at more than ~20°) and open borders.
    fn compute_edges(&mut self) {
        use std::collections::HashMap;
        let q = |v: Vec3| (v * 1000.0).round().as_ivec3();
        let mut map: HashMap<(bevy::math::IVec3, bevy::math::IVec3), (Vec3, Vec3, Vec3, u32)> = HashMap::new();
        for p in &self.parts {
            for t in p.indices.as_chunks::<3>().0 {
                let v = t.map(|i| p.positions[i as usize]);
                let n = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero();
                for (a, b) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
                    let (ka, kb) = (q(a), q(b));
                    let key = if (ka.x, ka.y, ka.z) <= (kb.x, kb.y, kb.z) { (ka, kb) } else { (kb, ka) };
                    let e = map.entry(key).or_insert((a, b, n, 0));
                    if e.3 == 1 {
                        // Second face on this edge: keep it only at a crease.
                        e.2 = if e.2.dot(n).abs() < 0.94 { Vec3::ZERO } else { Vec3::ONE };
                    }
                    e.3 += 1;
                }
            }
        }
        const MAX: usize = 40_000;
        self.edges = map.into_values().filter(|e| e.3 == 1 || (e.3 == 2 && e.2 == Vec3::ZERO)).take(MAX).map(|e| [e.0, e.1]).collect();
    }

    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.indices.len() / 3).sum()
    }

    /// Bounding box (min, max) in BK coordinates.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        bounds(self.parts.iter().flat_map(|p| &p.positions))
    }
}

fn bounds<'a>(mut it: impl Iterator<Item = &'a Vec3>) -> Option<(Vec3, Vec3)> {
    let first = *it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub objects: Vec<ModelObject>,
}

#[cfg(test)]
impl Model {
    pub fn triangle_count(&self) -> usize {
        self.objects.iter().map(ModelObject::triangle_count).sum()
    }

    /// Bounding box (min, max) in BK coordinates.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        bounds(self.objects.iter().flat_map(|o| &o.parts).flat_map(|p| &p.positions))
    }
}

impl Model {
    /// Model units to meters and the file's up axis to BK's Z.
    fn normalize(&mut self, unit: f32, up: UpAxis) {
        let convert = |v: Vec3| match up {
            UpAxis::Z => v,
            // Y up (right-handed): Y becomes Z, Z (towards the viewer) becomes -Y.
            UpAxis::Y => Vec3::new(v.x, -v.z, v.y),
        };
        for part in self.objects.iter_mut().flat_map(|o| &mut o.parts) {
            for p in &mut part.positions {
                *p = convert(*p) * unit;
            }
            for n in &mut part.normals {
                *n = convert(*n).normalize_or_zero();
            }
        }
    }
}

/// Collects triangles per object and material while a loader walks a file.
#[derive(Default)]
struct Builder {
    objects: Vec<ModelObject>,
    index: std::collections::HashMap<(usize, String, [u32; 4]), usize>,
    current: usize,
}

impl Builder {
    /// Following triangles belong to object `key` (created on first use).
    fn object(&mut self, key: &str, name: &str) {
        self.current = match self.objects.iter().position(|o| o.key == key) {
            Some(i) => i,
            None => {
                self.objects.push(ModelObject { key: key.to_string(), name: name.to_string(), ..Default::default() });
                self.objects.len() - 1
            }
        };
    }

    fn billboard(&mut self) {
        if let Some(o) = self.objects.get_mut(self.current) {
            o.billboard = true;
        }
    }

    fn part(&mut self, color: [f32; 4], texture: Option<std::sync::Arc<Texture>>) -> &mut Part {
        if self.objects.is_empty() {
            self.object("", "Modell");
        }
        let o = self.current;
        let key = (o, texture.as_ref().map_or_else(String::new, |t| t.key.clone()), color.map(f32::to_bits));
        let parts = &mut self.objects[o].parts;
        let i = *self.index.entry(key).or_insert_with(|| {
            parts.push(Part { color, texture, ..Default::default() });
            parts.len() - 1
        });
        &mut parts[i]
    }

    /// Adds finished triangles (already with normals/uvs) to the current object.
    fn push(&mut self, part: Part) {
        if self.objects.is_empty() {
            self.object("", "Modell");
        }
        self.objects[self.current].parts.push(part);
    }

    /// Adds one flat-shaded polygon (with holes); `uv` maps a corner to texture coordinates.
    fn polygon(&mut self, outer: &[Vec3], holes: &[Vec<Vec3>], color: [f32; 4], texture: Option<std::sync::Arc<Texture>>, uv: Option<&dyn Fn(Vec3) -> Vec2>) {
        let tris = triangulate::triangulate(outer, holes);
        if tris.is_empty() {
            return;
        }
        let corners: Vec<Vec3> = outer.iter().chain(holes.iter().flatten()).copied().collect();
        let normal = {
            let t = tris[0];
            (corners[t[1] as usize] - corners[t[0] as usize]).cross(corners[t[2] as usize] - corners[t[0] as usize]).normalize_or_zero()
        };
        let textured = texture.is_some();
        let part = self.part(color, texture);
        let base = part.positions.len() as u32;
        for &c in &corners {
            part.positions.push(c);
            part.normals.push(normal);
            if textured {
                part.uvs.push(uv.map_or(Vec2::ZERO, |f| f(c)));
            }
        }
        part.indices.extend(tris.iter().flat_map(|t| t.map(|i| base + i)));
    }

    fn finish(mut self) -> Model {
        for o in &mut self.objects {
            o.parts.retain(|p| !p.indices.is_empty());
        }
        self.objects.retain(|o| !o.parts.is_empty());
        Model { objects: self.objects }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("Dateiformat .{0} wird nicht unterstützt (glb, gltf, obj, stl, igz)")]
    Unsupported(String),
    #[error("{0}")]
    Format(String),
}

/// Formats the import dialog offers.
pub const EXTENSIONS: [&str; 5] = ["igz", "glb", "gltf", "obj", "stl"];

/// Sensible unit and up axis for a file: glTF is meters and Y up by spec,
/// IngeTrazo meters and Z up, STL mostly millimeters from CAD, OBJ meters Y up.
pub fn defaults_for(path: &Path) -> (f32, UpAxis) {
    match ext(path).as_str() {
        "stl" => (0.001, UpAxis::Z),
        "obj" => (1.0, UpAxis::Y),
        "glb" | "gltf" => (1.0, UpAxis::Y),
        _ => (1.0, UpAxis::Z),
    }
}

fn ext(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Loads `path` and converts it to BK coordinates with the given unit
/// (meters per model unit) and up axis. glTF is always Y up.
pub fn load(path: &Path, unit: f32, up: UpAxis) -> Result<Model, LoadError> {
    let (mut model, up) = match ext(path).as_str() {
        "igz" => (igz::load(path)?, up),
        "glb" | "gltf" => (gltf::load(path)?, UpAxis::Y),
        "obj" => (obj::load(path)?, up),
        "stl" => (stl::load(path)?, up),
        other => return Err(LoadError::Unsupported(other.to_string())),
    };
    model.normalize(unit, up);
    for o in &mut model.objects {
        o.compute_edges();
    }
    Ok(model)
}

/// Largest model file read (a whole .igz/.glb/.obj/.stl).
pub const MAX_MODEL_BYTES: u64 = 1 << 30;
/// Largest file a model refers to (texture, buffer, material library).
pub const MAX_SIDE_BYTES: u64 = 256 << 20;

/// Reads a file only if it is a regular file of at most `max` bytes. Files
/// from unknown sources name other files (textures, buffers); this keeps a
/// name like `/dev/zero` or a named pipe from hanging or exhausting memory.
pub fn read_limited(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(Error::new(ErrorKind::InvalidInput, format!("{} is not a regular file", path.display())));
    }
    if meta.len() > max {
        return Err(Error::new(ErrorKind::InvalidInput, format!("{} is larger than {} MB", path.display(), max >> 20)));
    }
    let mut data = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)?.take(max + 1).read_to_end(&mut data)?;
    if data.len() as u64 > max {
        return Err(Error::new(ErrorKind::InvalidInput, format!("{} grew while reading", path.display())));
    }
    Ok(data)
}

/// sRGB (0..1) to linear.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn decode_image(key: String, bytes: &[u8]) -> Option<std::sync::Arc<Texture>> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(texture(key, img.width(), img.height(), img.into_raw()))
}

fn texture(key: String, width: u32, height: u32, rgba: Vec<u8>) -> std::sync::Arc<Texture> {
    // A sparse sample is plenty for an average.
    let px: Vec<&[u8]> = rgba.chunks(4).step_by(((rgba.len() / 4) / 4096).max(1)).collect();
    let n = px.len().max(1) as f32;
    let mut average = [0.0f32; 3];
    let mut cutout = false;
    for p in &px {
        for (a, &c) in average.iter_mut().zip(p.iter()) {
            *a += srgb_to_linear(c as f32 / 255.0) / n;
        }
        cutout |= p.get(3).is_some_and(|&a| a < 128);
    }
    std::sync::Arc::new(Texture { key, width, height, rgba, average, cutout })
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn only_regular_files_of_sane_size() {
        let dir = std::env::temp_dir().join(format!("tapetum-limits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.bin");
        std::fs::write(&f, [1u8; 100]).unwrap();
        assert_eq!(read_limited(&f, 100).unwrap().len(), 100);
        assert!(read_limited(&f, 99).is_err(), "too large");
        assert!(read_limited(&dir, 1 << 20).is_err(), "a directory");
        #[cfg(unix)]
        assert!(read_limited(Path::new("/dev/zero"), 1 << 20).is_err(), "a device never ends");
    }

    #[cfg(unix)]
    #[test]
    fn obj_naming_a_device_still_loads() {
        let dir = std::env::temp_dir().join(format!("tapetum-obj-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("evil.obj");
        std::fs::write(&f, "mtllib /dev/zero\nusemtl m\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        // Without the checks this reads /dev/zero forever.
        let m = load(&f, 1.0, UpAxis::Y).unwrap();
        assert_eq!(m.triangle_count(), 1);
    }

    #[test]
    fn zip_entries_are_limited() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("tapetum-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("bomb.igz");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&f).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("document.json", opts).unwrap();
        zip.write_all(br#"{"igz_format": 3, "scene": {"groups": []}}"#).unwrap();
        // Compresses to almost nothing, unpacks to more than the limit.
        zip.start_file("geom/huge.bin", opts).unwrap();
        let chunk = vec![0u8; 1 << 20];
        for _ in 0..(MAX_SIDE_BYTES >> 20) + 1 {
            zip.write_all(&chunk).unwrap();
        }
        zip.finish().unwrap();
        assert!(std::fs::metadata(&f).unwrap().len() < 2 << 20);
        let err = load(&f, 1.0, UpAxis::Z).unwrap_err().to_string();
        assert!(err.contains("huge.bin"), "{err}");
    }
}
