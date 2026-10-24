//! Generic fixture models and their per-frame update from DMX.
//!
//! Everything below `BkSpace` uses BlinderKitten's coordinates (Z up). A
//! fixture hangs from its origin, and its beam leaves the head along local -Z,
//! which is also the direction a Bevy `SpotLight` shines in.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bk_olga::placement::world_from_local;
use bk_olga::{Attribute, FixtureClass, FixtureState, FixtureType, Resolution, light};
use tapetum_project::{BeamKind, CellMode, ColorSlot, FixtureOverride, Gobo, GoboSlot, LedLayout, LedShape, Quality, wheel_slot};

use super::beam::{BeamMaterial, BeamParams, LensMaterial, LensParams, SURFACE_GOBOS, SurfaceGobos};
use super::gobo::{GoboImages, is_water};
use super::layout;
use super::motion::{Axis, Limits};
use super::state::{Dmx, ShowState};

/// Root of all BK-space content: rotates BK's Z-up frame into Bevy's Y-up frame.
#[derive(Component)]
pub struct BkSpace;

#[derive(Component)]
pub struct FixtureRoot {
    pub index: usize,
}

#[derive(Component)]
pub struct Yoke {
    pub index: usize,
    pub pan: Axis,
}

#[derive(Component)]
pub struct Head {
    pub index: usize,
    pub tilt: Axis,
}

#[derive(Component)]
pub struct Emitter {
    pub fixture: usize,
    /// Cells mixed into this beam; `None` is the fixture body.
    pub cells: Vec<Option<u32>>,
    /// Weight of each cell in the mix: its number of LEDs.
    pub weights: Vec<f32>,
    pub lumens: f32,
    /// Radius of the light-emitting area this beam leaves (LED field or cell).
    pub radius: f32,
    /// The fixture's main beam (the others are extra cells in "one light per cell").
    pub main: bool,
    /// Beam angle on its way to the zoom's target.
    pub zoom: Axis,
}

#[derive(Component)]
pub struct Lens {
    pub fixture: usize,
    pub cell: Option<u32>,
    pub material: Handle<LensMaterial>,
}

/// The beam's volume in the haze, a child of its emitter.
/// Takes over from its emitter's spot light beyond 180°: a spot can't shine
/// backwards. A child of the emitter.
#[derive(Component)]
pub struct WideLight;

/// Widest half angle Bevy's spot lights take (just under 90°).
const SPOT_MAX_HALF: f32 = 89.0 * std::f32::consts::PI / 180.0;
/// Beyond this half angle there's no beam to see in the haze, nor a gobo picture.
const BEAM_MAX_HALF: f32 = 60.0 * std::f32::consts::PI / 180.0;

#[derive(Component)]
pub struct BeamVolume {
    pub material: Handle<BeamMaterial>,
}

/// Resolved settings of a fixture type: automatic choice + project overrides.
#[derive(Clone, Debug)]
pub struct FixtureSpec {
    pub class: FixtureClass,
    pub beam: BeamKind,
    pub cells: Vec<u32>,
    pub cell_mode: CellMode,
    /// `None`: one lens instead of single LEDs.
    pub leds: Option<LedLayout>,
    /// Full beam angle in degrees without a zoom channel.
    pub beam_angle: f32,
    /// Beam angle at zoom 0 and full, if the type has a zoom channel.
    pub zoom_range: Option<[f32; 2]>,
    pub pan_range: [f32; 2],
    pub tilt_range: [f32; 2],
    pub pan_limits: Limits,
    pub tilt_limits: Limits,
    pub zoom_limits: Limits,
    pub lumens: f32,
    pub tuning: light::Tuning,
    pub color_wheel: Vec<ColorSlot>,
    pub gobo_wheel: Vec<GoboSlot>,
    pub fixed_gobo: Option<Gobo>,
    pub body_scale: f32,
}

pub const CLASSES: [FixtureClass; 5] =
    [FixtureClass::MovingHead, FixtureClass::Spot, FixtureClass::Wash, FixtureClass::Bar, FixtureClass::Conventional];

pub fn class_from_label(label: &str) -> Option<FixtureClass> {
    CLASSES.into_iter().find(|c| c.label() == label)
}

pub const PAN_DEFAULT: [f32; 2] = [-270.0, 270.0];
pub const TILT_DEFAULT: [f32; 2] = [-135.0, 135.0];
/// Typical LED moving head: 540° pan in a bit over two seconds, tilt somewhat slower.
pub const PAN_SPEED_DEFAULT: f32 = 250.0;
pub const TILT_SPEED_DEFAULT: f32 = 180.0;
pub const PT_ACCELERATION_DEFAULT: f32 = 900.0;
/// A typical zoom: about a second from one end of the range to the other.
pub const ZOOM_SPEED_DEFAULT: f32 = 40.0;

/// Zoom motors have no own acceleration setting: top speed within a quarter second.
pub fn zoom_limits(max_speed: f32) -> Limits {
    Limits { max_speed, acceleration: max_speed * 4.0 }
}

/// Spot if the type has optics only a profile has; otherwise a wash.
pub fn auto_beam(ty: &FixtureType) -> BeamKind {
    use Attribute::*;
    if [Gobo, Iris, Focus, Prism].iter().any(|a| ty.has(a)) { BeamKind::Spot } else { BeamKind::Wash }
}

