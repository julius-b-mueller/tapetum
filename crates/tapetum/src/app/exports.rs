//! Exports from the current view: the fixture plan (PDF or SVG), the
//! set-change drawings (PDF) and a screenshot (PNG). From a standard view the
//! plans are framed anew for the page, whatever the window's shape; after
//! moving the camera by hand they take what shows between the side panels.
//! Orthographic views come out to scale.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::camera::{CameraProjection, RenderTarget, ScalingMode};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;
use bevy_egui::egui;
use tapetum_project::Quality;

use super::fixtures::FixtureRoot;
use super::gizmo::bk_to_world;
use super::objects::{FrameBoxes, SceneRuntime, StageObjects};
use super::orbit::{OrbitCamera, frame_bounds, tan_half_fov};
use super::scene::View3d;
use super::state::ShowState;
use super::ui::UiRects;
use super::undo::UndoHistory;
use crate::export::drawing::{A3, Page};
use crate::export::linework::{self, Style, Surface};
use crate::export::plan::{FOOTER, Mapping, PlanFixture, PlanView, drawing_aspect, fixture_plan, footer};
use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    PlanPdf,
    PlanSvg,
    /// A page per scene: the set as a line drawing.
    SetChangesPdf,
    /// The 3D view at the highest quality.
    ScreenshotPng,
}

/// The export section of the sidebar; a click sets `export`.
pub fn section(ui: &mut egui::Ui, project: &mut tapetum_project::Project, export: &mut Option<ExportKind>) {
    egui::CollapsingHeader::new(egui::RichText::new(tr("💾 Export", "💾 Export")).strong()).id_salt("export").show(ui, |ui| {
        ui.colored_label(
            egui::Color32::GRAY,
            tr(
                "Aus der aktuellen Ansicht. Die Standardansichten (unten) richten Pläne gerade aus und rahmen sie fürs Blatt; orthografisch sind sie maßstäblich.",
                "From the current view. The standard views (at the bottom) line plans up and frame them for the page; orthographic ones are to scale.",
            ),
        );
        let mut button = |ui: &mut egui::Ui, kind: ExportKind, label: &str, tip: &str| {
            if ui.button(label).on_hover_text(tip).clicked() {
                *export = Some(kind);
            }
        };
        let plan_tip = tr(
            "Jede Lampe als Symbol mit Typ, Universum/Adresse und Kanalzahl, bei Moving Heads die Hängerichtung; das Bühnenbild blass darunter.",
            "Every fixture as a symbol with type, universe/address and channel count, for moving heads the hanging direction; the set faint underneath.",
        );
        button(ui, ExportKind::PlanPdf, tr("Scheinwerferplan als PDF …", "Fixture plan as PDF …"), plan_tip);
        button(ui, ExportKind::PlanSvg, tr("Scheinwerferplan als SVG …", "Fixture plan as SVG …"), plan_tip);
        button(
            ui,
            ExportKind::SetChangesPdf,
            tr("Umbaupläne als PDF …", "Set changes as PDF …"),
            tr("Je Szene eine Seite mit dem Bühnenbild als Linienzeichnung, alle im selben Ausschnitt. Figuren behalten ihr Bild.", "A page per scene with the set as a line drawing, all in the same frame. Figures keep their picture."),
        );
        let mut dashed = !project.view.omit_hidden;
        if ui.checkbox(&mut dashed, tr("Verdeckte Linien gestrichelt", "Hidden lines dashed")).on_hover_text(tr("Sonst fehlen sie.", "Otherwise they are left out.")).changed() {
            project.view.omit_hidden = !dashed;
        }
        ui.colored_label(egui::Color32::GRAY, tr("Was in Plänen erscheint: Druckersymbol bei den Bühnenobjekten.", "What shows in plans: the printer symbol at the stage objects."));
        button(
            ui,
            ExportKind::ScreenshotPng,
            tr("Screenshot als PNG …", "Screenshot as PNG …"),
            tr("Die 3D-Ansicht zwischen den Seitenleisten, kurz in der höchsten Qualität gerendert.", "The 3D view between the side panels, rendered at the highest quality for a moment."),
        );
    });
}

/// An export asked for (from the sidebar); with a path it skips the file
/// dialog (tests).
#[derive(Resource, Default)]
pub struct ExportRequest {
    pub kind: Option<ExportKind>,
    pub path: Option<PathBuf>,
}

