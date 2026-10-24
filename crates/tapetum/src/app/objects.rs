//! Stage objects and scenes.
//!
//! A model file holds objects (IngeTrazo groups and components, glTF nodes,
//! OBJ objects). The base scene shows them as the file has them; a scene
//! moves, turns or hides objects relative to that, and follows the main
//! conductor cuelist when it runs. Materials come from the file and can be
//! overridden per object; the file is never written. Files are watched, so
//! saving in IngeTrazo updates the stage.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::{NotShadowCaster, TransmittedShadowReceiver};
use bevy::picking::mesh_picking::ray_cast::RayCastBackfaces;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use notify::Watcher;
use tapetum_project::{PartOverride, SceneObjectState, StageObject, Surface, UpAxis};

use super::beam::{SurfaceExt, SurfaceGobos, SurfaceMaterial};
use super::fixtures::{BkSpace, placement_rotation};
use super::gobo::GoboImages;
use super::state::{BkStatus, ShowState};
use crate::stage::{self, Model, ModelObject};

/// One object on stage; `key` is `"<file id>/<object key>"`.
#[derive(Component)]
pub struct StageObjectRoot {
    pub key: String,
}

/// The built-in platform, shown only while there are no stage objects.
#[derive(Component)]
pub struct Placeholder;

/// The ground plane; IngeTrazo scenes draw their own floor.
#[derive(Component)]
pub struct Floor;

/// A stage object locked in the sidebar: the 3D view doesn't pick it.
pub fn is_locked(state: &ShowState, key: &str) -> bool {
    split_key(key).is_some_and(|(file, part)| {
        state.project.objects.iter().find(|o| o.id == file).and_then(|f| f.parts.get(part)).and_then(|p| p.locked).unwrap_or(false)
    })
}

pub fn object_key(file: u64, part: &str) -> String {
    format!("{file}/{part}")
}

pub fn split_key(key: &str) -> Option<(u64, &str)> {
    let (f, p) = key.split_once('/')?;
    Some((f.parse().ok()?, p))
}

pub fn is_igz(o: &StageObject) -> bool {
    o.model.to_ascii_lowercase().ends_with(".igz")
}

/// What a loaded model depends on; any change reloads the file.
#[derive(Clone, Debug, PartialEq)]
struct FileKey {
    path: PathBuf,
    unit: f32,
    up: UpAxis,
    /// Bumped whenever the file changes on disk.
    version: u64,
}

struct Job {
    key: FileKey,
    rx: Mutex<mpsc::Receiver<Result<Model, String>>>,
}

/// What an object's meshes were built from.
#[derive(Clone, PartialEq)]
struct BuildKey {
    file: FileKey,
    over: PartOverride,
    /// Shadows also on the back of translucent surfaces (shadow play; High quality).
    back_shadows: bool,
}

struct Spawned {
    root: Entity,
    inner: Entity,
    build: BuildKey,
    /// Face-me figure: horizontal direction it faces in the base scene.
    billboard: Option<Vec3>,
}

#[derive(Resource)]
pub struct StageObjects {
    models: HashMap<u64, (FileKey, Arc<Model>)>,
    jobs: HashMap<u64, Job>,
    spawned: HashMap<String, Spawned>,
    /// Foot point (bottom center) of every object in the base scene, BK space.
    pivots: HashMap<String, Vec3>,
    pub errors: HashMap<u64, String>,
    versions: HashMap<PathBuf, u64>,
    watcher: Option<Mutex<notify::RecommendedWatcher>>,
    changes: Mutex<mpsc::Receiver<PathBuf>>,
    watched: HashSet<PathBuf>,
}

impl Default for StageObjects {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            if let Ok(ev) = ev
                && !matches!(ev.kind, notify::EventKind::Access(_))
            {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        })
        .ok()
        .map(Mutex::new);
        StageObjects {
            models: HashMap::new(),
            jobs: HashMap::new(),
            spawned: HashMap::new(),
            pivots: HashMap::new(),
            errors: HashMap::new(),
            versions: HashMap::new(),
            watcher,
            changes: Mutex::new(rx),
            watched: HashSet::new(),
        }
    }
}

