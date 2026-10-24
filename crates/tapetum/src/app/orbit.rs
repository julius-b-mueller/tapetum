//! Orbit camera: left drag rotates, right/middle drag (or shift + left) pans,
//! scroll zooms. Input over egui panels is ignored. A SpaceMouse moves it too.

use bevy::camera::ScalingMode;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use std::f32::consts::FRAC_PI_2;
use tapetum_project::StandardView;

use super::fixtures::FixtureRoot;
use super::gizmo::GizmoState;
use super::objects::{FrameBoxes, Occluders, Placeholder};
use super::spacemouse::SpaceMouse;
use super::state::ShowState;
use super::ui::UiRects;

/// Frame everything from a standard view once `delay` frames have passed
/// (loading models takes a moment); with `glide` the camera swings there.
#[derive(Resource, Default)]
pub struct FitRequest {
    pub view: Option<StandardView>,
    pub delay: u32,
    pub glide: bool,
}

/// The camera was moved by hand: it has left its standard view.
#[derive(Resource, Default)]
pub struct MovedByHand(pub bool);

/// Seconds the camera takes to swing to a standard view.
const GLIDE_SECS: f32 = 0.6;

/// Where an orthographic camera stands back from its target (the scale comes
/// from `distance`, so it never ends up inside the set).
const ORTHO_BACK: f32 = 150.0;

/// Bevy's default vertical field of view is 45°.
pub fn tan_half_fov() -> f32 {
    std::f32::consts::FRAC_PI_8.tan()
}

/// Yaw and pitch of a standard view (BK -Y, IngeTrazo's front, is the audience).
pub fn angles(view: StandardView) -> (f32, f32) {
    match view {
        StandardView::Audience => (0.0, 0.18),
        StandardView::Top => (0.0, FRAC_PI_2),
        StandardView::Front => (0.0, 0.0),
        StandardView::Left => (-FRAC_PI_2, 0.0),
        StandardView::Right => (FRAC_PI_2, 0.0),
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    /// Radians around world Y.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
}

impl OrbitCamera {
    /// From the audience: BK (0, -14, 4) looking at (0, 0, 1.5).
    pub fn audience() -> Self {
        OrbitCamera { target: Vec3::new(0.0, 1.5, 0.0), distance: 14.5, yaw: 0.0, pitch: 0.18 }
    }

    /// A standard view of the box (Bevy world) that just fits the part of the
    /// window the sidebar leaves free. `view` is the window size, `left` the
    /// width the sidebar covers. Orthographic, `distance` sets the scale.
    pub fn framing(lo: Vec3, hi: Vec3, standard: StandardView, orthographic: bool, view: Vec2, left: f32) -> Self {
        let center = (lo + hi) / 2.0;
        let (yaw, pitch) = angles(standard);
        let mut cam = OrbitCamera { target: center, distance: 1.0, yaw, pitch };
        let rot = cam.rotation();
        let (right, up, forward) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::NEG_Z);
        let tan_v = tan_half_fov();
        let tan_full = tan_v * view.x / view.y.max(1.0);
        let tan_free = tan_v * (view.x - left).max(view.x * 0.3) / view.y.max(1.0);
        // Distance at which every corner is inside the free part (from its middle);
        // orthographic, depth doesn't matter.
        let mut distance: f32 = 1.0;
        for i in 0..8 {
            let corner = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
            let d = corner - center;
            let ahead = if orthographic { 0.0 } else { d.dot(forward) };
            distance = distance.max(d.dot(right).abs() / tan_free - ahead).max(d.dot(up).abs() / tan_v - ahead);
        }
        cam.distance = distance * 1.08;
        // Move the picture to the middle of the free part: the free part's
        // middle is `left / width` of the way right in normalized screen x.
        cam.target -= right * (left / view.x.max(1.0)) * cam.distance * tan_full;
        cam
    }

    fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, -self.pitch, 0.0)
    }

    /// The camera looks along its -Z at the target (also straight down).
    pub fn transform(&self) -> Transform {
        let rot = self.rotation();
        Transform::from_translation(self.target + rot * Vec3::new(0.0, 0.0, self.distance)).with_rotation(rot)
    }

    /// Orthographic: further back, same direction.
    pub fn transform_ortho(&self) -> Transform {
        let rot = self.rotation();
        Transform::from_translation(self.target + rot * Vec3::new(0.0, 0.0, self.distance + ORTHO_BACK)).with_rotation(rot)
    }

    /// On the way from `a` to `b` (t 0..1, eased): yaw the short way round,
    /// distance evenly in scale.
    fn between(a: &Self, b: &Self, t: f32) -> Self {
        let t = t * t * (3.0 - 2.0 * t);
        let tau = std::f32::consts::TAU;
        let dyaw = (b.yaw - a.yaw + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
        OrbitCamera {
            target: a.target.lerp(b.target, t),
            distance: (a.distance.ln() + (b.distance.ln() - a.distance.ln()) * t).exp(),
            yaw: a.yaw + dyaw * t,
            pitch: a.pitch + (b.pitch - a.pitch) * t,
        }
    }
}

