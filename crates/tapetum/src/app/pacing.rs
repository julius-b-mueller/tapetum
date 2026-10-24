//! Draft saves work: at most 30 frames a second while something changes, and
//! only a few while nothing does. Art-Net arrives on its own thread and wakes
//! nothing, so even then the app looks a few times a second.

use std::time::Duration;

use bevy::prelude::*;
use bevy::winit::{UpdateMode, WinitSettings};
use tapetum_project::Quality;

use super::fixtures::{Head, Yoke};
use super::objects::StageObjectRoot;
use super::orbit::OrbitCamera;
use super::state::ShowState;

/// Something visible changed this frame (set where it happens, cleared here).
#[derive(Resource, Default)]
pub struct Activity {
    pub changed: bool,
}

/// While changing: 30 frames a second; idle: 4.
const BUSY: Duration = Duration::from_micros(33_333);
const IDLE: Duration = Duration::from_millis(250);
/// Stays busy this long after the last change (fades end smoothly).
const AFTERGLOW: f32 = 0.6;

#[allow(clippy::type_complexity)]
pub fn pace(
    state: Res<ShowState>,
    mut activity: ResMut<Activity>,
    mut winit: ResMut<WinitSettings>,
    time: Res<Time<Real>>,
    moved: Query<(), (Or<(With<Yoke>, With<Head>, With<StageObjectRoot>)>, Changed<Transform>)>,
    camera: Query<&OrbitCamera>,
    (mut last_camera, mut busy_until, mut was_draft): (Local<Option<OrbitCamera>>, Local<f32>, Local<bool>),
) {
    let draft = state.project.render.quality == Quality::Draft;
    if !draft {
        if *was_draft {
            *winit = WinitSettings::default();
            *was_draft = false;
        }
        activity.changed = false;
        return;
    }
    *was_draft = true;
    let cam = camera.single().ok().copied();
    let changed = std::mem::take(&mut activity.changed) || !moved.is_empty() || cam != *last_camera;
    *last_camera = cam;
    let now = time.elapsed_secs();
    if changed {
        *busy_until = now + AFTERGLOW;
    }
    let wait = if now < *busy_until { BUSY } else { IDLE };
    // Window events (the mouse over Tapetum) wake it anyway; not every move elsewhere.
    let mode = UpdateMode::reactive_low_power(wait);
    if winit.focused_mode != mode {
        *winit = WinitSettings { focused_mode: mode, unfocused_mode: mode };
    }
}
