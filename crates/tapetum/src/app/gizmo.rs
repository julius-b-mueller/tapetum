//! Moving fixtures with the mouse. A click on any part of a fixture selects
//! it, a right click its BK group (a menu asks if it is in several); with
//! Shift or Cmd/Ctrl both add to the selection. Drag the arrows to move along BK's X/Y/Z or the rings
//! to rotate the selection around its center. Turning snaps to 15° steps
//! unless Shift is held; moving snaps to 10 cm only with Shift.
//! Placements go to BlinderKitten while dragging.

use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bk_olga::placement::rotate_around_world_axis;
use tapetum_project::Placement;

use super::fixtures::{BeamVolume, FixtureRoot};
use super::objects::{SceneRuntime, StageObjectRoot, StageObjects, is_igz, split_key};
use super::layout;
use super::state::{PlacementSync, ShowState};
use super::ui::UiRects;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move(usize),
    Rotate(usize),
}

/// Something the gizmo moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Fixture by index.
    Fixture(usize),
    /// A stage object's place in a scene, by object key.
    Object(String),
    /// A whole model file in the base scene (not IngeTrazo files), by ID.
    File(u64),
}

#[derive(Clone, Debug)]
struct Drag {
    handle: Handle,
    /// Center of the selection when the drag began (world).
    center: Vec3,
    /// Everything dragged, with its placement at the start.
    start: Vec<(Target, Placement)>,
    /// Axis parameter (move) or in-plane vector (rotate) where the drag began.
    grab: Vec3,
}

/// Choice between a fixture's groups, opened next to the mouse.
#[derive(Clone, Debug)]
pub struct GroupMenu {
    pub fixture: usize,
    /// Window position (egui points).
    pub pos: Vec2,
    /// Add the chosen group to the selection instead of replacing it.
    pub add: bool,
}

#[derive(Resource, Default)]
pub struct GizmoState {
    drag: Option<Drag>,
    hovered: Option<Handle>,
    press: Option<Vec2>,
    press_right: Option<Vec2>,
    pub menu: Option<GroupMenu>,
}

impl GizmoState {
    /// The left mouse button belongs to the gizmo (the camera must not orbit).
    pub fn active(&self) -> bool {
        self.drag.is_some() || self.hovered.is_some() && self.press.is_some()
    }
}

const COLORS: [Color; 3] = [Color::srgb(0.95, 0.25, 0.25), Color::srgb(0.3, 0.9, 0.3), Color::srgb(0.3, 0.5, 1.0)];
const HOVER: Color = Color::srgb(1.0, 0.85, 0.2);
/// The middle of a moving head's pan range, and its ends.
const PAN_CENTRE: Color = Color::srgb(0.25, 0.85, 1.0);
const PAN_END: Color = Color::srgba(0.25, 0.85, 1.0, 0.35);

/// BK (Z up) to Bevy world (Y up), the rotation of `BkSpace`.
pub fn bk_to_world(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[2], -p[1])
}

fn world_to_bk(v: Vec3) -> [f32; 3] {
    [v.x, -v.z, v.y]
}

/// BK's X, Y and Z axes in world space.
fn axis(i: usize) -> Vec3 {
    let mut a = [0.0; 3];
    a[i] = 1.0;
    bk_to_world(a)
}

/// Cursor ray and helpers to go between window and 3D image coordinates
/// (the 3D view renders at its own resolution).
struct View<'a> {
    camera: &'a Camera,
    gt: &'a GlobalTransform,
    scale: Vec2,
}

