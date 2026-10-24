//! Cameras, stage floor and the render settings that apply live.

use std::f32::consts::FRAC_PI_2;

use bevy::camera::{Exposure, Hdr, RenderTarget};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::light::{DirectionalLightShadowMap, GlobalAmbientLight, ShadowFilteringMethod};
use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::render::view::Tonemapping;
use bevy_egui::PrimaryEguiContext;
use tapetum_project::{Quality, RenderSettings};

use super::beam::{SurfaceGobos, SurfaceMaterial};
use super::fixtures::BkSpace;
use super::orbit::OrbitCamera;
use super::state::ShowState;

/// The 3D view is rendered into this image, at a resolution independent of
/// the UI; the UI paints it full screen behind the panels.
#[derive(Resource)]
pub struct View3d {
    pub image: Handle<Image>,
    /// The app icon, for the about window.
    pub logo: Handle<Image>,
}

const ICON: &[u8] = include_bytes!("../../assets/icon-256.png");

/// Window icon on Windows and Linux (macOS takes it from the app bundle).
pub fn set_window_icon(_main_thread: bevy::ecs::system::NonSendMarker, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Ok(img) = image::load_from_memory(ICON) else { return };
    let img = img.to_rgba8();
    let (w, h) = img.dimensions();
    let Ok(icon) = winit::window::Icon::from_rgba(img.into_raw(), w, h) else { return };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        for window in windows.windows.values() {
            window.set_window_icon(Some(icon.clone()));
            *done = true;
        }
    });
}

fn logo_image() -> Image {
    let img = image::load_from_memory(ICON).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let (w, h) = img.dimensions();
    Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, bevy::render::render_resource::TextureDimension::D2, img.into_raw(), TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::RENDER_WORLD)
}

/// One of the house lights: wide downlights under the ceiling, 3 x 2 over
/// whatever the scene holds (see [`place_house_lights`]), warm white, without shadows.
#[derive(Component)]
pub struct HouseLight(usize);

const HOUSE_LIGHTS: usize = 6;
/// Illuminance on the floor at full house light (lux).
const HOUSE_LUX: f32 = 400.0;

/// The house light fader's level as light: square law like a dimmer curve,
/// so the lower half (where it is used with stage light) is fine-grained.
fn house_level(fader: f32) -> f32 {
    fader.clamp(0.0, 1.0).powi(2)
}

/// The house light's colour: the project's, else warm white (3000 K).
pub fn house_color(r: &tapetum_project::RenderSettings) -> Color {
    let [cr, cg, cb] = r.house_color.unwrap_or_else(|| bk_olga::light::kelvin_to_rgb(3000.0));
    Color::linear_rgb(cr, cg, cb)
}

/// Lumens per house light at `height` above the floor for a slider value
/// 0..1. Bevy spreads a spot light's lumens like a point light's: I = lm / 4π;
/// about three lights reach a point on the floor.
fn house_lumens(level: f32, height: f32) -> f32 {
    level * HOUSE_LUX * height * height / 3.0 * 4.0 * std::f32::consts::PI
}

/// Where the house lights hang for a scene box (Bevy world): a grid over its
/// middle, spread with its size, under a ceiling a bit above the highest point.
/// Returns the positions and the height above the floor.
fn house_layout(lo: Vec3, hi: Vec3) -> ([Vec3; HOUSE_LIGHTS], f32) {
    let center = (lo + hi) / 2.0;
    let floor = lo.y;
    let height = (hi.y - floor + 2.0).max(6.0);
    let (sx, sz) = (((hi.x - lo.x) / 3.0).max(4.0), ((hi.z - lo.z) / 4.0).max(3.0));
    let mut spots = [Vec3::ZERO; HOUSE_LIGHTS];
    for (i, s) in spots.iter_mut().enumerate() {
        let (col, row) = ((i % 3) as f32 - 1.0, if i < 3 { -1.0 } else { 1.0 });
        *s = Vec3::new(center.x + col * sx, floor + height, center.z + row * sz);
    }
    (spots, height)
}

/// Bloom's blur passes cost weak integrated graphics more than they give.
fn bloom_on(r: &RenderSettings) -> bool {
    r.bloom && r.quality > Quality::Low
}