/// The camera swinging to a standard view.
pub struct Glide {
    from: OrbitCamera,
    to: OrbitCamera,
    start: f32,
}

pub fn orbit_camera(
    mut q: Query<(&mut OrbitCamera, &mut Transform, &mut Projection)>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    ui_rects: Res<UiRects>,
    window: Query<&Window, With<PrimaryWindow>>,
    gizmo: Res<GizmoState>,
    (spacemouse, state, time): (Res<SpaceMouse>, Res<ShowState>, Res<Time>),
    (mut last_buttons, mut active, mut fit, mut glide, mut moved): (Local<u32>, Local<Option<bool>>, ResMut<FitRequest>, Local<Option<Glide>>, ResMut<MovedByHand>),
    (fixtures, frames, placeholder): (Query<&GlobalTransform, With<FixtureRoot>>, Res<FrameBoxes>, Query<&ViewVisibility, With<Placeholder>>),
) {
    let Ok((mut cam, mut tf, mut projection)) = q.single_mut() else { return };
    let orthographic = state.project.view.orthographic;
    let before = *cam;
    // Normally only while Tapetum has the focus: other programs get the device otherwise.
    let settings = state.project.spacemouse;
    let wanted = settings.enabled && (settings.keep_in_background || window.single().is_ok_and(|w| w.focused));
    if *active != Some(wanted) {
        spacemouse.set_active(wanted);
        *active = Some(wanted);
    }
    if wanted {
        let a = spacemouse.axes();
        let k = settings.speed * time.delta_secs();
        // Moving the stage is moving the camera the other way; only tilting
        // keeps its sense (tilting the cap towards you tilts the stage towards you).
        let (m, t) = if settings.move_stage { (-1.0, -1.0) } else { (1.0, 1.0) };
        let right = tf.rotation * Vec3::X;
        let up = tf.rotation * Vec3::Y;
        // Cap right/up moves the camera right/up, pulling it towards you moves it back.
        let step = (right * a.translation.x - up * a.translation.z) * m * cam.distance * 0.8 * k;
        cam.target += step;
        cam.distance = (cam.distance * (a.translation.y * m * 1.5 * k).exp()).clamp(0.5, 200.0);
        // Twisting turns around the stage, tilting forward looks down.
        cam.yaw -= a.rotation.z * t * 1.5 * k;
        cam.pitch = (cam.pitch + a.rotation.x * 1.2 * k).clamp(-FRAC_PI_2, FRAC_PI_2);

        let buttons = spacemouse.buttons();
        let pressed = buttons & !*last_buttons;
        if pressed & 3 != 0 {
            *fit = FitRequest { view: Some(if pressed & 1 == 0 { StandardView::Top } else { StandardView::Audience }), delay: 0, glide: true };
        }
        *last_buttons = buttons;
    }
    // The SpaceMouse moving the camera ends a swing.
    let mut by_hand = *cam != before;
    if let Some(standard) = fit.view {
        if fit.delay > 0 {
            fit.delay -= 1;
        } else {
            let (lo, hi) = frame_bounds(&fixtures, &frames, placeholder.iter().any(|v| v.get()));
            let view = window.single().map_or(Vec2::new(1600.0, 960.0), |w| w.size());
            // The sidebar: panels starting at the left edge.
            let left = ui_rects.0.iter().filter(|r| r.min.x < 1.0 && r.height() > view.y * 0.5).map(|r| r.max.x).fold(0.0, f32::max);
            let to = OrbitCamera::framing(lo, hi, standard, orthographic, view, left);
            if fit.glide {
                *glide = Some(Glide { from: *cam, to, start: time.elapsed_secs() });
            } else {
                *cam = to;
            }
            fit.view = None;
            by_hand = false;
        }
    }
    let over_panel = window.single().ok().and_then(|w| w.cursor_position()).is_some_and(|p| ui_rects.contains(p));
    if !egui.wants_any_pointer_input() && !over_panel {
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let delta = motion.delta;
        // Shift + left drag pans, except while the gizmo has the mouse: there
        // Shift turns without snapping.
        let panning = buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) || (shift && buttons.pressed(MouseButton::Left) && !gizmo.active());
        if panning {
            let right = tf.rotation * Vec3::X;
            let up = tf.rotation * Vec3::Y;
            let k = cam.distance * 0.0015;
            cam.target += (-right * delta.x + up * delta.y) * k;
        } else if buttons.pressed(MouseButton::Left) && !gizmo.active() {
            cam.yaw -= delta.x * 0.005;
            cam.pitch = (cam.pitch + delta.y * 0.005).clamp(-FRAC_PI_2, FRAC_PI_2);
        }
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if lines != 0.0 {
            cam.distance = (cam.distance * (1.0 - lines * 0.1)).clamp(0.5, 200.0);
        }
        by_hand |= panning || (buttons.pressed(MouseButton::Left) && !gizmo.active() && delta != Vec2::ZERO) || lines != 0.0;
    }
    if by_hand {
        *glide = None;
        moved.0 = true;
    }
    if let Some(g) = &*glide {
        let t = ((time.elapsed_secs() - g.start) / GLIDE_SECS).clamp(0.0, 1.0);
        *cam = OrbitCamera::between(&g.from, &g.to, t);
        if t >= 1.0 {
            *glide = None;
        }
    }
    // Orthographic: as much in view as perspective shows at the target's distance.
    let height = 2.0 * cam.distance * tan_half_fov();
    // Read first: touching it mutably marks it changed every frame.
    let wanted = match (&*projection, orthographic) {
        (Projection::Orthographic(o), true) if matches!(o.scaling_mode, ScalingMode::FixedVertical { viewport_height } if viewport_height == height) => None,
        (Projection::Orthographic(_), false) => Some(Projection::Perspective(PerspectiveProjection::default())),
        (_, true) => Some(Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical { viewport_height: height },
            far: ORTHO_BACK * 2.0 + 200.0,
            ..OrthographicProjection::default_3d()
        })),
        _ => None,
    };
    if let Some(p) = wanted {
        *projection = p;
    }
    *tf = if orthographic { cam.transform_ortho() } else { cam.transform() };
}

