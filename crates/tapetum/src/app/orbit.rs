//! Orbit camera: left drag rotates, right/middle drag (or shift + left) pans,
//! scroll zooms. Input over egui panels is ignored. A SpaceMouse moves it too.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;

use super::fixtures::FixtureRoot;
use super::gizmo::GizmoState;
use super::objects::{Occluders, Placeholder};
use super::spacemouse::SpaceMouse;
use super::state::ShowState;
use super::ui::UiRects;

/// Frame everything: from the audience (`top: false`) or from above, once
/// `delay` frames have passed (loading models takes a moment).
#[derive(Resource, Default)]
pub struct FitRequest {
    pub top: Option<bool>,
    pub delay: u32,
}

#[derive(Component)]
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

    /// From the audience (BK -Y, IngeTrazo's front view too), slightly from
    /// above, or straight from the top with the audience at the bottom, so the
    /// box (Bevy world) just fits the part of the view the sidebar leaves free.
    /// `view` is the window size, `left` the width the sidebar covers.
    pub fn framing(lo: Vec3, hi: Vec3, top: bool, view: Vec2, left: f32) -> Self {
        let center = (lo + hi) / 2.0;
        let mut cam = OrbitCamera { target: center, distance: 1.0, yaw: 0.0, pitch: if top { 1.5 } else { 0.18 } };
        let rot = cam.transform().rotation;
        let (right, up, forward) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::NEG_Z);
        // Bevy's default vertical field of view is 45°.
        let tan_v = std::f32::consts::FRAC_PI_8.tan();
        let tan_full = tan_v * view.x / view.y.max(1.0);
        let tan_free = tan_v * (view.x - left).max(view.x * 0.3) / view.y.max(1.0);
        // Distance at which every corner is inside the free part (from its middle).
        let mut distance: f32 = 1.0;
        for i in 0..8 {
            let corner = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
            let d = corner - center;
            let ahead = d.dot(forward);
            distance = distance.max(d.dot(right).abs() / tan_free - ahead).max(d.dot(up).abs() / tan_v - ahead);
        }
        cam.distance = distance * 1.08;
        // Move the picture to the middle of the free part: the free part's
        // middle is `left / width` of the way right in normalized screen x.
        cam.target -= right * (left / view.x.max(1.0)) * cam.distance * tan_full;
        cam
    }

    pub fn transform(&self) -> Transform {
        let rot = Quat::from_euler(EulerRot::YXZ, self.yaw, -self.pitch, 0.0);
        let pos = self.target + rot * Vec3::new(0.0, 0.0, self.distance);
        Transform::from_translation(pos).looking_at(self.target, Vec3::Y)
    }
}

pub fn orbit_camera(
    mut q: Query<(&mut OrbitCamera, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    ui_rects: Res<UiRects>,
    window: Query<&Window, With<PrimaryWindow>>,
    gizmo: Res<GizmoState>,
    (spacemouse, state, time): (Res<SpaceMouse>, Res<ShowState>, Res<Time>),
    (mut last_buttons, mut active, mut fit): (Local<u32>, Local<Option<bool>>, ResMut<FitRequest>),
    (fixtures, occluders, placeholder): (Query<&GlobalTransform, With<FixtureRoot>>, Res<Occluders>, Query<&ViewVisibility, With<Placeholder>>),
) {
    let Ok((mut cam, mut tf)) = q.single_mut() else { return };
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
        cam.pitch = (cam.pitch + a.rotation.x * 1.2 * k).clamp(-1.5, 1.5);

        let buttons = spacemouse.buttons();
        let pressed = buttons & !*last_buttons;
        if pressed & 3 != 0 {
            *fit = FitRequest { top: Some(pressed & 1 == 0), delay: 0 };
        }
        *last_buttons = buttons;
    }
    if let Some(top) = fit.top {
        if fit.delay > 0 {
            fit.delay -= 1;
        } else {
            let (lo, hi) = scene_bounds(&fixtures, &occluders, placeholder.iter().any(|v| v.get()));
            let view = window.single().map_or(Vec2::new(1600.0, 960.0), |w| w.size());
            // The sidebar: panels starting at the left edge.
            let left = ui_rects.0.iter().filter(|r| r.min.x < 1.0 && r.height() > view.y * 0.5).map(|r| r.max.x).fold(0.0, f32::max);
            *cam = OrbitCamera::framing(lo, hi, top, view, left);
            fit.top = None;
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
            cam.pitch = (cam.pitch + delta.y * 0.005).clamp(-1.5, 1.5);
        }
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if lines != 0.0 {
            cam.distance = (cam.distance * (1.0 - lines * 0.1)).clamp(0.5, 200.0);
        }
    }
    *tf = cam.transform();
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
