//! Gobo images: built-in patterns and imported pictures, as projection masks.
//!
//! Bevy's spot light textures need binding arrays (not on Metal), so gobos are
//! applied by our own shaders: in the haze (`beam.wesl`) and on every surface
//! the beam lights (`surface.wesl`, all masks as one texture array).

use crate::i18n::tr;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use tapetum_project::Gobo;

pub const BUILTIN: [&str; 9] = ["Punkte", "Stern", "Linien", "Ring", "Dreieck", "Fenster", "Breakup", "Spirale", WATER];

/// Animated: drawn by the shaders as moving caustics, not from its picture.
pub const WATER: &str = "Wasser";

/// Display name of a built-in gobo (the German name is its key in project files).
pub fn label(name: &str) -> &str {
    if !crate::i18n::english() {
        return name;
    }
    match name {
        "Punkte" => "Dots",
        "Stern" => "Star",
        "Linien" => "Lines",
        "Ring" => "Ring",
        "Dreieck" => "Triangle",
        "Fenster" => "Window",
        "Breakup" => "Breakup",
        "Spirale" => "Spiral",
        WATER => "Water",
        other => other,
    }
}

pub fn is_water(gobo: &Gobo) -> bool {
    matches!(gobo, Gobo::Builtin(n) if n == WATER)
}
const SIZE: u32 = 128;

/// Loaded gobo masks with their open area (0..1, share of light passing).
#[derive(Resource, Default)]
pub struct GoboImages {
    map: HashMap<String, (Handle<Image>, f32)>,
    /// Masks in load order; a gobo's layer in the surface texture array is its
    /// index + 1 (layer 0 is open).
    masks: Vec<(String, Vec<u8>)>,
    /// A gobo was added since the texture array was last built.
    pub masks_changed: bool,
}

fn key(gobo: &Gobo) -> Option<String> {
    match gobo {
        Gobo::Open => None,
        Gobo::Builtin(n) => Some(format!("builtin:{n}")),
        Gobo::Image(p) => Some(format!("file:{p}")),
    }
}

impl GoboImages {
    /// Mask and transmission; `None` for an open slot or an unreadable image.
    pub fn get(&self, gobo: &Gobo) -> Option<&(Handle<Image>, f32)> {
        self.map.get(&key(gobo)?)
    }

    /// Makes sure `gobo` is loaded. Image paths are relative to `base`.
    pub fn ensure(&mut self, gobo: &Gobo, base: &Path, images: &mut Assets<Image>) -> Result<(), String> {
        let Some(k) = key(gobo) else { return Ok(()) };
        if self.map.contains_key(&k) {
            return Ok(());
        }
        let mask = match gobo {
            Gobo::Builtin(name) => builtin(name),
            Gobo::Image(rel) => load_mask(&base.join(rel))?,
            Gobo::Open => unreachable!(),
        };
        let open = mask.iter().map(|&v| v as f32 / 255.0).sum::<f32>() / disc_area();
        self.map.insert(k.clone(), (images.add(to_image(&mask)), open.clamp(0.02, 1.0)));
        self.masks.push((k, mask));
        self.masks_changed = true;
        Ok(())
    }

    /// Layer of `gobo` in [`GoboImages::array`].
    pub fn layer(&self, gobo: &Gobo) -> Option<u32> {
        let k = key(gobo)?;
        self.masks.iter().position(|(m, _)| *m == k).map(|i| i as u32 + 1)
    }

    /// All masks as one texture array, open first.
    pub fn array(&self) -> Image {
        let masks: Vec<&[u8]> = self.masks.iter().map(|(_, m)| m.as_slice()).collect();
        mask_array(&masks)
    }
}

/// Texture array: an open layer, then `masks`.
pub fn mask_array(masks: &[&[u8]]) -> Image {
    let open = vec![255u8; (SIZE * SIZE) as usize];
    let layers: Vec<&[u8]> = std::iter::once(open.as_slice()).chain(masks.iter().copied()).collect();
    let data: Vec<u8> = layers.iter().flat_map(|m| m.iter().flat_map(|&v| [v, v, v, 255])).collect();
    let mut img = Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: layers.len() as u32 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // An array even with a single layer.
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    img
}

/// Number of pixels inside the gobo aperture.
fn disc_area() -> f32 {
    (0..SIZE * SIZE).filter(|&i| uv(i).length() <= 1.0).count() as f32
}