/// Keeps the house lights over the scene as fixtures and objects move.
pub fn place_house_lights(
    state: Res<ShowState>,
    fixtures: Query<&GlobalTransform, With<super::fixtures::FixtureRoot>>,
    occluders: Res<super::objects::Occluders>,
    placeholder: Query<&ViewVisibility, With<super::objects::Placeholder>>,
    mut house: Query<(&HouseLight, &mut SpotLight, &mut Transform, &mut Visibility)>,
) {
    let (lo, hi) = super::orbit::scene_bounds(&fixtures, &occluders, placeholder.iter().any(|v| v.get()));
    let (spots, height) = house_layout(lo, hi);
    // On Low (and Draft) the ambient light stands in (`apply_render_settings`).
    let level = if state.project.render.quality <= Quality::Low { 0.0 } else { house_level(state.project.render.ambient) };
    for (light, mut spot, mut tf, mut vis) in &mut house {
        let p = spots[light.0];
        tf.set_if_neq(Transform::from_translation(p).looking_at(p - Vec3::Y, Vec3::NEG_Z));
        let intensity = house_lumens(level, height);
        let color = house_color(&state.project.render);
        if spot.intensity != intensity || spot.range != height * 3.0 || spot.color != color {
            spot.intensity = intensity;
            spot.range = height * 3.0;
            spot.color = color;
        }
        vis.set_if_neq(if level > 0.0 { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Camera that draws the 3D image and the egui UI to the window.
#[derive(Component)]
pub struct UiCamera;

pub fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    mut images: ResMut<Assets<Image>>,
    state: Res<ShowState>,
    surface: Res<SurfaceGobos>,
) {
    let image = images.add(Image::new_target_texture(64, 64, TextureFormat::Rgba8UnormSrgb, None));
    let logo = images.add(logo_image());
    commands.insert_resource(View3d { image: image.clone(), logo });
    commands.spawn((
        Camera2d,
        Camera { order: 1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        Msaa::Off,
        // Only egui draws here; keep gizmos and meshes to the 3D camera.
        bevy::camera::visibility::RenderLayers::layer(1),
        UiCamera,
        PrimaryEguiContext,
    ));
    commands.insert_resource(DirectionalLightShadowMap { size: shadow_map_size(state.project.render.quality) });

    // BK is Z-up; rotating -90° around X maps BK +Z to Bevy +Y and BK +Y to Bevy -Z.
    commands.spawn((BkSpace, Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)), Visibility::default()));

    let orbit = OrbitCamera::audience();
    let mut cam = commands.spawn((
        Camera3d::default(),
        Hdr,
        Tonemapping::AgX,
        Exposure { ev100: state.project.render.exposure_ev },
        orbit.transform(),
        orbit,
        // Beams and gobo projectors read the depth of the surfaces they meet.
        DepthPrepass,
        RenderTarget::Image(image.into()),
        // Renders into an image, so Bevy wouldn't pick it for the shadow LOD by itself.
        bevy::camera::ShadowLodOrigin,
        // Shadow maps exist only on High; one tap is plenty with TAA.
        ShadowFilteringMethod::Hardware2x2,
    ));
    if bloom_on(&state.project.render) {
        cam.insert(Bloom::NATURAL);
    }
    if state.project.render.quality > Quality::Low {
        // Averages the beams' jittered ray marching over frames.
        cam.insert((TemporalAntiAliasing::default(), Msaa::Off));
    } else {
        // Cheap edge smoothing for weak GPUs (MSAA costs more there).
        cam.insert((Fxaa::default(), Msaa::Off));
    }

    commands.spawn((
        super::objects::Floor,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(materials.add(SurfaceMaterial {
            base: StandardMaterial { base_color: Color::srgb(0.18, 0.18, 0.19), perceptual_roughness: 0.85, ..default() },
            extension: surface.ext(),
        })),
    ));
    // Stage platform: 10 x 8 m, 1 m high, upstage of the audience.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(10.0, 1.0, 8.0))),
        MeshMaterial3d(materials.add(SurfaceMaterial {
            base: StandardMaterial { base_color: Color::srgb(0.12, 0.11, 0.10), perceptual_roughness: 0.7, ..default() },
            extension: surface.ext(),
        })),
        Transform::from_xyz(0.0, 0.5, -1.0),
        super::objects::Placeholder,
    ));

    let [r, g, b] = bk_olga::light::kelvin_to_rgb(3000.0);
    for i in 0..HOUSE_LIGHTS {
        commands.spawn((
            HouseLight(i),
            SpotLight {
                color: Color::linear_rgb(r, g, b),
                intensity: 0.0,
                range: 30.0,
                outer_angle: 70f32.to_radians(),
                inner_angle: 35f32.to_radians(),
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::default(),
            Visibility::Hidden,
        ));
    }

    commands.insert_resource(ClearColor(Color::srgb(0.004, 0.004, 0.006)));
}

/// Spot light shadow maps take their size from the directional shadow map
/// setting (2048 by default, far more than a beam needs).
fn shadow_map_size(quality: Quality) -> usize {
    match quality {
        Quality::High => 1024,
        _ => 512,
    }
}

/// Share of the physical pixels the 3D view renders at. Automatic: Low
/// three quarters of the logical resolution, Medium the logical one (half on
/// Retina), High every physical pixel up to about 4K.
pub fn resolution_scale(r: &RenderSettings, scale_factor: f32, physical_pixels: f32) -> f32 {
    let logical = 1.0 / scale_factor.max(1.0);
    let auto = match r.quality {
        Quality::Draft => logical * 0.5,
        Quality::Low => logical * 0.75,
        Quality::Medium => logical,
        Quality::High => (8.3e6 / physical_pixels.max(1.0)).sqrt().clamp(logical, 1.0),
    };
    r.resolution.unwrap_or(auto).clamp(0.25, 1.0)
}

/// Keeps the 3D image matched to the window (or screenshot target) size.
pub fn fit_view(
    state: Res<ShowState>,
    view: Res<View3d>,
    ui_camera: Query<&Camera, With<UiCamera>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Ok(camera) = ui_camera.single() else { return };
    let (Some(physical), Some(logical)) = (camera.physical_target_size(), camera.logical_target_size()) else { return };
    let scale = resolution_scale(&state.project.render, physical.x as f32 / logical.x.max(1.0), physical.as_vec2().element_product());
    let size = (physical.as_vec2() * scale).round().max(Vec2::splat(16.0)).as_uvec2();
    if images.get(&view.image).is_some_and(|i| i.size() != size)
        && let Some(mut image) = images.get_mut(&view.image)
    {
        image.resize(Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 });
    }
}