impl StageObjects {
    pub fn loading(&self, file: u64) -> bool {
        self.jobs.contains_key(&file)
    }

    /// The objects of a loaded file.
    pub fn objects(&self, file: u64) -> &[ModelObject] {
        self.models.get(&file).map_or(&[], |(_, m)| &m.objects)
    }

    pub fn object(&self, key: &str) -> Option<&ModelObject> {
        let (file, part) = split_key(key)?;
        self.objects(file).iter().find(|o| o.key == part)
    }

    pub fn pivot(&self, key: &str) -> Option<Vec3> {
        self.pivots.get(key).copied()
    }

    /// The entity whose transform puts an object's meshes (and outline) in place.
    pub fn inner(&self, key: &str) -> Option<Entity> {
        self.spawned.get(key).map(|s| s.inner)
    }

    fn watch(&mut self, path: &Path) {
        let Some(dir) = path.parent().map(Path::to_path_buf) else { return };
        if self.watched.insert(dir.clone())
            && let Some(w) = &self.watcher
            && let Ok(mut w) = w.lock()
        {
            // The directory, not the file: editors save via a temp file and rename.
            let _ = w.watch(&dir, notify::RecursiveMode::NonRecursive);
        }
    }
}

/// Absolute path of an object's model (relative ones start at the project).
pub fn model_path(state: &ShowState, o: &StageObject) -> PathBuf {
    let p = PathBuf::from(&o.model);
    if p.is_absolute() { p } else { state.project_dir().join(p) }
}

/// How a file is stored: relative when it lies below the project.
pub fn stored_path(state: &ShowState, file: &Path) -> String {
    let dir = state.project_dir();
    file.strip_prefix(&dir).map_or_else(|_| file.to_string_lossy().into_owned(), |r| r.to_string_lossy().into_owned())
}