/// The result of the last export, for the status line: the file, or what went wrong.
#[derive(Resource, Default, Clone)]
pub struct ExportDone(pub Arc<Mutex<Option<Result<PathBuf, String>>>>);

/// Today as `2026-10-24` (UTC).
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Civil date from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The camera a plan is drawn with.
struct ExportCamera {
    clip_from_world: Mat4,
    view_from_world: Mat4,
    aspect: f32,
    /// Orthographic: meters from the middle to the top edge of the picture.
    ortho_half_height: Option<f32>,
    eye: Vec3,
    /// What of the picture counts (picture coordinates); `None`: all of it.
    frame: Option<(Vec2, Vec2)>,
}

impl ExportCamera {
    fn lines(&self) -> linework::Camera {
        linework::Camera { view_from_world: self.view_from_world, orthographic: self.ortho_half_height.is_some() }
    }
}

/// The side panels' inner edges (window pixels): what the 3D view shows between them.
fn free_area(ui_rects: &UiRects, window: Vec2) -> (f32, f32) {
    let tall = |r: &&egui::Rect| r.height() > window.y * 0.5;
    let left = ui_rects.0.iter().filter(tall).filter(|r| r.min.x < 1.0).map(|r| r.max.x).fold(0.0, f32::max);
    let right = ui_rects.0.iter().filter(tall).filter(|r| r.max.x > window.x - 1.0).map(|r| r.min.x).fold(window.x, f32::min);
    if right - left > window.x * 0.2 { (left, right) } else { (0.0, window.x) }
}

/// From a standard view: the same view framed anew for a page (the window's
/// shape and panels don't matter). Moved by hand: the camera as it is, the
/// part between the side panels.
fn export_camera(state: &ShowState, live: (&Camera, &GlobalTransform, &Projection), bounds: (Vec3, Vec3), panels: (f32, f32), window: Vec2) -> ExportCamera {
    let (camera, gt, projection) = live;
    let orthographic = state.project.view.orthographic;
    if let Some(standard) = state.project.view.standard {
        let aspect = drawing_aspect(A3);
        let cam = OrbitCamera::framing(bounds.0, bounds.1, standard, orthographic, Vec2::new(aspect * 1000.0, 1000.0), 0.0);
        let tf = if orthographic { cam.transform_ortho() } else { cam.transform() };
        let view_from_world = tf.to_matrix().inverse();
        let half = cam.distance * tan_half_fov();
        let clip_from_view = if orthographic {
            Mat4::from_diagonal(Vec4::new(1.0 / (half * aspect), 1.0 / half, -0.001, 1.0))
        } else {
            PerspectiveProjection { aspect_ratio: aspect, ..default() }.get_clip_from_view()
        };
        return ExportCamera { clip_from_world: clip_from_view * view_from_world, view_from_world, aspect, ortho_half_height: orthographic.then_some(half), eye: tf.translation, frame: None };
    }
    let size = camera.logical_viewport_size().unwrap_or(window);
    let aspect = size.x / size.y.max(1.0);
    let ortho_half_height = match projection {
        Projection::Orthographic(o) => match o.scaling_mode {
            ScalingMode::FixedVertical { viewport_height } => Some(viewport_height / 2.0 * o.scale),
            _ => None,
        },
        _ => None,
    };
    let x = |px: f32| (2.0 * px / window.x.max(1.0) - 1.0) * aspect;
    ExportCamera {
        clip_from_world: camera.clip_from_view() * gt.to_matrix().inverse(),
        view_from_world: gt.to_matrix().inverse(),
        aspect,
        ortho_half_height,
        eye: gt.translation(),
        frame: Some((Vec2::new(x(panels.0), -1.0), Vec2::new(x(panels.1), 1.0))),
    }
}

/// The view for a plan, and the name of the view for the title block.
fn plan_view(state: &ShowState, ec: &ExportCamera, what: &str) -> PlanView {
    let ortho = ec.ortho_half_height;
    let view_name = state.project.view.standard.map(super::overlays::view_label);
    let mut subtitle = what.to_string();
    if let Some(v) = view_name {
        subtitle.push_str(&trf!(" · Ansicht {}", " · view {}", v));
    }
    if ortho.is_some() {
        subtitle.push_str(tr(" · orthografisch", " · orthographic"));
    }
    let show = state.show_path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    PlanView {
        clip_from_world: ec.clip_from_world,
        aspect: ec.aspect,
        ortho_half_height: ortho,
        title: show,
        subtitle,
        date: today(),
    }
}

