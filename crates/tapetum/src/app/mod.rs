//! The 3D visualizer window.

mod about;
pub use about::open_at_start as open_about_at_start;
pub use ui::hide_sidebar_at_start;
mod beam;
mod fixtures;
mod gizmo;
pub mod gobo;
pub mod layout;
mod motion;
pub mod objects;
mod orbit;
mod scene;
mod spacemouse;
pub use spacemouse::diagnose as spacemouse_diagnose;
mod state;
mod type_editor;
mod ui;
mod undo;

use std::net::Ipv4Addr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass};
use tapetum_project::Project;

use state::{BkLink, BkStatus, Dmx, GoboImport, ObjectImport, PlacementSync, RangeSync, ShowState, Watch};

#[derive(Resource)]
struct Offscreen(Handle<Image>);

fn render_offscreen(mut commands: Commands, mut images: ResMut<Assets<Image>>, cameras: Query<Entity, With<scene::UiCamera>>) {
    let image = Image::new_target_texture(1600, 960, TextureFormat::Rgba8UnormSrgb, None);
    let handle = images.add(image);
    for cam in &cameras {
        commands.entity(cam).insert(RenderTarget::Image(handle.clone().into()));
    }
    commands.insert_resource(Offscreen(handle));
}

/// Handles draw on top of the fixtures, a little thicker than hairlines.
fn setup_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.depth_bias = -1.0;
    config.line.width = 3.0;
}

pub struct Options {
    pub show: PathBuf,
    pub bind: Ipv4Addr,
    /// Overrides the project's Art-Net port.
    pub port: Option<u16>,
    /// Save a screenshot after this many frames and quit (for testing).
    pub screenshot: Option<(PathBuf, u32)>,
    pub select: Option<i64>,
    pub select_object: Option<String>,
    pub scene: Option<u64>,
    pub bench: bool,
    pub orbit: Option<[f32; 4]>,
    /// Frame everything at start: `Some(true)` from above.
    pub fit: Option<bool>,
}

/// How long a taken Art-Net port may take to become free at start: the Tapetum
/// that just restarted into this one is still quitting.
pub const PORT_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