/// Loads, reloads and (re)builds the objects of all model files.
pub fn sync_objects(
    mut commands: Commands,
    state: Res<ShowState>,
    mut objs: ResMut<StageObjects>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    surface_gobos: Res<SurfaceGobos>,
    mut images: ResMut<Assets<Image>>,
    space: Query<Entity, With<BkSpace>>,
    mut placeholder: Query<&mut Visibility, (With<Placeholder>, Without<Floor>)>,
    mut floor: Query<&mut Visibility, (With<Floor>, Without<Placeholder>)>,
) {
    // Files that changed on disk get a new version, which reloads them.
    let changed: Vec<PathBuf> = objs.changes.lock().map(|rx| rx.try_iter().collect()).unwrap_or_default();
    for p in changed {
        let canon = std::fs::canonicalize(&p).unwrap_or(p);
        *objs.versions.entry(canon).or_insert(0) += 1;
    }

    for o in &state.project.objects {
        let path = model_path(&state, o);
        let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        objs.watch(&canon);
        let key = FileKey { path: path.clone(), unit: o.unit_scale, up: o.up, version: objs.versions.get(&canon).copied().unwrap_or(0) };
        let have = objs.models.get(&o.id).is_some_and(|(k, _)| *k == key);
        let loading = objs.jobs.get(&o.id).is_some_and(|j| j.key == key);
        if !have && !loading {
            let (tx, rx) = mpsc::channel();
            let (p, unit, up) = (path.clone(), o.unit_scale, o.up);
            std::thread::spawn(move || {
                // A broken file must not leave the object "loading" forever, even if a
                // parser library panics on it.
                let result = std::panic::catch_unwind(|| stage::load(&p, unit, up))
                    .unwrap_or_else(|_| Err(stage::LoadError::Format(crate::i18n::tr("Datei ist beschädigt", "The file is damaged").into())));
                let _ = tx.send(result.map_err(|e| e.to_string()));
            });
            objs.jobs.insert(o.id, Job { key, rx: Mutex::new(rx) });
        }
    }

    // Finished loads.
    let ids: Vec<u64> = objs.jobs.keys().copied().collect();
    for id in ids {
        let result = objs.jobs[&id].rx.lock().ok().and_then(|rx| rx.try_recv().ok());
        let Some(result) = result else { continue };
        let job = objs.jobs.remove(&id).unwrap();
        match result {
            Ok(model) => {
                objs.errors.remove(&id);
                objs.models.insert(id, (job.key, Arc::new(model)));
            }
            Err(e) => {
                objs.errors.insert(id, e);
            }
        }
    }

    // Everything that should exist now.
    let files: HashSet<u64> = state.project.objects.iter().map(|o| o.id).collect();
    objs.models.retain(|id, _| files.contains(id));
    objs.jobs.retain(|id, _| files.contains(id));
    let mut wanted: HashSet<String> = HashSet::new();
    let Ok(space) = space.single() else { return };
    for file in &state.project.objects {
        let Some((fkey, model)) = objs.models.get(&file.id).map(|(k, m)| (k.clone(), m.clone())) else { continue };
        let base = file_transform(file);
        for mo in &model.objects {
            let key = object_key(file.id, &mo.key);
            wanted.insert(key.clone());
            let over = file.parts.get(&mo.key).copied().unwrap_or_default();
            let back_shadows = state.project.render.quality == tapetum_project::Quality::High;
            let build = BuildKey { file: fkey.clone(), over, back_shadows };
            // Foot point in the base scene: scenes turn objects around it.
            let pivot = mo.bounds().map_or(Vec3::ZERO, |(lo, hi)| {
                let foot = Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, lo.z);
                base.transform_point(foot)
            });
            objs.pivots.insert(key.clone(), pivot);
            let billboard = mo.billboard.then(|| (base.rotation * mo.facing()).with_z(0.0).normalize_or(Vec3::NEG_Y));
            if let Some(s) = objs.spawned.get_mut(&key) {
                if s.build == build {
                    s.billboard = billboard;
                    commands.entity(s.inner).insert(Transform::from_translation(-pivot).mul_transform(base));
                    continue;
                }
                commands.entity(s.root).despawn();
            }
            let root = commands.spawn((StageObjectRoot { key: key.clone() }, Transform::from_translation(pivot), Visibility::default(), ChildOf(space), Name::new(mo.name.clone()))).id();
            let inner = commands.spawn((Transform::from_translation(-pivot).mul_transform(base), Visibility::default(), ChildOf(root))).id();
            build_object(&mut commands, inner, mo, &over, &mut meshes, &mut materials, &surface_gobos.ext(), &mut images, back_shadows);
            objs.spawned.insert(key, Spawned { root, inner, build, billboard });
        }
    }
    let gone: Vec<String> = objs.spawned.keys().filter(|k| !wanted.contains(*k)).cloned().collect();
    for k in gone {
        if let Some(s) = objs.spawned.remove(&k) {
            commands.entity(s.root).despawn();
        }
        objs.pivots.remove(&k);
    }

    let any = state.project.objects.iter().any(|o| o.visible);
    for mut v in &mut placeholder {
        v.set_if_neq(if any { Visibility::Hidden } else { Visibility::Inherited });
    }
    let igz = state.project.objects.iter().any(|o| o.visible && is_igz(o));
    for mut v in &mut floor {
        v.set_if_neq(if igz { Visibility::Hidden } else { Visibility::Inherited });
    }
}

/// Placement of a whole file (IngeTrazo files stay where IngeTrazo has them).
pub fn file_transform(o: &StageObject) -> Transform {
    if is_igz(o) {
        return Transform::IDENTITY;
    }
    let t = &o.transform;
    Transform::from_translation(Vec3::from(t.translation)).with_rotation(placement_rotation(t.rotation)).with_scale(Vec3::from(t.scale))
}