/// LED layout used when none is configured.
pub fn auto_leds(class: FixtureClass, cells: &[u32]) -> Option<LedLayout> {
    let n = cells.len() as u32;
    if n == 0 {
        return None;
    }
    let (shape, pitch) = match class {
        FixtureClass::Bar => (LedShape::Rect { rows: 1, cols: n }, 0.09),
        _ => {
            let rings = match n {
                7 => vec![1, 6],
                19 => vec![1, 6, 12],
                37 => vec![1, 6, 12, 18],
                _ => vec![n],
            };
            let pitch = if rings.len() == 1 { 0.05 } else { 0.065 };
            (LedShape::Circular { rings }, pitch)
        }
    };
    Some(LedLayout::new(shape, pitch, cells))
}

/// BK's physical range of the first (coarse) channel of `attr`.
pub fn bk_range(ty: &FixtureType, attr: &Attribute) -> Option<[f32; 2]> {
    ty.channels.iter().find(|c| &c.attribute == attr && c.resolution != Resolution::FineOnly).and_then(|c| c.physical_range)
}

fn class_defaults(class: FixtureClass, beam: BeamKind) -> (f32, f32) {
    let (angle, lumens) = match class {
        FixtureClass::MovingHead => (15.0, 4000.0),
        FixtureClass::Spot => (25.0, 3000.0),
        FixtureClass::Wash => (40.0, 2500.0),
        FixtureClass::Bar => (30.0, 2000.0),
        FixtureClass::Conventional => (30.0, 8000.0),
        FixtureClass::Unknown => (30.0, 0.0),
    };
    let angle = match (class, beam) {
        (FixtureClass::MovingHead, BeamKind::Wash) => 30.0,
        (FixtureClass::Wash, BeamKind::Spot) => 25.0,
        _ => angle,
    };
    (angle, lumens)
}

pub fn resolve(ty: &FixtureType, o: &FixtureOverride) -> FixtureSpec {
    let class = o.class.as_deref().and_then(class_from_label).unwrap_or_else(|| ty.classify());
    let cells = ty.cells();
    let beam = o.beam.unwrap_or_else(|| auto_beam(ty));
    let (angle, lumens) = class_defaults(class, beam);
    let beam_angle = o.beam_angle.unwrap_or(angle);
    let leds = match &o.leds {
        // An empty layout means "one lens".
        Some(l) if l.shape.count() == 0 => None,
        Some(l) => {
            let mut l = l.clone();
            l.fit(&cells);
            Some(l)
        }
        None => auto_leds(class, &cells),
    };
    FixtureSpec {
        class,
        beam,
        cell_mode: o.cell_mode.unwrap_or_default(),
        leds,
        beam_angle,
        zoom_range: ty
            .has(&Attribute::Zoom)
            .then(|| o.zoom_range.or_else(|| bk_range(ty, &Attribute::Zoom)).unwrap_or([beam_angle * 2.0, beam_angle * 0.5])),
        pan_range: o.pan_range.or_else(|| bk_range(ty, &Attribute::Pan)).unwrap_or(PAN_DEFAULT),
        tilt_range: o.tilt_range.or_else(|| bk_range(ty, &Attribute::Tilt)).unwrap_or(TILT_DEFAULT),
        pan_limits: Limits {
            max_speed: o.pan_speed.unwrap_or(PAN_SPEED_DEFAULT),
            acceleration: o.pt_acceleration.unwrap_or(PT_ACCELERATION_DEFAULT),
        },
        tilt_limits: Limits {
            max_speed: o.tilt_speed.unwrap_or(TILT_SPEED_DEFAULT),
            acceleration: o.pt_acceleration.unwrap_or(PT_ACCELERATION_DEFAULT),
        },
        zoom_limits: zoom_limits(o.zoom_speed.unwrap_or(ZOOM_SPEED_DEFAULT)),
        lumens: o.lumens.unwrap_or(lumens),
        tuning: light::Tuning {
            white: o.white_kelvin.map(light::kelvin_to_rgb),
            strobe_hz: o.strobe_hz.unwrap_or(light::Tuning::default().strobe_hz),
        },
        color_wheel: o.color_wheel.clone().unwrap_or_default(),
        gobo_wheel: o.gobo_wheel.clone().unwrap_or_default(),
        fixed_gobo: o.fixed_gobo.clone().filter(|g| *g != Gobo::Open),
        body_scale: o.body_scale.unwrap_or(1.0),
        cells,
    }
}

pub fn spec_for(state: &ShowState, index: usize) -> Option<FixtureSpec> {
    let f = &state.show.fixtures[index];
    let ty = state.show.fixture_type(f)?;
    Some(resolve(ty, &state.project.fixture_override(f.id, &ty.short_name)))
}

#[derive(Resource)]
pub struct FixtureAssets {
    body: Handle<StandardMaterial>,
    lens_off: Handle<StandardMaterial>,
    cuboid: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    disc: Handle<Mesh>,
    cone: Handle<Mesh>,
    /// Stand-in gobo texture for beams without a gobo.
    pub white: Handle<Image>,
}

pub fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(FixtureAssets {
        body: materials.add(StandardMaterial { base_color: Color::srgb(0.11, 0.11, 0.12), perceptual_roughness: 0.4, metallic: 0.5, ..default() }),
        lens_off: materials.add(StandardMaterial { base_color: Color::srgb(0.02, 0.02, 0.025), perceptual_roughness: 0.1, ..default() }),
        cuboid: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        // Unit cylinder/cone along Y; rotated into Z by the spawn code.
        cylinder: meshes.add(Cylinder::new(1.0, 1.0)),
        disc: meshes.add(Circle::new(1.0)),
        // Enough segments that the bounding mesh hugs the analytic cone of the shader.
        cone: meshes.add(Cone { radius: 1.0, height: 1.0 }.mesh().resolution(48)),
        white: images.add(Image::new_fill(
            bevy::render::render_resource::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            &[255, 255, 255, 255],
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        )),
    });
    commands.init_resource::<GoboImages>();
}