/// What a standard view frames (Bevy world): the fixtures and the visible stage
/// objects that aren't locked (the set, not the auditorium around it); all of
/// them if every one is locked. The placeholder stage while there are none.
pub fn frame_bounds(fixtures: &Query<&GlobalTransform, With<FixtureRoot>>, frames: &FrameBoxes, placeholder: bool) -> (Vec3, Vec3) {
    let mut boxes: Vec<(Vec3, Vec3)> = fixtures.iter().map(|gt| (gt.translation() - 0.3, gt.translation() + 0.3)).collect();
    let free = frames.0.iter().any(|(_, _, locked)| !locked);
    boxes.extend(frames.0.iter().filter(|(_, _, locked)| !free || !locked).map(|(lo, hi, _)| (*lo, *hi)));
    if placeholder || boxes.is_empty() {
        boxes.extend([(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 1.0, 3.0))]);
    }
    let lo = boxes.iter().fold(Vec3::splat(f32::MAX), |lo, (l, _)| lo.min(*l));
    let hi = boxes.iter().fold(Vec3::splat(f32::MIN), |hi, (_, h)| hi.max(*h));
    (lo, hi)
}

/// Box (Bevy world) around the fixtures and the visible stage objects; the
/// placeholder stage counts while there are no objects.
pub fn scene_bounds(fixtures: &Query<&GlobalTransform, With<FixtureRoot>>, occluders: &Occluders, placeholder: bool) -> (Vec3, Vec3) {
    let mut boxes: Vec<(Vec3, f32)> = fixtures.iter().map(|gt| (gt.translation(), 0.3)).collect();
    boxes.extend(occluders.0.iter().copied());
    if placeholder || boxes.is_empty() {
        // The placeholder platform (10 x 1 x 8 m around (0, 0.5, -1)) and the floor under it.
        boxes.extend([(Vec3::new(-5.0, 0.0, -5.0), 0.0), (Vec3::new(5.0, 1.0, 3.0), 0.0)]);
    }
    let lo = boxes.iter().fold(Vec3::splat(f32::MAX), |lo, &(p, r)| lo.min(p - r));
    let hi = boxes.iter().fold(Vec3::splat(f32::MIN), |hi, &(p, r)| hi.max(p + r));
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swings_the_short_way_and_ends_there() {
        let a = OrbitCamera { target: Vec3::ZERO, distance: 10.0, yaw: 3.0, pitch: 0.0 };
        let b = OrbitCamera { target: Vec3::X, distance: 40.0, yaw: -3.0, pitch: 1.0 };
        let mid = OrbitCamera::between(&a, &b, 0.5);
        // From 3.0 to -3.0 rad the short way passes π, not 0.
        assert!(mid.yaw.abs() > 3.0, "{}", mid.yaw);
        assert!((mid.distance - 20.0).abs() < 0.01, "evenly in scale: {}", mid.distance);
        let end = OrbitCamera::between(&a, &b, 1.0);
        assert!((end.target - b.target).length() < 1e-5 && (end.pitch - b.pitch).abs() < 1e-5);
    }

    #[test]
    fn standard_views_look_where_they_should() {
        let (lo, hi) = (Vec3::new(-5.0, 0.0, -4.0), Vec3::new(5.0, 6.0, 4.0));
        let forward = |v| OrbitCamera::framing(lo, hi, v, false, Vec2::new(1600.0, 900.0), 0.0).transform().forward();
        assert!(forward(StandardView::Top).dot(Vec3::NEG_Y) > 0.999);
        assert!(forward(StandardView::Front).dot(Vec3::NEG_Z) > 0.999, "towards the stage (BK +Y)");
        assert!(forward(StandardView::Left).dot(Vec3::X) > 0.999);
        assert!(forward(StandardView::Right).dot(Vec3::NEG_X) > 0.999);
        // From the top the audience (Bevy +Z) is at the bottom of the picture.
        let top = OrbitCamera::framing(lo, hi, StandardView::Top, false, Vec2::new(1600.0, 900.0), 0.0).transform();
        assert!(top.up().dot(Vec3::NEG_Z) > 0.999);
        // Orthographic, the scale fits the box: half its height on screen.
        let o = OrbitCamera::framing(lo, hi, StandardView::Front, true, Vec2::new(1600.0, 900.0), 0.0);
        assert!((o.distance * tan_half_fov() - 3.0 * 1.08).abs() < 0.01, "{}", o.distance);
    }
}