impl View<'_> {
    fn ray(&self, cursor: Vec2) -> Option<Ray3d> {
        self.camera.viewport_to_world(self.gt, cursor * self.scale).ok()
    }

    fn screen(&self, p: Vec3) -> Option<Vec2> {
        self.camera.world_to_viewport(self.gt, p).ok().map(|v| v / self.scale)
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Parameter on the line `o + t * a` closest to the ray.
fn closest_on_axis(o: Vec3, a: Vec3, ray: Ray3d) -> Option<f32> {
    let d = *ray.direction;
    let w = o - ray.origin;
    let (aa, ad, dd) = (a.dot(a), a.dot(d), d.dot(d));
    let denom = aa * dd - ad * ad;
    // Looking straight along the axis: no meaningful drag.
    (denom.abs() > 1e-4).then(|| (ad * d.dot(w) - dd * a.dot(w)) / denom)
}

/// Where the ray meets the plane through `o` with normal `n`, relative to `o`.
fn on_plane(o: Vec3, n: Vec3, ray: Ray3d) -> Option<Vec3> {
    let denom = ray.direction.dot(n);
    if denom.abs() < 1e-4 {
        return None;
    }
    let s = (o - ray.origin).dot(n) / denom;
    (s > 0.0).then(|| ray.origin + *ray.direction * s - o)
}

/// Finds which fixture a cursor ray hits, on any of its parts.
#[derive(bevy::ecs::system::SystemParam)]
pub struct FixturePicker<'w, 's> {
    ray_cast: MeshRayCast<'w, 's>,
    parents: Query<'w, 's, &'static ChildOf>,
    roots: Query<'w, 's, &'static FixtureRoot>,
    objects: Query<'w, 's, &'static StageObjectRoot>,
    ignored: Query<'w, 's, (), With<BeamVolume>>,
}

impl FixturePicker<'_, '_> {
    /// The fixture or object under the ray; locked objects are looked through.
    fn pick(&mut self, ray: Ray3d, locked: impl Fn(&str) -> bool) -> Option<Target> {
        let ignored = &self.ignored;
        let filter = |e: Entity| !ignored.contains(e);
        let settings = MeshRayCastSettings::default().with_filter(&filter).never_early_exit();
        let hits: Vec<Entity> = self.ray_cast.cast_ray(ray, &settings).iter().map(|(e, _)| *e).collect();
        hits.into_iter().find_map(|hit| match self.owner(hit)? {
            Target::Object(key) if locked(&key) => None,
            t => Some(t),
        })
    }

    /// Walks up from a hit part to the fixture or object it belongs to.
    fn owner(&self, mut e: Entity) -> Option<Target> {
        loop {
            if let Ok(root) = self.roots.get(e) {
                return Some(Target::Fixture(root.index));
            }
            if let Ok(root) = self.objects.get(e) {
                return Some(Target::Object(root.key.clone()));
            }
            e = self.parents.get(e).ok()?.parent();
        }
    }
}

pub fn gizmo(
    mut state: ResMut<ShowState>,
    mut gs: ResMut<GizmoState>,
    mut sync: ResMut<PlacementSync>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    egui: Res<EguiWantsInput>,
    ui_rects: Res<UiRects>,
    mut picker: FixturePicker,
    (objs, rt): (Res<StageObjects>, Res<SceneRuntime>),
    mut gizmos: Gizmos,
) {
    let (Ok(window), Ok((camera, gt))) = (window.single(), camera.single()) else { return };
    let Some(view_size) = camera.logical_viewport_size() else { return };
    let view = View { camera, gt, scale: view_size / window.size().max(Vec2::ONE) };
    let cursor = window.cursor_position();
    let over_ui = egui.wants_any_pointer_input() || cursor.is_some_and(|c| ui_rects.contains(c));
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let add = [KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::ShiftLeft, KeyCode::ShiftRight]
        .iter()
        .any(|k| keys.pressed(*k));

    let targets = selected_targets(&state, &objs, rt.active);

    // Dragging.
    if let (Some(drag), Some(c)) = (gs.drag.clone(), cursor) {
        if !buttons.pressed(MouseButton::Left) {
            gs.drag = None;
            state.save_project();
        } else if let Some(ray) = view.ray(c) {
            for (target, p) in drag_placements(&drag, ray, shift) {
                match target {
                    Target::Fixture(i) => {
                        let f = &state.show.fixtures[i];
                        let (id, short) = (f.id, f.short_name.clone());
                        if state.project.pending_placements.get(&id) != Some(&p) {
                            state.project.pending_placements.insert(id, p);
                            sync.queue(id, &short, p);
                        }
                    }
                    Target::File(id) => {
                        let current = state.project.objects.iter().find(|o| o.id == id).map(|o| o.transform);
                        if let Some(t) = current
                            && (t.translation != p.position || t.rotation != p.rotation)
                            && let Some(o) = state.project.objects.iter_mut().find(|o| o.id == id)
                        {
                            o.transform.translation = p.position;
                            o.transform.rotation = p.rotation;
                        }
                    }
                    Target::Object(key) => {
                        // Scenes store where the object is relative to the base scene.
                        let (Some(scene), Some(pivot)) = (rt.active, objs.pivot(&key)) else { continue };
                        let offset = (Vec3::from(p.position) - pivot).to_array();
                        let current = state.project.scenes.iter().find(|s| s.id == scene).map(|s| s.objects.get(&key).copied().unwrap_or_default());
                        if current.is_some_and(|c| c.offset != offset || c.rotation != p.rotation)
                            && let Some(s) = state.project.scenes.iter_mut().find(|s| s.id == scene)
                        {
                            let entry = s.objects.entry(key).or_default();
                            entry.offset = offset;
                            entry.rotation = p.rotation;
                        }
                    }
                }
            }
        }
    }

    // Selected things get a marker; the handles sit at their center.
    if targets.is_empty() {
        gs.hovered = None;
        handle_click(&mut state, &mut gs, &buttons, cursor, over_ui, add, &view, &mut picker);
        return;
    }
    let positions: Vec<Vec3> = targets.iter().map(|(_, p)| bk_to_world(p.position)).collect();
    let center = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
    let size = gt.translation().distance(center) * 0.12;
    // Fixtures get a marker; stage objects show their outline instead.
    for (p, (t, placement)) in positions.iter().zip(&targets) {
        if let Target::Fixture(i) = t {
            gizmos.sphere(Isometry3d::from_translation(*p), size * 0.08, HOVER);
            pan_centre(&mut gizmos, &state, *i, placement, *p, size);
        }
    }
    if gs.drag.is_none() {
        gs.hovered = cursor.filter(|_| !over_ui).and_then(|c| hovered_handle(&view, c, center, size));
    }
    if buttons.just_pressed(MouseButton::Left)
        && !over_ui
        && let (Some(handle), Some(c)) = (gs.hovered, cursor)
        && let Some(ray) = view.ray(c)
    {
        let grab = match handle {
            Handle::Move(a) => closest_on_axis(center, axis(a), ray).map(|t| Vec3::new(t, 0.0, 0.0)),
            Handle::Rotate(a) => on_plane(center, axis(a), ray),
        };
        if let Some(grab) = grab {
            gs.drag = Some(Drag { handle, center, start: targets.clone(), grab });
            gs.menu = None;
        }
    }
    draw(&mut gizmos, center, size, gs.drag.as_ref().map(|d| d.handle).or(gs.hovered));
    if gs.drag.is_none() {
        handle_click(&mut state, &mut gs, &buttons, cursor, over_ui, add, &view, &mut picker);
    }
}

/// A moving head's middle direction: where it points level at the middle of
/// its pan range (ideally the stage); faint lines to the ends of the range.
fn pan_centre(gizmos: &mut Gizmos, state: &ShowState, i: usize, placement: &Placement, o: Vec3, size: f32) {
    let Some(spec) = state.specs.get(i).and_then(|s| s.as_ref()).filter(|s| s.class == bk_olga::FixtureClass::MovingHead) else { return };
    let w = bk_olga::placement::world_from_local(placement.rotation);
    // Level (tilt −90°: BK's forward) at a pan angle, in the world.
    let dir = |pan: f32| bk_to_world(bk_olga::placement::mul(&w, bk_olga::placement::beam_local(pan, -90.0))).normalize_or_zero();
    let [lo, hi] = spec.pan_range;
    let len = size * 1.1;
    gizmos.arrow(o, o + dir((lo + hi) / 2.0) * len, PAN_CENTRE);
    if (hi - lo).abs() < 359.0 {
        gizmos.line(o, o + dir(lo) * len * 0.6, PAN_END);
        gizmos.line(o, o + dir(hi) * len * 0.6, PAN_END);
    }
}

/// The selected fixtures, or the selected stage object, with their placements.
/// In the base scene an object moves its whole file, which IngeTrazo files
/// can't (they are placed in IngeTrazo); in a scene it moves on its own.
fn selected_targets(state: &ShowState, objs: &StageObjects, scene: Option<u64>) -> Vec<(Target, Placement)> {
    if !state.selected_objects.is_empty() {
        let mut out = Vec::new();
        for key in &state.selected_objects {
            let Some((file_id, _)) = split_key(key) else { continue };
            let Some(file) = state.project.objects.iter().find(|o| o.id == file_id) else { continue };
            match scene {
                Some(_) => {
                    let Some(pivot) = objs.pivot(key) else { continue };
                    let s = super::objects::scene_state(state, scene, key);
                    let position = (pivot + Vec3::from(s.offset)).to_array();
                    out.push((Target::Object(key.clone()), Placement { position, rotation: s.rotation }));
                }
                None if is_igz(file) => {}
                None => {
                    // The base scene moves whole files, each once.
                    if !out.iter().any(|(t, _)| *t == Target::File(file_id)) {
                        out.push((Target::File(file_id), Placement { position: file.transform.translation, rotation: file.transform.rotation }));
                    }
                }
            }
        }
        return out;
    }
    let n = state.show.fixtures.len();
    let selection: Vec<usize> = state.selection.iter().copied().filter(|&i| i < n).collect();
    if selection.is_empty() {
        return Vec::new();
    }
    let placements = layout::placements(&state.show, &state.project);
    selection.into_iter().map(|i| (Target::Fixture(i), placements[i])).collect()
}

/// New placements of the dragged things for the current mouse ray.
/// `shift`: moving snaps to 10 cm, turning goes without its 15° steps.
fn drag_placements(drag: &Drag, ray: Ray3d, shift: bool) -> Vec<(Target, Placement)> {
    let o = drag.center;
    match drag.handle {
        Handle::Move(a) => {
            let Some(t) = closest_on_axis(o, axis(a), ray) else { return Vec::new() };
            let mut d = t - drag.grab.x;
            if shift {
                d = (d * 10.0).round() / 10.0;
            }
            let offset = axis(a) * d;
            drag.start.iter().map(|(i, p)| (i.clone(), Placement { position: world_to_bk(bk_to_world(p.position) + offset), ..*p })).collect()
        }
        Handle::Rotate(a) => {
            let Some(v) = on_plane(o, axis(a), ray) else { return Vec::new() };
            let n = axis(a);
            let mut angle = n.dot(drag.grab.cross(v)).atan2(drag.grab.dot(v)).to_degrees();
            if !shift {
                angle = (angle / 15.0).round() * 15.0;
            }
            // Positions turn around the center, and every fixture's orientation
            // turns around the same world axis (not around its own tilted axes).
            let turn = Quat::from_axis_angle(n, angle.to_radians());
            drag.start
                .iter()
                .map(|(i, p)| {
                    let position = world_to_bk(o + turn * (bk_to_world(p.position) - o));
                    (i.clone(), Placement { position, rotation: rotate_around_world_axis(p.rotation, a, angle) })
                })
                .collect()
        }
    }
}

fn hovered_handle(view: &View, cursor: Vec2, o: Vec3, size: f32) -> Option<Handle> {
    let so = view.screen(o)?;
    let mut best: Option<(f32, Handle)> = None;
    let mut consider = |d: f32, h: Handle| {
        if d < 8.0 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, h));
        }
    };
    for i in 0..3 {
        if let Some(tip) = view.screen(o + axis(i) * size) {
            consider(segment_distance(cursor, so, tip), Handle::Move(i));
        }
        let ring: Vec<Vec2> = ring_points(o, axis(i), size * RING).into_iter().filter_map(|p| view.screen(p)).collect();
        for w in ring.windows(2) {
            consider(segment_distance(cursor, w[0], w[1]), Handle::Rotate(i));
        }
    }
    best.map(|(_, h)| h)
}