/// Y-up primitive -> Z-up, so a unit cylinder/cone spans local z -0.5..0.5.
fn y_to_z() -> Quat {
    Quat::from_rotation_x(FRAC_PI_2)
}

/// Orientation of a fixture root for a BK rotation (degrees).
pub fn placement_rotation(rot: [f32; 3]) -> Quat {
    mat3_to_quat(world_from_local(rot))
}

fn mat3_to_quat(m: [[f32; 3]; 3]) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::new(m[0][0], m[1][0], m[2][0]),
        Vec3::new(m[0][1], m[1][1], m[2][1]),
        Vec3::new(m[0][2], m[1][2], m[2][2]),
    ))
}

pub const LIGHT_RANGE: f32 = 40.0;
const BEAM_LENGTH: f32 = 25.0;

/// Despawns and recreates all fixtures whenever the show or a fixture setting changed.
pub fn rebuild_fixtures(
    mut commands: Commands,
    mut state: ResMut<ShowState>,
    assets: Res<FixtureAssets>,
    mut beam_materials: ResMut<Assets<BeamMaterial>>,
    mut lens_materials: ResMut<Assets<LensMaterial>>,
    mut gobos: ResMut<GoboImages>,
    mut images: ResMut<Assets<Image>>,
    space: Query<Entity, With<BkSpace>>,
    existing: Query<Entity, With<FixtureRoot>>,
    mut built: Local<Option<u64>>,
) {
    if *built == Some(state.generation) {
        return;
    }
    *built = Some(state.generation);
    for e in &existing {
        commands.entity(e).despawn();
    }
    let Ok(space) = space.single() else { return };

    let placements = layout::placements(&state.show, &state.project);
    let quality = state.project.render.quality;
    let base = state.project_dir();
    let mut specs = Vec::with_capacity(state.show.fixtures.len());
    let mut problems = Vec::new();
    for (index, placement) in placements.iter().enumerate() {
        let Some(spec) = spec_for(&state, index) else {
            specs.push(None);
            continue;
        };
        for gobo in spec.gobo_wheel.iter().map(|s| &s.gobo).chain(&spec.fixed_gobo) {
            if let Err(e) = gobos.ensure(gobo, &base, &mut images) {
                problems.push(format!("Gobo: {e}"));
            }
        }
        let p = placement.position;
        let root = commands
            .spawn((
                FixtureRoot { index },
                Transform::from_translation(Vec3::new(p[0], p[1], p[2])).with_rotation(mat3_to_quat(world_from_local(placement.rotation))),
                Visibility::default(),
                ChildOf(space),
                Name::new(state.show.fixtures[index].name.clone()),
            ))
            .id();
        let mut mats = Materials { beam: &mut beam_materials, lens: &mut lens_materials };
        spawn_model(&mut commands, &assets, &mut mats, root, index, &spec, quality);
        specs.push(Some(spec));
    }
    state.specs = specs;
    problems.dedup();
    if !problems.is_empty() {
        state.status = Some(problems.join("\n"));
    }
}

/// Fixture parts don't cast shadows: the emitters sit inside the housing and
/// would otherwise shadow their own (and neighboring cells') beams.
fn part(commands: &mut Commands, parent: Entity, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>, tf: Transform) {
    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), tf, NotShadowCaster, ChildOf(parent)));
}

/// One beam of a fixture: position on the front, cells it shows, share of the
/// light and the radius of the area it leaves (0 = the whole lens).
struct EmitterPlan {
    pos: Vec2,
    cells: Vec<Option<u32>>,
    /// LED count per cell, for mixing.
    weights: Vec<f32>,
    share: f32,
    radius: f32,
}

struct Materials<'a> {
    beam: &'a mut Assets<BeamMaterial>,
    lens: &'a mut Assets<LensMaterial>,
}

fn cell_of(id: u32) -> Option<u32> {
    (id != 0).then_some(id)
}

fn plan_emitters(spec: &FixtureSpec, positions: &[Vec2], quality: Quality) -> Vec<EmitterPlan> {
    let Some(leds) = &spec.leds else {
        return vec![EmitterPlan { pos: Vec2::ZERO, cells: vec![None], weights: vec![1.0], share: 1.0, radius: 0.0 }];
    };
    let led_radius = leds.pitch * LED_SIZE;
    let radius = |ps: &[Vec2], c: Vec2| ps.iter().map(|p| p.distance(c)).fold(0.0, f32::max) + led_radius;
    // Distinct cells in LED order, with the LEDs that show them.
    let mut groups: Vec<(Option<u32>, Vec<Vec2>)> = Vec::new();
    for (&id, &p) in leds.led_cells.iter().zip(positions) {
        match groups.iter_mut().find(|(c, _)| *c == cell_of(id)) {
            Some((_, ps)) => ps.push(p),
            None => groups.push((cell_of(id), vec![p])),
        }
    }
    let total = positions.len().max(1) as f32;
    let centroid = |ps: &[Vec2]| ps.iter().copied().sum::<Vec2>() / ps.len().max(1) as f32;
    // Low quality always mixes, since overlapping additive cones would wash out.
    if spec.cell_mode == CellMode::Mixed || quality <= Quality::Low || groups.len() <= 1 {
        let c = centroid(positions);
        return vec![EmitterPlan {
            pos: c,
            weights: groups.iter().map(|(_, ps)| ps.len() as f32).collect(),
            cells: groups.into_iter().map(|(c, _)| c).collect(),
            share: 1.0,
            radius: radius(positions, c),
        }];
    }
    groups
        .into_iter()
        .map(|(cell, ps)| {
            let c = centroid(&ps);
            EmitterPlan { pos: c, share: ps.len() as f32 / total, cells: vec![cell], weights: vec![1.0], radius: radius(&ps, c) }
        })
        .collect()
}