/// Pixel index -> position in -1..1, y up.
fn uv(i: u32) -> Vec2 {
    let (x, y) = (i % SIZE, i / SIZE);
    Vec2::new((x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / SIZE as f32 * 2.0)
}

fn to_image(mask: &[u8]) -> Image {
    let data: Vec<u8> = mask.iter().flat_map(|&v| [v, v, v, 255]).collect();
    Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Soft edge over about one pixel for a signed distance (negative = inside).
fn coverage(d: f32) -> f32 {
    (0.5 - d * SIZE as f32 / 2.0).clamp(0.0, 1.0)
}

pub fn builtin(name: &str) -> Vec<u8> {
    (0..SIZE * SIZE)
        .map(|i| {
            let p = uv(i);
            // Everything outside the aperture (and its rim) stays black.
            let rim = coverage(p.length() - 0.94);
            let v = match name {
                "Punkte" => {
                    let centers = std::iter::once(Vec2::ZERO).chain((0..6).map(|k| Vec2::from_angle(k as f32 * std::f32::consts::TAU / 6.0) * 0.55));
                    centers.map(|c| coverage(p.distance(c) - 0.2)).fold(0.0, f32::max)
                }
                "Stern" => {
                    let a = p.y.atan2(p.x);
                    let r = 0.35 + 0.5 * (0.5 + 0.5 * (a * 5.0 + std::f32::consts::FRAC_PI_2 * 5.0).cos()).powf(3.0);
                    coverage(p.length() - r)
                }
                "Linien" => {
                    if ((p.x + 1.0) * 4.0).fract() < 0.5 { 1.0 } else { 0.0 }
                }
                "Ring" => coverage((p.length() - 0.62).abs() - 0.14),
                "Dreieck" => {
                    let n = [Vec2::new(0.0, -1.0), Vec2::new(0.866, 0.5), Vec2::new(-0.866, 0.5)];
                    let d = n.iter().map(|n| p.dot(*n) - 0.42).fold(f32::MIN, f32::max);
                    coverage(d)
                }
                "Fenster" => {
                    let bar = 0.07;
                    if p.x.abs() < bar || p.y.abs() < bar { 0.0 } else { 1.0 }
                }
                "Breakup" => {
                    let n = value_noise(p * 3.0) * 0.65 + value_noise(p * 7.0) * 0.35;
                    ((n - 0.45) * 12.0).clamp(0.0, 1.0)
                }
                // A still frame for the open-area estimate; the shaders animate it.
                WATER => {
                    let n = value_noise(p * 4.0);
                    let ridge = 1.0 - (n * 2.0 - 1.0).abs();
                    (0.12 + ridge.powf(6.0) * 1.5).min(1.0)
                }
                "Spirale" => {
                    let a = p.y.atan2(p.x) / std::f32::consts::TAU + 0.5;
                    let t = (p.length() * 3.0 - a).rem_euclid(1.0);
                    if t < 0.5 { 1.0 } else { 0.0 }
                }
                _ => 1.0,
            };
            (v * rim * 255.0).round() as u8
        })
        .collect()
}

fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

fn value_noise(p: Vec2) -> f32 {
    let (i, f) = (p.floor(), p.fract_gl());
    let s = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (i.x as i32, i.y as i32);
    let a = hash(x, y) + (hash(x + 1, y) - hash(x, y)) * s.x;
    let b = hash(x, y + 1) + (hash(x + 1, y + 1) - hash(x, y + 1)) * s.x;
    a + (b - a) * s.y
}

/// Picture -> mask: brightness passes light, cut to the round aperture.
fn load_mask(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = crate::stage::read_limited(path, crate::stage::MAX_SIDE_BYTES).map_err(|e| format!("{}: {e}", path.display()))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let img = img.resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Triangle).to_luma8();
    Ok((0..SIZE * SIZE)
        .map(|i| {
            let v = img.as_raw()[i as usize] as f32;
            (v * coverage(uv(i).length() - 0.94)).round() as u8
        })
        .collect())
}

/// Copies an imported picture next to the show so the project stays portable;
/// returns its path relative to `project_dir`.
pub fn import(file: &Path, project_dir: &Path) -> Result<String, String> {
    load_mask(file)?;
    let dir = project_dir.join("tapetum-gobos");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = file.file_name().ok_or(tr("kein Dateiname", "no file name"))?;
    let target: PathBuf = dir.join(name);
    if target != file {
        std::fs::copy(file, &target).map_err(|e| e.to_string())?;
    }
    Ok(format!("tapetum-gobos/{}", name.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_gobos_block_part_of_the_light() {
        let area = disc_area();
        for name in BUILTIN {
            let mask = builtin(name);
            let open = mask.iter().map(|&v| v as f32 / 255.0).sum::<f32>() / area;
            assert!((0.1..0.9).contains(&open), "{name}: {open}");
            // Corners are outside the aperture.
            assert_eq!(mask[0], 0, "{name}");
        }
    }
}