#[allow(clippy::too_many_arguments)]
fn build_object(
    commands: &mut Commands,
    parent: Entity,
    mo: &ModelObject,
    over: &PartOverride,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SurfaceMaterial>,
    ext: &SurfaceExt,
    images: &mut Assets<Image>,
    back_shadows: bool,
) {
    let mut textures: HashMap<String, Handle<Image>> = HashMap::new();
    let surface = over.surface.unwrap_or(Surface::Opaque);
    for part in &mo.parts {
        // Main world too: the mouse picks objects by ray casting their meshes.
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, part.positions.iter().map(|p| p.to_array()).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, part.normals.iter().map(|n| n.to_array()).collect::<Vec<_>>());
        let use_texture = part.texture.is_some() && over.color.is_none();
        if use_texture {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs.iter().map(|u| u.to_array()).collect::<Vec<_>>());
        }
        mesh.insert_indices(Indices::U32(part.indices.clone()));

        let texture = part.texture.as_ref().filter(|_| use_texture).map(|t| {
            textures
                .entry(t.key.clone())
                .or_insert_with(|| {
                    let mut img = Image::new(
                        Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
                        TextureDimension::D2,
                        t.rgba.clone(),
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    // Architectural textures tile.
                    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    });
                    images.add(img)
                })
                .clone()
        });
        let (rgb, alpha) = surface_color(part, over);
        let mut mat = StandardMaterial {
            base_color: Color::linear_rgba(rgb[0], rgb[1], rgb[2], alpha),
            base_color_texture: texture,
            perceptual_roughness: 0.85,
            // Models often have single faces seen from both sides (walls, flats).
            double_sided: true,
            cull_mode: None,
            ..default()
        };
        let mut casts_shadow = true;
        // Cut-out pictures (people, foliage): transparent pixels vanish, the rest is solid.
        let cutout = use_texture && part.texture.as_ref().is_some_and(|t| t.cutout);
        match surface {
            Surface::Opaque => {
                if cutout {
                    mat.alpha_mode = AlphaMode::Mask(0.5);
                } else if alpha < 0.99 {
                    mat.alpha_mode = AlphaMode::Blend;
                }
            }
            Surface::Translucent { transmission, diffusion } => {
                // Lit from the front like any surface; light from behind shines
                // through (diffusion), and with little diffusion you see through it.
                let see_through = transmission * (1.0 - diffusion);
                mat.diffuse_transmission = (transmission * diffusion).clamp(0.0, 1.0);
                if see_through < 0.1 && alpha > 0.99 {
                    // Practically not see-through (opera foil, frosted panels): drawn
                    // solid, so it is in the depth buffer and gobos land on it.
                    mat.alpha_mode = if cutout { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque };
                } else {
                    // See-through (gauze, glass): blended. It shows the gobos of the
                    // light spots on it like every surface (`surface.wesl`).
                    mat.base_color.set_alpha(alpha * (1.0 - see_through));
                    mat.alpha_mode = AlphaMode::Blend;
                }
                casts_shadow = transmission < 0.5;
            }
        }
        let mut e = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(SurfaceMaterial { base: mat, extension: ext.clone() })),
            Transform::default(),
            ChildOf(parent),
            // Drawn from both sides, so clicks on the back select it too.
            RayCastBackfaces,
        ));
        if back_shadows && matches!(surface, Surface::Translucent { diffusion, .. } if diffusion > 0.0) {
            // Light shining through (opera foil from behind) is shadowed too: shadow play.
            e.insert(TransmittedShadowReceiver);
        }
        if !casts_shadow {
            e.insert(NotShadowCaster);
        }
    }
}

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Linear base color (tint for textures) and alpha of a part after overrides.
fn surface_color(part: &stage::Part, over: &PartOverride) -> ([f32; 3], f32) {
    let c = part.color;
    let mut rgb = match over.color {
        Some(srgb) => srgb.map(srgb_to_linear),
        None => [c[0], c[1], c[2]],
    };
    if let Some(r) = over.reflectance {
        // What the eye sees: the tint times the texture's average.
        let seen = match (&part.texture, over.color) {
            (Some(t), None) => [rgb[0] * t.average[0], rgb[1] * t.average[1], rgb[2] * t.average[2]],
            _ => rgb,
        };
        let k = r / luminance(seen).max(1e-4);
        rgb = rgb.map(|v| (v * k).min(4.0));
    }
    (rgb, c[3])
}