/// LED disc radius relative to the LED pitch.
const LED_SIZE: f32 = 0.42;

fn spawn_model(
    commands: &mut Commands,
    a: &FixtureAssets,
    mats: &mut Materials,
    root: Entity,
    index: usize,
    spec: &FixtureSpec,
    quality: Quality,
) {
    let s = spec.body_scale;
    let positions: Vec<Vec2> = spec.leds.as_ref().map_or_else(Vec::new, |l| l.shape.positions(l.pitch).into_iter().map(Vec2::from_array).collect());
    let pitch = spec.leds.as_ref().map_or(0.0, |l| l.pitch);
    let led_radius = pitch * LED_SIZE;
    // Half extent of the LED field; the housing grows to fit it.
    let extent = positions.iter().fold(Vec2::ZERO, |m, p| m.max(p.abs())) + Vec2::splat(led_radius + 0.012);
    let field_radius = positions.iter().map(|p| p.length()).fold(0.0, f32::max) + led_radius + 0.012;

    // Head entity carries the lens and the emitters; `lens_z` is its front face.
    let (head, lens_z, lens_radius) = match spec.class {
        FixtureClass::MovingHead => {
            let r = (0.125 * s).max(field_radius);
            part(commands, root, &a.cuboid, &a.body, Transform::from_xyz(0.0, 0.0, -0.05 * s).with_scale(Vec3::new(0.34, 0.24, 0.10) * s.max(r / 0.125)));
            let yoke = commands.spawn((Yoke { index, pan: Axis::default() }, Transform::from_xyz(0.0, 0.0, -0.10 * s), Visibility::default(), ChildOf(root))).id();
            let arm_x = r + 0.03;
            for x in [-arm_x, arm_x] {
                part(commands, yoke, &a.cuboid, &a.body, Transform::from_xyz(x, 0.0, -0.12 * s).with_scale(Vec3::new(0.03, 0.09, 0.26) * s));
            }
            let head = commands.spawn((Head { index, tilt: Axis::default() }, Transform::from_xyz(0.0, 0.0, -0.22 * s), Visibility::default(), ChildOf(yoke))).id();
            let h = 0.26 * s;
            part(commands, head, &a.cylinder, &a.body, Transform::from_rotation(y_to_z()).with_scale(Vec3::new(r, h, r)));
            (head, -h / 2.0 - 0.001, r * 0.92)
        }
        FixtureClass::Bar => {
            let len = (extent.x * 2.0).max(0.3 * s);
            let height = (extent.y * 2.0).max(0.13 * s);
            let head = commands.spawn((Head { index, tilt: Axis::default() }, Transform::from_xyz(0.0, 0.0, -0.06 * s), Visibility::default(), ChildOf(root))).id();
            part(commands, head, &a.cuboid, &a.body, Transform::from_scale(Vec3::new(len, height, 0.10 * s)));
            (head, -0.05 * s - 0.001, 0.0)
        }
        _ => {
            let (r, h) = if spec.class == FixtureClass::Conventional { (0.11, 0.34) } else { (0.13, 0.28) };
            let (r, h) = ((r * s).max(field_radius), h * s);
            // Hanging bracket.
            part(commands, root, &a.cuboid, &a.body, Transform::from_xyz(0.0, 0.0, -0.04 * s).with_scale(Vec3::new(0.04, 0.04, 0.08) * s));
            let head = commands.spawn((Head { index, tilt: Axis::default() }, Transform::from_xyz(0.0, 0.0, -0.08 * s - h / 2.0), Visibility::default(), ChildOf(root))).id();
            part(commands, head, &a.cylinder, &a.body, Transform::from_rotation(y_to_z()).with_scale(Vec3::new(r, h, r)));
            (head, -h / 2.0 - 0.001, r * 0.9)
        }
    };

    // The lens faces -Z: flip the +Z-facing disc.
    let facing_down = Quat::from_rotation_x(PI);
    if lens_radius > 0.0 {
        part(commands, head, &a.disc, &a.lens_off, Transform::from_xyz(0.0, 0.0, lens_z).with_rotation(facing_down).with_scale(Vec3::splat(lens_radius)));
    }
    let glow = |mats: &mut Materials| mats.lens.add(LensMaterial { params: LensParams::default() });
    match &spec.leds {
        None => {
            let material = glow(mats);
            let radius = if lens_radius > 0.0 { lens_radius * 0.95 } else { 0.04 * s };
            commands.spawn((
                Mesh3d(a.disc.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(0.0, 0.0, lens_z - 0.001).with_rotation(facing_down).with_scale(Vec3::splat(radius)),
                Lens { fixture: index, cell: None, material },
                NotShadowCaster,
                ChildOf(head),
            ));
        }
        Some(leds) => {
            // LEDs of one cell always show the same: one material for them all.
            let mut per_cell: std::collections::HashMap<u32, Handle<LensMaterial>> = std::collections::HashMap::new();
            for (&id, pos) in leds.led_cells.iter().zip(&positions) {
                let material = per_cell.entry(id).or_insert_with(|| glow(mats)).clone();
                commands.spawn((
                    Mesh3d(a.disc.clone()),
                    MeshMaterial3d(material.clone()),
                    // Seen from the front the layout's x points left in head space (the disc is flipped).
                    Transform::from_xyz(-pos.x, pos.y, lens_z - 0.002).with_rotation(facing_down).with_scale(Vec3::splat(led_radius)),
                    Lens { fixture: index, cell: cell_of(id), material },
                    NotShadowCaster,
                    ChildOf(head),
                ));
            }
        }
    }

    let plans = plan_emitters(spec, &positions, quality);
    let inner = inner_ratio(spec.beam);
    for (i, plan) in plans.into_iter().enumerate() {
        let half = (spec.beam_angle / 2.0).to_radians().min(SPOT_MAX_HALF);
        let radius = if plan.radius > 0.0 { plan.radius } else { lens_radius.max(0.03) * 0.9 };
        let material = mats.beam.add(BeamMaterial { params: BeamParams::default(), gobo: a.white.clone() });
        let mut emitter = commands.spawn((
            Transform::from_xyz(-plan.pos.x, plan.pos.y, lens_z - 0.01),
            Visibility::default(),
            Emitter { fixture: index, cells: plan.cells, weights: plan.weights, lumens: spec.lumens * plan.share, radius, main: i == 0, zoom: Axis::default() },
            ChildOf(head),
        ));
        // Draft lights nothing: no light to sort into clusters, nothing to shade.
        if quality > Quality::Draft {
            emitter.insert(SpotLight {
                color: Color::WHITE,
                intensity: 0.0,
                range: LIGHT_RANGE,
                radius: 0.02,
                // Switched on per frame only while the beam meets a stage object
                // (`update_fixtures`): every shadow map costs a render pass.
                shadow_maps_enabled: false,
                outer_angle: half,
                inner_angle: half * inner,
                ..default()
            });
        }
        emitter
            .with_child((Mesh3d(a.cone.clone()), MeshMaterial3d(material.clone()), Transform::default(), Visibility::Hidden, BeamVolume { material }, NotShadowCaster))
            .with_child((
                PointLight { intensity: 0.0, range: LIGHT_RANGE, radius: 0.02, shadow_maps_enabled: false, ..default() },
                Transform::default(),
                Visibility::Hidden,
                WideLight,
            ));
    }
}

/// Haze density the cones of Draft show with while haze is off.
const DRAFT_HAZE: f32 = 0.12;

/// Inner (full intensity) cone relative to the outer edge.
fn inner_ratio(beam: BeamKind) -> f32 {
    match beam {
        BeamKind::Spot => 0.85,
        BeamKind::Wash => 0.25,
    }
}

/// Decodes every fixture from the latest DMX frames.
pub fn decode_states(state: &ShowState, dmx: &Dmx) -> Vec<Option<FixtureState>> {
    let universes = dmx.receiver.store.snapshot();
    state
        .show
        .fixtures
        .iter()
        .map(|f| {
            let ty = state.show.fixture_type(f)?;
            let (patch, pa) = crate::routing::primary_patch(&state.show, &state.project.artnet, &f.patches)?;
            let u = universes.get(&pa)?;
            Some(ty.decode(patch.address, &u.data))
        })
        .collect()
}

/// Full cone solid angle for a half angle in radians.
fn solid_angle(half: f32) -> f32 {
    TAU * (1.0 - half.cos())
}

fn lerp(r: [f32; 2], t: f32) -> f32 {
    r[0] + (r[1] - r[0]) * t
}

/// Beam of one emitter in this frame.
#[derive(Clone, Copy)]
struct Beam {
    out: light::LightOutput,
    /// Full angle in degrees.
    angle: f32,
}

/// Per-frame values shared by the fixture update systems.
struct Frame<'a> {
    state: &'a ShowState,
    states: Vec<Option<FixtureState>>,
    empty: FixtureState,
    t: f32,
}

