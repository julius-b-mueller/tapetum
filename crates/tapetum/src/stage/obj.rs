//! Wavefront OBJ with its MTL colors and diffuse textures (via `tobj`).

use crate::i18n::tr;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bevy::math::{Vec2, Vec3};

use super::{Builder, LoadError, Model, Part, Texture, decode_image, srgb_to_linear};

pub fn load(path: &Path) -> Result<Model, LoadError> {
    let opts = tobj::LoadOptions { triangulate: true, single_index: true, ..Default::default() };
    let dir = path.parent().unwrap_or(Path::new("."));
    let bytes = super::read_limited(path, super::MAX_MODEL_BYTES)?;
    // Material libraries through the same checks as the model itself.
    let (models, materials) = tobj::load_obj_buf(&mut std::io::Cursor::new(bytes), &opts, |mtl| {
        let data = super::read_limited(&dir.join(mtl), super::MAX_SIDE_BYTES).map_err(|_| tobj::LoadError::OpenFileFailed)?;
        tobj::load_mtl_buf(&mut std::io::Cursor::new(data))
    })
    .map_err(|e| LoadError::Format(format!("OBJ: {e}")))?;
    let materials = materials.unwrap_or_default();
    let mut textures: HashMap<String, Option<Arc<Texture>>> = HashMap::new();

    let mut b = Builder::default();
    for m in models {
        // OBJ "o"/"g" names: objects to move and override separately.
        let name = if m.name.is_empty() || m.name == "unnamed_object" { tr("Modell", "Model").to_string() } else { m.name.clone() };
        b.object(&format!("o:{name}"), &name);
        let mesh = m.mesh;
        let mat = mesh.material_id.and_then(|i| materials.get(i));
        let color = mat.and_then(|m| m.diffuse).map_or([0.7, 0.7, 0.7, 1.0], |d| [srgb_to_linear(d[0]), srgb_to_linear(d[1]), srgb_to_linear(d[2]), 1.0]);
        let texture = mat.and_then(|m| m.diffuse_texture.clone()).and_then(|name| {
            textures
                .entry(name.clone())
                .or_insert_with(|| super::read_limited(&dir.join(&name), super::MAX_SIDE_BYTES).ok().and_then(|b| decode_image(format!("obj:{name}"), &b)))
                .clone()
        });
        let positions: Vec<Vec3> = mesh.positions.as_chunks::<3>().0.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect();
        let mut normals: Vec<Vec3> = mesh.normals.as_chunks::<3>().0.iter().map(|n| Vec3::new(n[0], n[1], n[2])).collect();
        if normals.len() != positions.len() {
            normals = smooth_normals(&positions, &mesh.indices);
        }
        let uvs: Vec<Vec2> = if texture.is_some() && mesh.texcoords.len() / 2 == positions.len() {
            // OBJ's v grows upwards, images' downwards.
            mesh.texcoords.as_chunks::<2>().0.iter().map(|t| Vec2::new(t[0], 1.0 - t[1])).collect()
        } else {
            Vec::new()
        };
        let texture = if uvs.is_empty() { None } else { texture };
        // A textured material shows the picture; its color then only tints.
        let color = if texture.is_some() { [1.0, 1.0, 1.0, 1.0] } else { color };
        b.push(Part { positions, normals, uvs, indices: mesh.indices, color, texture });
    }
    Ok(b.finish())
}

/// Area-weighted vertex normals for files without their own.
pub(super) fn smooth_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; positions.len()];
    for t in indices.as_chunks::<3>().0 {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let face = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += face;
        }
    }
    n.into_iter().map(|v| v.normalize_or_zero()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_with_material() {
        let dir = std::env::temp_dir().join(format!("tapetum-obj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.mtl"), "newmtl red\nKd 1 0 0\n").unwrap();
        std::fs::write(
            dir.join("q.obj"),
            "mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nusemtl red\nf 1 2 3 4\n",
        )
        .unwrap();
        let m = load(&dir.join("q.obj")).unwrap();
        assert_eq!(m.triangle_count(), 2);
        assert_eq!(m.objects[0].parts[0].color, [1.0, 0.0, 0.0, 1.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