/// What the eye sees of a part as the file has it: tint times texture average.
fn seen_color(p: &stage::Part) -> [f32; 3] {
    match &p.texture {
        Some(t) => [p.color[0] * t.average[0], p.color[1] * t.average[1], p.color[2] * t.average[2]],
        None => [p.color[0], p.color[1], p.color[2]],
    }
}

/// Reflectance of an object as the file has it (area-weighted by triangles).
pub fn file_reflectance(mo: &ModelObject) -> f32 {
    let total: usize = mo.parts.iter().map(|p| p.indices.len()).sum();
    mo.parts.iter().map(|p| luminance(seen_color(p)) * p.indices.len() as f32).sum::<f32>() / total.max(1) as f32
}

/// A color of the object as the file has it (its largest part), sRGB.
pub fn file_color(mo: &ModelObject) -> [f32; 3] {
    let part = mo.parts.iter().max_by_key(|p| p.indices.len());
    part.map_or([0.8; 3], |p| seen_color(p).map(linear_to_srgb))
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.max(0.0).powf(1.0 / 2.4) - 0.055 }
}

/// Rebuilds the gobo texture array when gobos were added and hands it to all surfaces.
pub fn sync_surface_masks(
    mut gobos: ResMut<GoboImages>,
    mut surface: ResMut<SurfaceGobos>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
) {
    if gobos.masks_changed {
        gobos.masks_changed = false;
        surface.masks = images.add(gobos.array());
    }
    let stale: Vec<AssetId<SurfaceMaterial>> = materials.iter().filter(|(_, m)| m.extension.masks != surface.masks).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.masks = surface.masks.clone();
        }
    }
}

/// Bounding spheres (world space) of the visible stage objects: the beams
/// that reach one of them get shadow maps.
#[derive(Resource, Default)]
pub struct Occluders(pub Vec<(Vec3, f32)>);

/// Boxes (Bevy world, min and max) around the visible stage objects, and
/// whether the object is locked: what the camera frames for a standard view.
#[derive(Resource, Default)]
pub struct FrameBoxes(pub Vec<(Vec3, Vec3, bool)>);

/// Which scene shows, and the move into it.
#[derive(Resource, Default)]
pub struct SceneRuntime {
    /// `None` = base scene.
    pub active: Option<u64>,
    /// The main cuelist decides (it runs in BK); otherwise the manual choice.
    pub following_cue: Option<f64>,
    /// A scene chosen by hand while the main cuelist runs, and the cue it was
    /// chosen at: shown until BK goes to another cue, then the cuelist decides again.
    pub detached: Option<(Option<u64>, f64)>,
    started: Option<f64>,
    duration: f32,
    from: HashMap<String, (Vec3, Quat)>,
    shown: HashMap<String, (Vec3, Quat)>,
    initialized: bool,
}

impl SceneRuntime {
    pub fn moving(&self) -> bool {
        self.started.is_some()
    }
}

/// An object's scene state in scene `scene` (`None` = base: unchanged).
pub fn scene_state(state: &ShowState, scene: Option<u64>, key: &str) -> SceneObjectState {
    scene
        .and_then(|id| state.project.scenes.iter().find(|s| s.id == id))
        .and_then(|s| s.objects.get(key).copied())
        .unwrap_or_default()
}