impl<'a> Frame<'a> {
    fn new(state: &'a ShowState, dmx: &Dmx, t: f32) -> Self {
        Frame { states: decode_states(state, dmx), state, empty: FixtureState::default(), t }
    }

    fn st(&self, i: usize) -> &FixtureState {
        self.states.get(i).and_then(|s| s.as_ref()).unwrap_or(&self.empty)
    }

    fn spec(&self, i: usize) -> Option<&'a FixtureSpec> {
        self.state.specs.get(i).and_then(|s| s.as_ref())
    }

    fn first_cell(&self, i: usize) -> Option<u32> {
        self.spec(i).and_then(|s| s.cells.first().copied())
    }

    fn value(&self, i: usize, cell: Option<u32>, attr: &Attribute) -> Option<f32> {
        self.st(i).get(cell, attr).map(|v| v.value)
    }

    fn output(&self, fixture: usize, cell: Option<u32>) -> light::LightOutput {
        // No DMX for a fixture means it is dark, not "all defaults".
        let (Some(Some(st)), Some(spec)) = (self.states.get(fixture), self.spec(fixture)) else { return light::LightOutput::OFF };
        let mut out = light::compute_tuned(st, cell, spec.class, &spec.tuning);
        if spec.class == FixtureClass::Conventional && spec.tuning.white.is_none() {
            out.color = light::tungsten_at(out.intensity);
        }
        if let Some(v) = self.value(fixture, cell, &Attribute::ColorWheel)
            && let Some(slot) = wheel_slot(&spec.color_wheel, |s| s.start, v)
        {
            // A dichroic filter passes its own color out of the white light.
            let c = [out.color[0] * slot.color[0], out.color[1] * slot.color[1], out.color[2] * slot.color[2]];
            let peak = c.iter().copied().fold(1e-4, f32::max);
            out.color = c.map(|x| x / peak);
            out.intensity *= (slot.color.iter().sum::<f32>() / 3.0).clamp(0.05, 1.0);
        }
        if let Some(hz) = out.strobe_hz
            && (self.t * hz).fract() > 0.3
        {
            out.intensity = 0.0;
        }
        out
    }

    /// The gobo in the beam: from the wheel by the gobo channel, else a fixed one.
    fn gobo(&self, fixture: usize) -> Option<&'a Gobo> {
        let spec = self.spec(fixture)?;
        match self.value(fixture, self.first_cell(fixture), &Attribute::Gobo) {
            Some(v) if !spec.gobo_wheel.is_empty() => wheel_slot(&spec.gobo_wheel, |s| s.start, v).map(|s| &s.gobo).filter(|g| **g != Gobo::Open),
            _ => spec.fixed_gobo.as_ref(),
        }
    }

    /// Full beam angle in degrees from zoom and iris, the zoom as `zoom`
    /// (target angle in, travelled angle out) makes it.
    fn angle(&self, fixture: usize, cell: Option<u32>, zoom: impl FnOnce(f32) -> f32) -> f32 {
        let spec = self.spec(fixture);
        let mut angle = match (spec.and_then(|s| s.zoom_range), self.value(fixture, cell, &Attribute::Zoom)) {
            (Some(range), Some(z)) => zoom(lerp(range, z)),
            _ => spec.map_or(30.0, |s| s.beam_angle),
        };
        // Closing the iris narrows the beam down to a fifth.
        if let Some(iris) = self.value(fixture, cell, &Attribute::Iris) {
            angle *= 1.0 - 0.8 * iris;
        }
        angle.clamp(0.5, 360.0)
    }

    fn beam(&self, em: &mut Emitter, dt: f32) -> Beam {
        let outs: Vec<_> = em.cells.iter().zip(&em.weights).map(|(&c, &w)| (self.output(em.fixture, c), w)).collect();
        let out = if outs.len() == 1 { outs[0].0 } else { light::mix_weighted(&outs) };
        let limits = self.spec(em.fixture).map_or(zoom_limits(0.0), |s| s.zoom_limits);
        let zoom = &mut em.zoom;
        Beam { out, angle: self.angle(em.fixture, em.cells[0], |target| zoom.step(target, limits, dt)) }
    }
}

