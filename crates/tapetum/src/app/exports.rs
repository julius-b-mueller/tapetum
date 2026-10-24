//! Exports from the current view: the fixture plan (PDF or SVG) and the
//! set-change drawings (PDF). The camera's view is the plan's: the standard
//! views line them up, orthographic ones come out to scale.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::camera::ScalingMode;
use bevy::prelude::*;

use super::gizmo::bk_to_world;
use super::objects::{SceneRuntime, StageObjects};
use super::orbit::OrbitCamera;
use super::state::ShowState;
use crate::export::drawing::{A3, Page};
use crate::export::linework::{self, Style, Surface};
use crate::export::plan::{FOOTER, Mapping, PlanFixture, PlanView, fixture_plan, footer};
use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    PlanPdf,
    PlanSvg,
    /// A page per scene: the set as a line drawing.
    SetChangesPdf,
}

/// An export asked for (from the panel at the bottom); with a path it skips
/// the file dialog (tests).
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

/// The camera's view for a plan, and the name of the view for the title block.
fn plan_view(state: &ShowState, camera: &Camera, gt: &GlobalTransform, projection: &Projection, what: &str) -> PlanView {
    let size = camera.logical_viewport_size().unwrap_or(Vec2::new(1600.0, 960.0));
    let ortho = match projection {
        Projection::Orthographic(o) => match o.scaling_mode {
            ScalingMode::FixedVertical { viewport_height } => Some(viewport_height / 2.0 * o.scale),
            _ => None,
        },
        _ => None,
    };
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
        clip_from_world: camera.clip_from_view() * gt.to_matrix().inverse(),
        aspect: size.x / size.y.max(1.0),
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
    for (mo, bk_from_model, billboard) in super::objects::placed(state, objs, scene, eye_bk) {
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

pub fn run_export(
    mut request: ResMut<ExportRequest>,
    done: Res<ExportDone>,
    (state, objs, scene_rt): (Res<ShowState>, Res<StageObjects>, Res<SceneRuntime>),
    camera: Query<(&Camera, &GlobalTransform, &Projection), With<OrbitCamera>>,
) {
    let Some(kind) = request.kind.take() else { return };
    let path = request.path.take();
    let Ok((cam, gt, projection)) = camera.single() else { return };
    let dir = state.show_path.parent().map(PathBuf::from);
    let line_cam = linework::Camera { view_from_world: gt.to_matrix().inverse(), orthographic: matches!(projection, Projection::Orthographic(_)) };
    let eye = gt.translation();
    match kind {
        ExportKind::PlanPdf | ExportKind::PlanSvg => {
            let what = tr("Scheinwerferplan", "Fixture plan");
            let view = plan_view(&state, cam, gt, projection, what);
            let set = scene_surfaces(&state, &objs, scene_rt.active, eye);
            let page: Page = fixture_plan(&view, &plan_fixtures(&state), Some((&line_cam, &set)), tr("Kanäle", "ch"));
            if kind == ExportKind::PlanPdf {
                save(&done, path, dir, proposal(&state, what, "pdf"), "pdf", crate::export::pdf::write(&[page], &view.title));
            } else {
                save(&done, path, dir, proposal(&state, what, "svg"), "svg", crate::export::svg::write(&page, &view.title).into_bytes());
            }
        }
        ExportKind::SetChangesPdf => {
            let what = tr("Umbauplan", "Set changes");
            let view = plan_view(&state, cam, gt, projection, what);
            let scenes: Vec<(String, Vec<Surface>)> = scenes(&state).into_iter().map(|(id, name)| (name, scene_surfaces(&state, &objs, id, eye))).collect();
            // One frame for every page, so they lie on top of each other.
            let probe = Mapping::probe(&view);
            let points: Vec<Vec2> = scenes.iter().flat_map(|(_, s)| linework::picture_points(&probe, s)).collect();
            let m = Mapping::fit(&view, &points, A3, FOOTER);
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
