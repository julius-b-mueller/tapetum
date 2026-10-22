//! glTF 2.0 (`.gltf` with buffers, or `.glb`): the default scene with its
//! node transforms, base colors and base color textures.

use std::path::Path;
use std::sync::Arc;

use bevy::math::{Mat4, Vec2, Vec3};

use super::{Builder, LoadError, Model, Part, Texture};

pub fn load(path: &Path) -> Result<Model, LoadError> {
    check_references(path)?;
    let (doc, buffers, images) = ::gltf::import(path).map_err(|e| LoadError::Format(format!("glTF: {e}")))?;
    let textures: Vec<Option<Arc<Texture>>> = images.iter().enumerate().map(|(i, img)| to_rgba(format!("gltf:{i}"), img)).collect();
    let mut b = Builder::default();
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    // Every top-level node is an object of its own.
    for node in scene.into_iter().flat_map(|s| s.nodes()) {
        let name = node.name().map_or_else(|| trf!("Knoten {}", "Node {}", node.index()), str::to_string);
        b.object(&format!("n:{}", node.index()), &name);
        walk(&node, Mat4::IDENTITY, &buffers, &textures, &mut b);
    }
    Ok(b.finish())
}

/// Every file the glTF names (buffers, images) must be a regular file of a
/// sane size before `gltf::import` reads it whole.
fn check_references(path: &Path) -> Result<(), LoadError> {
    let bytes = super::read_limited(path, super::MAX_MODEL_BYTES)?;
    let doc = ::gltf::Gltf::from_slice(&bytes).map_err(|e| LoadError::Format(format!("glTF: {e}")))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let uris = doc
        .buffers()
        .filter_map(|b| match b.source() {
            ::gltf::buffer::Source::Uri(u) => Some(u.to_string()),
            ::gltf::buffer::Source::Bin => None,
        })
        .chain(doc.images().filter_map(|i| match i.source() {
            ::gltf::image::Source::Uri { uri, .. } => Some(uri.to_string()),
            ::gltf::image::Source::View { .. } => None,
        }));
    for uri in uris {
        if uri.starts_with("data:") {
            continue;
        }
        let file = match uri.strip_prefix("file://").or_else(|| uri.strip_prefix("file:")) {
            Some(p) => std::path::PathBuf::from(p),
            None if uri.contains(':') => continue, // unsupported scheme: gltf refuses it
            None => base.join(percent_decode(&uri).ok_or_else(|| LoadError::Format(format!("glTF: {uri}")))?),
        };
        let meta = std::fs::metadata(&file)?;
        if !meta.is_file() || meta.len() > super::MAX_SIDE_BYTES {
            return Err(LoadError::Format(format!("glTF: {}", file.display())));
        }
    }
    Ok(())
}

/// `%xx` escapes of a relative URI; `None` if they don't make UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn walk(node: &::gltf::Node, parent: Mat4, buffers: &[::gltf::buffer::Data], textures: &[Option<Arc<Texture>>], parts: &mut Builder) {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        let normal_matrix = world.inverse().transpose();
        for prim in mesh.primitives() {
            if prim.mode() != ::gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(pos) = reader.read_positions() else { continue };
            let positions: Vec<Vec3> = pos.map(|p| world.transform_point3(Vec3::from(p))).collect();
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            let normals: Vec<Vec3> = match reader.read_normals() {
                Some(n) => n.map(|n| normal_matrix.transform_vector3(Vec3::from(n)).normalize_or_zero()).collect(),
                None => super::obj::smooth_normals(&positions, &indices),
            };
            let material = prim.material();
            let pbr = material.pbr_metallic_roughness();
            let color = pbr.base_color_factor();
            let texture = pbr.base_color_texture().and_then(|t| textures.get(t.texture().source().index()).cloned().flatten());
            let uvs: Vec<Vec2> = match (&texture, reader.read_tex_coords(0)) {
                (Some(_), Some(t)) => t.into_f32().map(Vec2::from).collect(),
                _ => Vec::new(),
            };
            let texture = if uvs.len() == positions.len() { texture } else { None };
            let uvs = if texture.is_some() { uvs } else { Vec::new() };
            parts.push(Part { positions, normals, uvs, indices, color, texture });
        }
    }
    for child in node.children() {
        walk(&child, world, buffers, textures, parts);
    }
}

fn to_rgba(key: String, img: &::gltf::image::Data) -> Option<Arc<Texture>> {
    use ::gltf::image::Format;
    let px = img.pixels.as_slice();
    let rgba: Vec<u8> = match img.format {
        Format::R8G8B8A8 => px.to_vec(),
        Format::R8G8B8 => px.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8G8 => px.as_chunks::<2>().0.iter().flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        Format::R8 => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        _ => return None,
    };
    Some(super::texture(key, img.width, img.height, rgba))
}