/// Picks the active scene: from the main cuelist's current cue while it is
/// on in BlinderKitten, otherwise the one chosen in the panel.
pub fn select_scene(mut rt: ResMut<SceneRuntime>, state: Res<ShowState>, bk: Res<BkStatus>, time: Res<Time>) {
    bk.set_conductor_path(state.show.conductor().map(|c| c.short_name.clone()));
    let live = bk.conductor().filter(|c| c.on && bk.info().is_some_and(|i| i.has_open(&state.show_path)));
    let following = live.map(|c| c.current_id);
    if rt.detached.is_some_and(|(_, at)| following != Some(at)) {
        rt.detached = None;
    }
    let exists = |id: &u64| state.project.scenes.iter().any(|s| s.id == *id);
    let active = match (following, rt.detached) {
        (Some(_), Some((scene, _))) => scene.filter(exists),
        (Some(cue), None) => state.project.scene_for_cue(cue),
        (None, _) => state.manual_scene.filter(exists),
    };
    if rt.following_cue != following {
        rt.following_cue = following;
    }
    if rt.active != active || !rt.initialized {
        let scene = |id: Option<u64>| id.and_then(|id| state.project.scenes.iter().find(|s| s.id == id));
        // Into a scene with its own time; back to the base with the time of the scene left.
        let duration = scene(active).or(scene(rt.active)).map_or(0.0, |s| s.transition);
        let first = !rt.initialized;
        rt.initialized = true;
        rt.active = active;
        rt.duration = duration;
        if first || duration <= 0.0 {
            rt.started = None;
            rt.from.clear();
        } else {
            rt.from = rt.shown.clone();
            rt.started = Some(time.elapsed_secs_f64());
        }
    }
}

/// The visible stage objects of a scene as they stand there: the object and
/// its transform from model to BK space; face-me figures turned towards
/// `camera` (BK space), and whether it is one. For exports; `printed_only`
/// leaves out what isn't printed.
pub fn placed<'a>(state: &ShowState, objs: &'a StageObjects, scene: Option<u64>, camera: Vec3, printed_only: bool) -> Vec<(&'a ModelObject, Mat4, bool)> {
    let mut out = Vec::new();
    for file in state.project.objects.iter().filter(|f| f.visible) {
        let base = file_transform(file).to_matrix();
        for mo in objs.objects(file.id) {
            let key = format!("{}/{}", file.id, mo.key);
            let s = scene_state(state, scene, &key);
            let part = file.parts.get(&mo.key);
            let printed = !printed_only || part.and_then(|p| p.printed).unwrap_or(true);
            let shown = printed && s.visible && part.and_then(|p| p.visible).unwrap_or(true);
            let Some(pivot) = objs.pivot(&key).filter(|_| shown) else { continue };
            let pos = pivot + Vec3::from(s.offset);
            let mut rot = placement_rotation(s.rotation);
            let billboard = objs.spawned.get(&key).and_then(|sp| sp.billboard);
            if let Some(facing) = billboard {
                let to_cam = (camera - pos).with_z(0.0);
                if to_cam.length_squared() > 1e-6 {
                    rot = Quat::from_rotation_z(to_cam.y.atan2(to_cam.x) - facing.y.atan2(facing.x)) * rot;
                }
            }
            out.push((mo, Mat4::from_rotation_translation(rot, pos) * Mat4::from_translation(-pivot) * base, billboard.is_some()));
        }
    }
    out
}