pub fn run(opts: Options) -> Result<()> {
    let show = bk_olga::load(&opts.show)?;
    let project_path = Project::path_for_show(&opts.show);
    let project = Project::load_or_default(&project_path)?;
    // A port given on the command line is taken as it is; the project's, if
    // another program holds it, gives way to the next free one (the sidebar
    // then offers to move the Tapetum interfaces in BK there).
    let port = opts.port.unwrap_or_else(|| bk_artnet::free_port_after(opts.bind, project.artnet.listen_port, PORT_GRACE));
    let receiver = Context::with_context(bk_artnet::Receiver::start(opts.bind, port), || {
        trf!("Art-Net-Port {}:{port} ist belegt", "Art-Net port {}:{port} is in use", opts.bind, port = port)
    })?;
    let watch = Watch::new(&opts.show).ok();
    let bk_status = BkStatus::start(project.artnet.bk_remote_port);
    let selected = opts.select.and_then(|id| show.fixtures.iter().position(|f| f.id == id));

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Tapetum".into(),
            resolution: WindowResolution::new(1600, 960),
            present_mode: if opts.bench { PresentMode::AutoNoVsync } else { PresentMode::AutoVsync },
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin::default())
    .add_plugins(beam::BeamPlugin)
    // The UI gets its own camera (see `scene::UiCamera`), not the 3D one.
    .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() })
    .insert_resource(ShowState {
        show_path: opts.show,
        show,
        project,
        project_path,
        generation: 0,
        specs: Vec::new(),
        selected,
        selection: selected.into_iter().collect(),
        selection_group: None,
        selected_objects: opts.select_object.iter().cloned().collect(),
        selected_object: opts.select_object,
        anchor_fixture: None,
        anchor_group: None,
        anchor_object: None,
        manual_scene: opts.scene,
        led_brush: 0,
        status: None,
    })
    .insert_resource(Dmx { receiver, bind: opts.bind })
    .insert_resource(bk_status)
    .init_resource::<BkLink>()
    .init_resource::<ui::UiRects>()
    .init_resource::<RangeSync>()
    .init_resource::<undo::UndoHistory>()
    .insert_resource(orbit::FitRequest { top: opts.fit, delay: 120 })
    // Test runs (screenshots, benchmarks) shouldn't follow a hand on the cap.
    .insert_resource(if opts.screenshot.is_some() || opts.bench { spacemouse::SpaceMouse::default() } else { spacemouse::SpaceMouse::start() })
    .init_resource::<PlacementSync>()
    .init_resource::<objects::StageObjects>()
    .init_resource::<objects::SceneRuntime>()
    .init_resource::<objects::Occluders>()
    .init_resource::<gizmo::GizmoState>()
    .init_resource::<GoboImport>()
    .init_resource::<ObjectImport>()
    .add_systems(Startup, (fixtures::setup_assets, scene::setup_scene, setup_gizmos))
    .add_systems(
        Update,
        (
            state::poll_reload,
            fixtures::rebuild_fixtures,
            gizmo::apply_placements,
            objects::sync_objects,
            objects::select_scene,
            objects::apply_scene,
            objects::draw_selection,
            fixtures::update_fixtures,
            objects::sync_surface_masks,
            scene::apply_render_settings,
            scene::place_house_lights,
            scene::set_window_icon,
            scene::fit_view,
            gizmo::gizmo,
            orbit::orbit_camera,
            undo::track_undo,
            state::sync_ranges,
            state::sync_placements,
            state::live_groups,
        )
            .chain(),
    )
    .add_systems(EguiPrimaryContextPass, ui::panel);
    if let Some(w) = watch {
        app.insert_resource(w);
    }
    if let Some([yaw, pitch, distance, height]) = opts.orbit {
        app.add_systems(PostStartup, move |mut q: Query<&mut orbit::OrbitCamera>| {
            for mut c in &mut q {
                (c.yaw, c.pitch, c.distance, c.target.y) = (yaw.to_radians(), pitch.to_radians(), distance, height);
            }
        });
    }
    if opts.bench {
        // Never wait for the display or for focus.
        app.insert_resource(bevy::winit::WinitSettings { focused_mode: bevy::winit::UpdateMode::Continuous, unfocused_mode: bevy::winit::UpdateMode::Continuous });
        // Offscreen like screenshots, so an occluded window doesn't throttle it.
        app.add_systems(PostStartup, render_offscreen);
        app.add_systems(Update, |mut n: Local<u32>, mut start: Local<Option<std::time::Instant>>, mut exit: MessageWriter<AppExit>| {
            *n += 1;
            if *n == 300 {
                *start = Some(std::time::Instant::now());
            }
            if *n == 600 {
                let ms = start.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0 / 300.0);
                println!("BENCH {ms:.2} ms/frame ({:.0} fps)", 1000.0 / ms.max(1e-6));
                exit.write(AppExit::Success);
            }
        });
    }
    if let Some((path, frames)) = opts.screenshot {
        // Never wait for focus: unfocused, the app only updates on events and
        // would neither take the picture nor quit afterwards.
        app.insert_resource(bevy::winit::WinitSettings { focused_mode: bevy::winit::UpdateMode::Continuous, unfocused_mode: bevy::winit::UpdateMode::Continuous });
        // Render offscreen: macOS stops presenting occluded windows, which
        // would make window screenshots black when the app is in the background.
        app.add_systems(PostStartup, render_offscreen);
        app.add_systems(Update, move |mut commands: Commands, mut n: Local<u32>, target: Res<Offscreen>, mut exit: MessageWriter<AppExit>| {
            *n += 1;
            if *n == frames {
                commands.spawn(Screenshot(RenderTarget::Image(target.0.clone().into()))).observe(save_to_disk(path.clone()));
            }
            // Saving happens a few frames later; wait for the file (with a limit).
            if *n > frames && (path.exists() || *n > frames + 600) {
                exit.write(AppExit::Success);
            }
        });
    }
    app.run();
    Ok(())
}