pub fn update_fixtures(
    state: Res<ShowState>,
    dmx: Res<Dmx>,
    time: Res<Time>,
    gobos: Res<GoboImages>,
    mut yokes: Query<(&mut Yoke, &mut Transform), (Without<Head>, Without<BeamVolume>)>,
    mut heads: Query<(&mut Head, &mut Transform), (Without<Yoke>, Without<BeamVolume>)>,
    mut emitters: Query<(&mut Emitter, Option<&mut SpotLight>, &GlobalTransform, Option<&Children>)>,
    mut beams: Query<(&BeamVolume, &mut Transform, &mut Visibility), (Without<Yoke>, Without<Head>)>,
    mut wide: Query<(&mut PointLight, &mut Visibility), (With<WideLight>, Without<BeamVolume>)>,
    mut surface_gobos: ResMut<SurfaceGobos>,
    lenses: Query<(&Lens, &GlobalTransform)>,
    occluders: Res<super::objects::Occluders>,
    mut lens_materials: ResMut<Assets<LensMaterial>>,
    mut beam_materials: ResMut<Assets<BeamMaterial>>,
    (assets, mut activity): (Res<FixtureAssets>, ResMut<super::pacing::Activity>),
) {
    let render = &state.project.render;
    // Most steps a beam pixel may take (fewer on short stretches); negative =
    // no jitter, for Low, which has no TAA to smooth it out.
    let steps = match render.quality {
        // Draft: hardly more than a cone, its gobo still readable.
        Quality::Draft => -2.0,
        Quality::Low => -10.0,
        Quality::Medium => 24.0,
        Quality::High => 48.0,
    };
    // Orthographic has no TAA to smooth jittered steps (see `apply_render_settings`).
    let steps = if state.project.view.orthographic && steps > 0.0 { -steps } else { steps };
    let frame = Frame::new(&state, &dmx, time.elapsed_secs());
    let mut active_gobos = Vec::new();

    let dt = time.delta_secs();
    for (mut yoke, mut tf) in &mut yokes {
        let Some(spec) = frame.spec(yoke.index) else { continue };
        let target = frame.value(yoke.index, frame.first_cell(yoke.index), &Attribute::Pan).map_or(0.0, |v| lerp(spec.pan_range, v));
        let pan = yoke.pan.step(target, spec.pan_limits, dt);
        let rotation = Quat::from_rotation_z((-pan).to_radians());
        if tf.rotation != rotation {
            tf.rotation = rotation;
        }
    }
    for (mut head, mut tf) in &mut heads {
        let Some(spec) = frame.spec(head.index).filter(|s| s.class == FixtureClass::MovingHead) else { continue };
        let target = frame.value(head.index, frame.first_cell(head.index), &Attribute::Tilt).map_or(0.0, |v| lerp(spec.tilt_range, v));
        let tilt = head.tilt.step(target, spec.tilt_limits, dt);
        let rotation = Quat::from_rotation_x(tilt.to_radians());
        if tf.rotation != rotation {
            tf.rotation = rotation;
        }
    }

    // Where each fixture's zoom is now, for the lens glare below.
    let mut zoom_now = std::collections::HashMap::new();
    let draft = render.quality == Quality::Draft;
    for (mut em, mut spot, gt, children) in &mut emitters {
        let Beam { out, angle } = frame.beam(&mut em, dt);
        if em.main
            && let Some(z) = em.zoom.angle
        {
            zoom_now.insert(em.fixture, z);
        }
        let half = (angle / 2.0).to_radians();
        let beam = frame.spec(em.fixture).map_or(BeamKind::Wash, |s| s.beam);
        // Frost softens the edge of a spot.
        let frost = frame.value(em.fixture, em.cells[0], &Attribute::Frost).unwrap_or(0.0);
        // Bevy spreads a spot light's lumens like a point light's; concentrate them
        // into the cone. A gobo's pattern is cut out of the light on the surfaces
        // (`surface.wesl`).
        // Shadows (and beams ending at objects) only where a stage object is in the
        // cone: the main beam on Medium, every beam on High, none on Low.
        let may_shadow = match render.quality {
            Quality::Draft | Quality::Low => false,
            Quality::Medium => em.main,
            Quality::High => true,
        };
        let shadows = may_shadow && out.intensity > 0.0 && occluders.0.iter().any(|&(c, r)| cone_hits(gt.translation(), gt.forward().as_vec3(), half, LIGHT_RANGE, c, r));
        // Beyond 180° the point light child shines instead (same light per solid angle).
        let all_round = half > FRAC_PI_2;
        let lumens = em.lumens * out.intensity * (2.0 * TAU / solid_angle(half));
        for child in children.into_iter().flatten() {
            if let Ok((mut point, mut vis)) = wide.get_mut(*child) {
                vis.set_if_neq(if all_round && lumens > 0.0 && !draft { Visibility::Inherited } else { Visibility::Hidden });
                let color = Color::linear_rgb(out.color[0], out.color[1], out.color[2]);
                // Shadows of all six cube faces only on High.
                let point_shadows = all_round && shadows && render.quality == Quality::High;
                if all_round && (point.intensity != lumens || point.color != color || point.shadow_maps_enabled != point_shadows) {
                    point.intensity = lumens;
                    point.color = color;
                    point.shadow_maps_enabled = point_shadows;
                }
            }
        }
        let inner_angle = half.min(SPOT_MAX_HALF) * (inner_ratio(beam) * (1.0 - frost) + inner_ratio(BeamKind::Wash) * frost);
        let mut casts = false;
        if let Some(spot) = spot.as_deref_mut() {
            let new = SpotLight {
                shadow_maps_enabled: shadows && !all_round && half < BEAM_MAX_HALF,
                outer_angle: half.min(SPOT_MAX_HALF),
                inner_angle,
                color: Color::linear_rgb(out.color[0], out.color[1], out.color[2]),
                intensity: if all_round { 0.0 } else { lumens },
                ..*spot
            };
            // A changed light makes Bevy rebuild its shadow view; skip that for unchanged ones.
            let same = spot.outer_angle == new.outer_angle
                && spot.inner_angle == new.inner_angle
                && spot.color == new.color
                && spot.intensity == new.intensity
                && spot.shadow_maps_enabled == new.shadow_maps_enabled;
            if !same {
                *spot = new;
            }
            casts = spot.shadow_maps_enabled;
        }

        // The main beam's gobo, for the surfaces it lights (none in Draft).
        if em.main
            && !draft
            && out.intensity > 0.0
            && half < BEAM_MAX_HALF
            && let Some(g) = frame.gobo(em.fixture)
            && let Some(layer) = gobos.layer(g)
            && active_gobos.len() < SURFACE_GOBOS
        {
            active_gobos.push([
                gt.translation().extend(layer as f32),
                gt.forward().as_vec3().extend(LIGHT_RANGE),
                gt.up().as_vec3().extend(half.tan().max(1e-3)),
                // Blocked parts keep a little light (lens flare and spill).
                Vec4::new(half.cos(), 0.06, if is_water(g) { 1.0 } else { 0.0 }, 0.0),
            ]);
        }

        for child in children.into_iter().flatten() {
            let Ok((volume, mut tf, mut vis)) = beams.get_mut(*child) else { continue };
            // Draft has nothing but the beams: they show without haze too.
            let shown = (draft || render.haze.enabled && render.haze.density > 0.0) && out.intensity > 0.0 && half < BEAM_MAX_HALF;
            vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
            if !shown {
                continue;
            }
            // Virtual apex behind the lens, so the beam leaves with the emitter's diameter.
            let tan_h = half.tan().max(1e-3);
            let behind = em.radius / tan_h;
            let total = behind + BEAM_LENGTH;
            // The mesh only bounds the beam; the shader cuts it to lens plane and length.
            let r = total * tan_h * 1.06;
            tf.set_if_neq(Transform::from_xyz(0.0, 0.0, behind - total / 2.0).with_rotation(Quat::from_rotation_x(FRAC_PI_2)).with_scale(Vec3::new(r, total, r)));

            let candela = em.lumens * out.intensity / solid_angle(half);
            let gobo_slot = frame.gobo(em.fixture);
            let gobo = gobo_slot.and_then(|g| gobos.get(g)).map(|g| g.0.clone());
            let gobo_mode = match gobo_slot {
                Some(g) if is_water(g) => 2.0,
                Some(_) if gobo.is_some() => 1.0,
                _ => 0.0,
            };
            let axis = gt.forward().as_vec3();
            let c = render.haze.color;
            let params = BeamParams {
                apex: (gt.transform_point(Vec3::Z * behind)).extend(behind),
                axis: axis.extend(total),
                up: gt.up().as_vec3().extend(tan_h),
                color: Vec4::new(out.color[0] * candela, out.color[1] * candela, out.color[2] * candela, inner_angle.cos()),
                haze: Vec4::new(if draft && !render.haze.enabled { DRAFT_HAZE } else { render.haze.density } * HAZE_SCATTERING, render.haze.anisotropy, half.cos(), steps),
                tint: Vec4::new(c[0], c[1], c[2], gobo_mode),
                // Beams whose light casts shadows end at the objects in their way.
                lens: gt.translation().extend(if casts { 1.0 } else { 0.0 }),
            };
            let image = gobo.unwrap_or_else(|| assets.white.clone());
            // Moving water is drawn by the shader: it changes without new values.
            if gobo_mode == 2.0 {
                activity.changed = true;
            }
            if beam_materials.get(&volume.material).is_some_and(|m| m.params != params || m.gobo != image)
                && let Some(mut m) = beam_materials.get_mut(&volume.material)
            {
                m.params = params;
                m.gobo = image;
                activity.changed = true;
            }
        }
    }

    if surface_gobos.entries != active_gobos {
        surface_gobos.entries = active_gobos;
    }

    let mut done: std::collections::HashSet<AssetId<LensMaterial>> = std::collections::HashSet::new();
    for (lens, gt) in &lenses {
        // Shared by the LEDs of a cell: one update is enough.
        if !done.insert(lens.material.id()) {
            continue;
        }
        let out = frame.output(lens.fixture, lens.cell);
        // The disc is flipped to face the beam: its local +Z is the beam direction.
        let axis = gt.affine().transform_vector3(Vec3::Z).normalize_or_zero();
        let nits = LENS_SIDE_NITS * out.intensity;
        let half = (frame.angle(lens.fixture, lens.cell, |target| zoom_now.get(&lens.fixture).copied().unwrap_or(target)) / 2.0).to_radians();
        let params = LensParams {
            color: Vec4::new(out.color[0] * nits, out.color[1] * nits, out.color[2] * nits, LENS_BEAM_NITS / LENS_SIDE_NITS),
            // Full glare a bit inside the beam edge, fading out just beyond it.
            cone: Vec4::new((half * 1.15).min(PI).cos(), (half * 0.6).min(PI).cos(), 0.0, 0.0),
            axis: axis.extend(0.0),
        };
        // Only write real changes: `get_mut` alone makes Bevy re-upload the material.
        if lens_materials.get(&lens.material).is_some_and(|m| m.params != params)
            && let Some(mut m) = lens_materials.get_mut(&lens.material)
        {
            m.params = params;
            activity.changed = true;
        }
    }
}