/// Every fixture as the plan shows it.
pub fn plan_fixtures(state: &ShowState) -> Vec<PlanFixture> {
    let placements = super::layout::placements(&state.show, &state.project);
    let show = &state.show;
    show.fixtures
        .iter()
        .zip(&placements)
        .enumerate()
        .map(|(i, (f, placement))| {
            let ty = show.fixture_type(f);
            let spec = state.specs.get(i).and_then(|s| s.as_ref());
            let class = spec.map_or_else(|| ty.map_or(bk_olga::FixtureClass::Unknown, |t| t.classify()), |s| s.class);
            let w = bk_olga::placement::world_from_local(placement.rotation);
            let world = |v: [f32; 3]| bk_to_world(bk_olga::placement::mul(&w, v)).normalize_or_zero();
            let forward = match (class, spec) {
                // Level at the middle of its pan range, like the arrow in the 3D view.
                (bk_olga::FixtureClass::MovingHead, Some(s)) => world(bk_olga::placement::beam_local((s.pan_range[0] + s.pan_range[1]) / 2.0, -90.0)),
                _ => world(bk_olga::placement::beam_local(0.0, 0.0)),
            };
            PlanFixture {
                id: f.id,
                name: f.name.clone(),
                type_name: ty.map_or_else(|| "?".into(), |t| t.name.clone()),
                patch: crate::routing::primary_patch(show, &state.project.artnet, &f.patches).map(|(p, pa)| (crate::routing::format_port_address(pa), p.address)),
                channels: ty.map_or(0, |t| t.footprint()),
                class,
                position: bk_to_world(placement.position),
                forward,
                across: world([1.0, 0.0, 0.0]),
            }
        })
        .collect()
}

/// The set of a scene as surfaces in the world (Bevy), seen from `eye` (world).
pub fn scene_surfaces(state: &ShowState, objs: &StageObjects, scene: Option<u64>, eye: Vec3) -> Vec<Surface> {
    // BK is Z-up; the world turns it -90° around X (BK +Z = Bevy +Y).
    let world_from_bk = Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let eye_bk = Vec3::new(eye.x, -eye.z, eye.y);
    let mut out = Vec::new();
    // Only what is printed: the line drawings honour the printer switch.
    for (mo, bk_from_model, billboard) in super::objects::placed(state, objs, scene, eye_bk, true) {
        let m = world_from_bk * bk_from_model;
        for (i, part) in mo.parts.iter().enumerate() {
            let texture = part.texture.clone().filter(|t| t.cutout || billboard);
            out.push(Surface {
                positions: part.positions.iter().map(|p| m.transform_point3(*p)).collect(),
                indices: part.indices.clone(),
                uvs: if texture.is_some() { part.uvs.clone() } else { Vec::new() },
                texture,
                // The object's outline goes with its first part.
                edges: if i == 0 { mo.edges.iter().map(|[a, b]| [m.transform_point3(*a), m.transform_point3(*b)]).collect() } else { Vec::new() },
                figure: billboard.then(|| mo.name.clone()),
            });
        }
    }
    out
}

/// The scenes in the order of their cues, the base scene first: (scene ID,
/// what the title block says about it).
fn scenes(state: &ShowState) -> Vec<(Option<u64>, String)> {
    let cue = |id: f64| state.show.conductor().and_then(|c| c.cues.iter().find(|q| q.id == id));
    let mut list = vec![(None, tr("Grundszene", "Base scene").to_string())];
    for i in super::overlays::scene_order(&state.project) {
        let sc = &state.project.scenes[i];
        let mut what = trf!("Szene „{}“", "Scene “{}”", sc.name);
        if let Some(id) = sc.cue_id {
            what.push_str(&trf!(" · Cue {}", " · cue {}", id));
            if let Some(q) = cue(id) {
                if !q.name.is_empty() {
                    what.push_str(&format!(" {}", q.name));
                }
                if !q.chapter.is_empty() {
                    what.push_str(&format!(" · {}", q.chapter));
                }
            }
        }
        list.push((Some(sc.id), what));
    }
    list
}

/// A file name proposal next to the show.
fn proposal(state: &ShowState, what: &str, ext: &str) -> String {
    let show = state.show_path.file_stem().map_or_else(|| "Tapetum".into(), |s| s.to_string_lossy().into_owned());
    format!("{show} – {what}.{ext}")
}