/// Puts every object where the active scene wants it, gliding during a transition.
pub fn apply_scene(
    mut rt: ResMut<SceneRuntime>,
    state: Res<ShowState>,
    objs: Res<StageObjects>,
    time: Res<Time>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut roots: Query<(&StageObjectRoot, &mut Transform, &mut Visibility)>,
    (mut occluders, mut frames): (ResMut<Occluders>, ResMut<FrameBoxes>),
) {
    let mut spheres = Vec::new();
    let mut boxes = Vec::new();
    // The camera in BK space (Z up), for the face-me figures.
    let cam = camera.single().ok().map(|c| {
        let p = c.translation();
        Vec3::new(p.x, -p.z, p.y)
    });
    let t = match rt.started {
        Some(start) => {
            let x = ((time.elapsed_secs_f64() - start) as f32 / rt.duration.max(1e-3)).clamp(0.0, 1.0);
            if x >= 1.0 {
                rt.started = None;
            }
            // Ease in and out like a motorized flat.
            x * x * (3.0 - 2.0 * x)
        }
        None => 1.0,
    };
    let active = rt.active;
    let mut shown = HashMap::new();
    for (root, mut tf, mut vis) in &mut roots {
        let Some(pivot) = objs.pivot(&root.key) else { continue };
        let s = scene_state(&state, active, &root.key);
        let target = (pivot + Vec3::from(s.offset), placement_rotation(s.rotation));
        let (pos, rot) = match rt.from.get(&root.key).filter(|_| t < 1.0) {
            Some(&(p0, r0)) => (p0.lerp(target.0, t), r0.slerp(target.1, t)),
            None => target,
        };
        shown.insert(root.key.clone(), (pos, rot));
        // Face-me figures turn around the vertical to look at the camera.
        let rot = match (objs.spawned.get(&root.key).and_then(|s| s.billboard), cam) {
            (Some(facing), Some(cam)) => {
                let to_cam = (cam - pos).with_z(0.0);
                let yaw = to_cam.y.atan2(to_cam.x) - facing.y.atan2(facing.x);
                if to_cam.length_squared() > 1e-6 { Quat::from_rotation_z(yaw) * rot } else { rot }
            }
            _ => rot,
        };
        tf.set_if_neq(Transform { translation: pos, rotation: rot, scale: Vec3::ONE });
        let visible = split_key(&root.key).is_some_and(|(file, part)| {
            state.project.objects.iter().find(|o| o.id == file).is_some_and(|f| f.visible && f.parts.get(part).and_then(|p| p.visible).unwrap_or(true))
        }) && s.visible;
        vis.set_if_neq(if visible { Visibility::Inherited } else { Visibility::Hidden });
        if visible
            && let (Some(mo), Some((file, _))) = (objs.object(&root.key), split_key(&root.key))
            && let (Some((lo, hi)), Some(f)) = (mo.bounds(), state.project.objects.iter().find(|o| o.id == file))
        {
            let base = file_transform(f);
            let center = pos + rot * (base.transform_point((lo + hi) / 2.0) - pivot);
            let radius = (hi - lo).length() / 2.0 * base.scale.max_element();
            // BK (Z up) to Bevy world (Y up).
            let bevy = |p: Vec3| Vec3::new(p.x, p.z, -p.y);
            spheres.push((bevy(center), radius));
            let (mut bmin, mut bmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for i in 0..8 {
                let corner = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
                let w = bevy(pos + rot * (base.transform_point(corner) - pivot));
                (bmin, bmax) = (bmin.min(w), bmax.max(w));
            }
            let locked = split_key(&root.key).and_then(|(_, part)| f.parts.get(part)).and_then(|p| p.locked).unwrap_or(false);
            boxes.push((bmin, bmax, locked));
        }
    }
    rt.shown = shown;
    occluders.0 = spheres;
    frames.0 = boxes;
}

const SELECTED: Color = Color::srgb(1.0, 0.75, 0.15);

/// Outlines the selected stage objects (their creases and borders).
pub fn draw_selection(state: Res<ShowState>, objs: Res<StageObjects>, transforms: Query<&GlobalTransform>, mut gizmos: Gizmos) {
    for key in &state.selected_objects {
        let (Some(mo), Some(inner)) = (objs.object(key), objs.inner(key)) else { continue };
        let Ok(gt) = transforms.get(inner) else { continue };
        for [a, b] in &mo.edges {
            gizmos.line(gt.transform_point(*a), gt.transform_point(*b), SELECTED);
        }
    }
}