const RING: f32 = 0.75;

fn ring_points(o: Vec3, n: Vec3, r: f32) -> Vec<Vec3> {
    let rot = Quat::from_rotation_arc(Vec3::Z, n);
    (0..=48).map(|k| o + rot * (Vec2::from_angle(k as f32 / 48.0 * std::f32::consts::TAU) * r).extend(0.0)).collect()
}

fn draw(gizmos: &mut Gizmos, o: Vec3, size: f32, active: Option<Handle>) {
    for (i, &base) in COLORS.iter().enumerate() {
        let color = |h: Handle| if active == Some(h) { HOVER } else { base };
        gizmos.arrow(o, o + axis(i) * size, color(Handle::Move(i)));
        gizmos.linestrip(ring_points(o, axis(i), size * RING), color(Handle::Rotate(i)));
    }
}

/// Clicks without moving the mouse: left selects the fixture under the cursor,
/// right its group (or asks which, if it has several); with `add` (Shift or
/// Cmd/Ctrl) both extend the selection. A click on empty space clears it.
#[allow(clippy::too_many_arguments)]
fn handle_click(
    state: &mut ShowState,
    gs: &mut GizmoState,
    buttons: &ButtonInput<MouseButton>,
    cursor: Option<Vec2>,
    over_ui: bool,
    add: bool,
    view: &View,
    picker: &mut FixturePicker,
) {
    if buttons.just_pressed(MouseButton::Left) {
        gs.press = cursor.filter(|_| !over_ui);
    }
    if buttons.just_pressed(MouseButton::Right) {
        gs.press_right = cursor.filter(|_| !over_ui);
    }
    let left = buttons.just_released(MouseButton::Left).then(|| gs.press.take()).flatten();
    let right = buttons.just_released(MouseButton::Right).then(|| gs.press_right.take()).flatten();
    let Some(c) = cursor else { return };
    let clicked = |press: Option<Vec2>| press.is_some_and(|p| p.distance(c) <= 4.0);
    if !clicked(left) && !clicked(right) {
        return;
    }
    // A click on the handles or in the 3D view closes an open menu.
    gs.menu = None;
    if clicked(left) && gs.hovered.is_some() {
        return;
    }
    let st: &ShowState = state;
    let hit = view.ray(c).and_then(|ray| picker.pick(ray, |key| super::objects::is_locked(st, key)));
    if let Some(Target::Object(key)) = &hit {
        if add {
            state.toggle_object(key.clone());
        } else if state.selected_objects != [key.clone()] {
            state.select_object(key.clone());
        }
        return;
    }
    let hit = hit.and_then(|t| match t {
        Target::Fixture(i) => Some(i),
        _ => None,
    });
    if clicked(right) {
        let Some(i) = hit else { return };
        match state.groups_of(i)[..] {
            // Not in any group: the fixture itself.
            [] if add => state.toggle_fixture(i),
            [] => state.select_fixture(i),
            [g] if add => state.add_group(g, Some(i)),
            [g] => state.select_group(g, Some(i)),
            _ => gs.menu = Some(GroupMenu { fixture: i, pos: c, add }),
        }
        return;
    }
    match hit {
        Some(i) if add => state.toggle_fixture(i),
        Some(i) if state.selection != [i] => state.select_fixture(i),
        Some(_) => {}
        None if !state.selection.is_empty() || !state.selected_objects.is_empty() => state.clear_selection(),
        None => {}
    }
}