/// Asks for a file (unless given one) and writes the bytes there, off the main thread.
fn save(done: &ExportDone, path: Option<PathBuf>, dir: Option<PathBuf>, name: String, ext: &'static str, bytes: Vec<u8>) {
    let slot = done.0.clone();
    std::thread::spawn(move || {
        let path = path.or_else(|| {
            let mut dialog = rfd::FileDialog::new().set_file_name(&name).add_filter(ext.to_uppercase(), &[ext]);
            if let Some(d) = dir {
                dialog = dialog.set_directory(d);
            }
            dialog.save_file()
        });
        let Some(path) = path else { return };
        let result = std::fs::write(&path, bytes).map(|()| path).map_err(|e| e.to_string());
        if let Ok(mut s) = slot.lock() {
            *s = Some(result);
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn run_export(
    mut request: ResMut<ExportRequest>,
    done: Res<ExportDone>,
    (state, objs, scene_rt): (Res<ShowState>, Res<StageObjects>, Res<SceneRuntime>),
    camera: Query<(&Camera, &GlobalTransform, &Projection), With<OrbitCamera>>,
    (fixtures, frames, placeholder): (Query<&GlobalTransform, With<FixtureRoot>>, Res<FrameBoxes>, Query<&ViewVisibility, With<super::objects::Placeholder>>),
    (ui_rects, window): (Res<UiRects>, Query<&Window, With<PrimaryWindow>>),
    mut shot: ResMut<ScreenshotJob>,
) {
    let Some(kind) = request.kind.take() else { return };
    let path = request.path.take();
    let dir = state.show_path.parent().map(PathBuf::from);
    if kind == ExportKind::ScreenshotPng {
        shot.start(path, dir, proposal(&state, tr("Ansicht", "View"), "png"));
        return;
    }
    let Ok(live) = camera.single() else { return };
    let window = window.single().map_or(Vec2::new(1600.0, 960.0), |w| w.size());
    let bounds = frame_bounds(&fixtures, &frames, placeholder.iter().any(|v| v.get()));
    let ec = export_camera(&state, live, bounds, free_area(&ui_rects, window), window);
    let line_cam = ec.lines();
    match kind {
        ExportKind::PlanPdf | ExportKind::PlanSvg => {
            let what = tr("Scheinwerferplan", "Fixture plan");
            let view = plan_view(&state, &ec, what);
            let set = scene_surfaces(&state, &objs, scene_rt.active, ec.eye);
            let page: Page = fixture_plan(&view, &plan_fixtures(&state), Some((&line_cam, &set)), tr("Kanäle", "ch"), ec.frame);
            if kind == ExportKind::PlanPdf {
                save(&done, path, dir, proposal(&state, what, "pdf"), "pdf", crate::export::pdf::write(&[page], &view.title));
            } else {
                save(&done, path, dir, proposal(&state, what, "svg"), "svg", crate::export::svg::write(&page, &view.title).into_bytes());
            }
        }
        ExportKind::SetChangesPdf => {
            let what = tr("Umbauplan", "Set changes");
            let view = plan_view(&state, &ec, what);
            let scenes: Vec<(String, Vec<Surface>)> = scenes(&state).into_iter().map(|(id, name)| (name, scene_surfaces(&state, &objs, id, ec.eye))).collect();
            // One frame for every page, so they lie on top of each other.
            let probe = Mapping::probe(&view);
            let points: Vec<Vec2> = scenes.iter().flat_map(|(_, s)| linework::picture_points(&probe, s)).collect();
            let m = Mapping::fit(&view, &points, A3, FOOTER, ec.frame);
            let pages: Vec<Page> = scenes
                .iter()
                .map(|(name, surfaces)| {
                    let mut page = Page::new(A3);
                    linework::draw(&mut page, &m, &line_cam, surfaces, Style::drawing(!state.project.view.omit_hidden));
                    let page_view = PlanView { subtitle: format!("{} · {name}", view.subtitle), ..view.clone() };
                    footer(&mut page, &page_view, m.ratio);
                    page
                })
                .collect();
            save(&done, path, dir, proposal(&state, what, "pdf"), "pdf", crate::export::pdf::write(&pages, &view.title));
        }
        ExportKind::ScreenshotPng => {}
    }
}

/// Frames the High quality gets to settle before the picture (fixtures
/// rebuilt, shadows, TAA).
const SETTLE_FRAMES: u32 = 60;

#[derive(Default)]
enum ShotStage {
    #[default]
    Idle,
    /// Waiting for the file dialog.
    Picking,
    /// At High quality for a while; the quality to go back to.
    Settling { frames: u32, back: Option<Quality>, path: PathBuf },
    Capturing { back: Option<Quality> },
}

/// A screenshot on its way: file, High quality, picture, quality back.
#[derive(Resource, Default)]
pub struct ScreenshotJob {
    stage: ShotStage,
    picked: Arc<Mutex<Option<Option<PathBuf>>>>,
    captured: Arc<AtomicBool>,
}

impl ScreenshotJob {
    fn start(&mut self, path: Option<PathBuf>, dir: Option<PathBuf>, name: String) {
        if !matches!(self.stage, ShotStage::Idle) {
            return;
        }
        let picked = self.picked.clone();
        match path {
            Some(p) => *picked.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(p)),
            None => {
                std::thread::spawn(move || {
                    let mut dialog = rfd::FileDialog::new().set_file_name(&name).add_filter("PNG", &["png"]);
                    if let Some(d) = dir {
                        dialog = dialog.set_directory(d);
                    }
                    let chosen = dialog.save_file();
                    if let Ok(mut slot) = picked.lock() {
                        *slot = Some(chosen);
                    }
                });
            }
        }
        self.stage = ShotStage::Picking;
    }
}

/// Switches the project's quality without an undo step, rebuilding the fixtures.
fn set_quality(state: &mut ShowState, undo: &mut UndoHistory, q: Quality) {
    let before = state.project.clone();
    state.project.render.quality = q;
    state.generation += 1;
    undo.external(&before, &state.project);
}

#[allow(clippy::too_many_arguments)]
pub fn run_screenshot(
    mut job: ResMut<ScreenshotJob>,
    mut state: ResMut<ShowState>,
    mut undo: ResMut<UndoHistory>,
    done: Res<ExportDone>,
    view: Res<View3d>,
    ui_rects: Res<UiRects>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    let job = &mut *job;
    match std::mem::take(&mut job.stage) {
        ShotStage::Idle => {}
        ShotStage::Picking => match job.picked.lock().ok().and_then(|mut p| p.take()) {
            None => job.stage = ShotStage::Picking,
            // Cancelled.
            Some(None) => {}
            Some(Some(path)) => {
                let back = (state.project.render.quality != Quality::High).then_some(state.project.render.quality);
                if back.is_some() {
                    set_quality(&mut state, &mut undo, Quality::High);
                }
                job.stage = ShotStage::Settling { frames: 0, back, path };
            }
        },
        ShotStage::Settling { frames, back, path } if frames < SETTLE_FRAMES => job.stage = ShotStage::Settling { frames: frames + 1, back, path },
        ShotStage::Settling { back, path, .. } => {
            let size = window.single().map_or(Vec2::new(1600.0, 960.0), |w| w.size());
            let (left, right) = free_area(&ui_rects, size);
            let (from, to) = (left / size.x.max(1.0), right / size.x.max(1.0));
            let (captured, slot) = (job.captured.clone(), done.0.clone());
            captured.store(false, Ordering::Relaxed);
            commands.spawn(Screenshot(RenderTarget::Image(view.image.clone().into()))).observe(move |shot: On<ScreenshotCaptured>| {
                let image = shot.image.clone();
                let (path, captured, slot) = (path.clone(), captured.clone(), slot.clone());
                std::thread::spawn(move || {
                    let result = image
                        .try_into_dynamic()
                        .map_err(|e| e.to_string())
                        .and_then(|img| {
                            let img = img.to_rgba8();
                            let (w, h) = img.dimensions();
                            let (x0, x1) = ((from * w as f32) as u32, ((to * w as f32) as u32).min(w));
                            image::imageops::crop_imm(&img, x0, 0, x1.saturating_sub(x0).max(1), h).to_image().save(&path).map_err(|e| e.to_string())
                        })
                        .map(|()| path);
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(result);
                    }
                    captured.store(true, Ordering::Relaxed);
                });
            });
            job.stage = ShotStage::Capturing { back };
        }
        ShotStage::Capturing { back } => {
            if job.captured.load(Ordering::Relaxed) {
                if let Some(q) = back {
                    set_quality(&mut state, &mut undo, q);
                }
            } else {
                job.stage = ShotStage::Capturing { back };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn today_looks_like_a_date() {
        let d = super::today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20") && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-');
    }
}