/// Whether a sphere (center, radius) reaches into a spot's cone.
fn cone_hits(apex: Vec3, axis: Vec3, half: f32, range: f32, center: Vec3, radius: f32) -> bool {
    let d = center - apex;
    // A light shining half the sphere or more reaches everything in range.
    if half >= FRAC_PI_2 - 0.05 {
        return d.length() <= range + radius;
    }
    let t = d.dot(axis);
    if t < -radius || t > range + radius {
        return false;
    }
    let radial = (d - axis * t).length();
    radial <= t.max(0.0) * half.tan() + radius / half.cos().max(1e-3)
}

/// LED luminance seen from inside the beam (dazzling) and from the side:
/// low enough that colors stay saturated instead of tonemapping to white.
const LENS_BEAM_NITS: f32 = 40_000.0;
const LENS_SIDE_NITS: f32 = 250.0;


/// Share of the haze density that scatters light (the rest absorbs).
const HAZE_SCATTERING: f32 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cone_against_spheres() {
        // A beam straight down from 6 m with a 10° half angle.
        let (apex, axis, half) = (Vec3::new(0.0, 6.0, 0.0), Vec3::NEG_Y, 10f32.to_radians());
        assert!(cone_hits(apex, axis, half, 40.0, Vec3::new(0.0, 3.0, 0.0), 0.5), "right below");
        assert!(cone_hits(apex, axis, half, 40.0, Vec3::new(1.0, 1.0, 0.0), 0.3), "near the edge, lower down");
        assert!(!cone_hits(apex, axis, half, 40.0, Vec3::new(3.0, 3.0, 0.0), 0.5), "beside the beam");
        assert!(!cone_hits(apex, axis, half, 40.0, Vec3::new(0.0, 8.0, 0.0), 0.5), "behind the lens");
        assert!(cone_hits(apex, axis, PI, 40.0, Vec3::new(0.0, 8.0, 0.0), 0.5), "an all-round light reaches behind");
    }

    #[test]
    fn emitters_follow_led_cells() {
        let mut spec = resolve(&bk_olga::FixtureType { name: "x".into(), short_name: "x".into(), channels: vec![] }, &FixtureOverride::default());
        spec.cells = vec![1, 2];
        spec.cell_mode = CellMode::Physical;
        spec.leds = Some(LedLayout { shape: LedShape::Rect { rows: 1, cols: 4 }, pitch: 0.1, led_cells: vec![1, 1, 2, 0] });
        let positions: Vec<Vec2> = spec.leds.as_ref().unwrap().shape.positions(0.1).into_iter().map(Vec2::from_array).collect();
        let plans = plan_emitters(&spec, &positions, Quality::Medium);
        assert_eq!(plans.len(), 3);
        assert_eq!(plans[0].cells, [Some(1)]);
        assert!((plans[0].pos.x + 0.1).abs() < 1e-6, "between the first two LEDs");
        assert_eq!(plans[2].cells, [None]);
        assert!((plans.iter().map(|p| p.share).sum::<f32>() - 1.0).abs() < 1e-6);

        spec.cell_mode = CellMode::Mixed;
        let mixed = &plan_emitters(&spec, &positions, Quality::Medium)[0];
        assert_eq!(mixed.cells, [Some(1), Some(2), None]);
        // Cell 1 has two of the four LEDs, so it counts twice.
        assert_eq!(mixed.weights, [2.0, 1.0, 1.0]);
    }
}