/// Moves the fixture models to their current placements.
pub fn apply_placements(state: Res<ShowState>, mut roots: Query<(&super::fixtures::FixtureRoot, &mut Transform)>) {
    if !state.is_changed() {
        return;
    }
    let placements = layout::placements(&state.show, &state.project);
    for (root, mut tf) in &mut roots {
        let Some(p) = placements.get(root.index) else { continue };
        let target = Transform::from_translation(Vec3::from(p.position)).with_rotation(super::fixtures::placement_rotation(p.rotation));
        tf.set_if_neq(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bk_world_roundtrip() {
        let p = [1.0, 2.0, 3.0];
        assert_eq!(world_to_bk(bk_to_world(p)), p);
        // BK Z (up) is world Y.
        assert_eq!(axis(2), Vec3::Y);
    }

    #[test]
    fn axis_drag_follows_the_ray() {
        // Ray from the front, crossing the X axis at x = 2.
        let ray = Ray3d::new(Vec3::new(2.0, 0.0, 10.0), Dir3::NEG_Z);
        let t = closest_on_axis(Vec3::ZERO, Vec3::X, ray).unwrap();
        assert!((t - 2.0).abs() < 1e-5);
    }

    #[test]
    fn group_rotates_around_its_center() {
        // Two fixtures 2 m apart at 6 m height, turned 90° around BK Z (world Y).
        let a = Placement { position: [-1.0, 0.0, 6.0], rotation: [0.0; 3] };
        let b = Placement { position: [1.0, 0.0, 6.0], rotation: [0.0; 3] };
        let center = bk_to_world([0.0, 0.0, 6.0]);
        let grab = Vec3::X; // grabbed the ring on +X ...
        let drag = Drag { handle: Handle::Rotate(2), center, start: vec![(Target::Fixture(0), a), (Target::Fixture(1), b)], grab };
        // ... and dragged to -Z (world), which is BK +Y: a quarter turn counterclockwise from above.
        let ray = Ray3d::new(center + Vec3::new(0.0, 5.0, -1.0), Dir3::NEG_Y);
        let moved = drag_placements(&drag, ray, false);
        let pa = moved[0].1.position;
        let pb = moved[1].1.position;
        assert!((pa[0]).abs() < 1e-4 && (pa[1] + 1.0).abs() < 1e-4, "{pa:?}");
        assert!((pb[0]).abs() < 1e-4 && (pb[1] - 1.0).abs() < 1e-4, "{pb:?}");
        assert!((moved[0].1.rotation[2].abs() - 90.0).abs() < 1e-3);
    }

}