/// Applies render settings that don't need a fixture rebuild.
pub fn apply_render_settings(
    mut commands: Commands,
    state: Res<ShowState>,
    mut camera: Query<(Entity, &mut Exposure, Has<Bloom>, Has<TemporalAntiAliasing>), With<Camera3d>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
) {
    if !state.is_changed() {
        return;
    }
    let r = &state.project.render;
    if shadow_map.size != shadow_map_size(r.quality) {
        shadow_map.size = shadow_map_size(r.quality);
    }
    if let Ok((cam, mut exposure, has_bloom, has_taa)) = camera.single_mut() {
        exposure.ev100 = r.exposure_ev;
        match (bloom_on(r), has_bloom) {
            (true, false) => {
                commands.entity(cam).insert(Bloom::NATURAL);
            }
            (false, true) => {
                commands.entity(cam).remove::<Bloom>();
            }
            _ => {}
        }
        // Re-inserting TAA would reset its history and bring the beam noise back.
        match (r.quality > Quality::Low, has_taa) {
            (true, false) => {
                commands.entity(cam).remove::<Fxaa>().insert((TemporalAntiAliasing::default(), Msaa::Off));
            }
            (false, true) => {
                commands.entity(cam).remove::<TemporalAntiAliasing>().insert((Fxaa::default(), Msaa::Off));
            }
            _ => {}
        }
    }
    // The house light is real light from the ceiling (`place_house_lights`); a
    // little ambient stands in for what walls and floor bounce back. On Low the
    // ceiling lights would cost every pixel six more lights: plain ambient of
    // the same brightness instead (illuminance ≈ π × brightness).
    ambient.brightness = house_level(r.ambient) * if r.quality <= Quality::Low { 30.0 + HOUSE_LUX / std::f32::consts::PI } else { 30.0 };
    ambient.color = house_color(r);
}
