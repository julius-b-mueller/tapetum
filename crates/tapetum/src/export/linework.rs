//! Line drawings of the set for printing: the creases, borders and outlines
//! of every surface, the ones hidden behind something dashed or left out.
//! Hidden parts come from a depth buffer the triangles are drawn into on the
//! CPU (in page pixels), so any view and any scene works without the GPU.
//! Face-me figures (often the costume designs) stay pictures, cut where
//! something stands in front of them, with an outline; seen from above, where
//! they would be a line, they are a circle with their name.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::{IVec3, Mat4, Vec2, Vec3};

use super::drawing::{Anchor, Color, Item, Page, Stroke};
use super::plan::Mapping;
use crate::stage::Texture;

/// Triangles of one object in the world (Bevy), with its creases and borders.
pub struct Surface {
    pub positions: Vec<Vec3>,
    pub indices: Vec<u32>,
    /// Per position; empty without a texture.
    pub uvs: Vec<Vec2>,
    /// A cut-out picture: its transparent pixels are no surface.
    pub texture: Option<Arc<Texture>>,
    pub edges: Vec<[Vec3; 2]>,
    /// A face-me figure and its name: drawn as its picture.
    pub figure: Option<String>,
}

pub struct Camera {
    pub view_from_world: Mat4,
    pub orthographic: bool,
}

/// How the lines look: visible ones, hidden ones (`None`: left out), and
/// whether figures show as pictures.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub visible: Stroke,
    pub hidden: Option<Stroke>,
    pub figures: bool,
}

impl Style {
    /// Black lines, hidden ones dashed grey (or left out).
    pub fn drawing(dashed: bool) -> Style {
        Style { visible: Stroke::solid(Color::BLACK, 0.22), hidden: dashed.then(|| Stroke::dashed(Color::gray(0.45), 0.15, 1.0, 0.8)), figures: true }
    }

    /// Faint, for under a fixture plan.
    pub fn background() -> Style {
        Style { visible: Stroke::solid(Color::gray(0.62), 0.15), hidden: None, figures: false }
    }
}

/// Every surface's corners in the camera's picture (for fitting a page).
pub fn picture_points(m: &Mapping, surfaces: &[Surface]) -> Vec<Vec2> {
    surfaces.iter().flat_map(|s| s.positions.iter().filter_map(|p| m.picture(*p))).collect()
}

/// Depth buffer pixels per mm.
const PX: f32 = 5.0;
/// Figures' pictures, pixels per mm.
const FIGURE_PX: f32 = 8.0;

struct Depth {
    w: usize,
    h: usize,
    z: Vec<f32>,
}

impl Depth {
    fn at(&self, p: [f32; 2]) -> f32 {
        let (x, y) = ((p[0] * PX) as isize, (p[1] * PX) as isize);
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return f32::INFINITY;
        }
        self.z[y as usize * self.w + x as usize]
    }

    /// The nearest depth around a point (edges lie between pixels).
    fn near(&self, p: [f32; 2]) -> f32 {
        let d = 0.5 / PX;
        [[0.0, 0.0], [d, 0.0], [-d, 0.0], [0.0, d], [0.0, -d]].iter().map(|o| self.at([p[0] + o[0], p[1] + o[1]])).fold(f32::INFINITY, f32::min)
    }
}

/// A point on the page and its distance in front of the camera.
#[derive(Clone, Copy)]
struct Projected {
    page: Vec2,
    depth: f32,
}

fn project(m: &Mapping, cam: &Camera, p: Vec3) -> Option<Projected> {
    let depth = -(cam.view_from_world * p.extend(1.0)).z;
    if !cam.orthographic && depth <= 0.01 {
        return None;
    }
    Some(Projected { page: Vec2::from(m.page(p)?), depth })
}

/// Interpolates depth (and anything with it) correctly: linear in 1/depth in
/// perspective, linear orthographic.
fn depth_between(cam: &Camera, a: f32, b: f32, t: f32) -> f32 {
    if cam.orthographic { a + (b - a) * t } else { 1.0 / (1.0 / a + (1.0 / b - 1.0 / a) * t) }
}

fn alpha_at(tex: &Texture, uv: Vec2) -> u8 {
    let x = ((uv.x.rem_euclid(1.0)) * tex.width as f32) as u32 % tex.width.max(1);
    let y = ((uv.y.rem_euclid(1.0)) * tex.height as f32) as u32 % tex.height.max(1);
    tex.rgba.get(((y * tex.width + x) * 4 + 3) as usize).copied().unwrap_or(255)
}

fn rgba_at(tex: &Texture, uv: Vec2) -> [u8; 4] {
    let x = ((uv.x.rem_euclid(1.0)) * tex.width as f32) as u32 % tex.width.max(1);
    let y = ((uv.y.rem_euclid(1.0)) * tex.height as f32) as u32 % tex.height.max(1);
    let i = ((y * tex.width + x) * 4) as usize;
    tex.rgba.get(i..i + 4).map_or([0; 4], |s| [s[0], s[1], s[2], s[3]])
}

/// Calls `f(x, y, barycentric, depth)` for the pixels (at `px` per mm) a
/// triangle covers; barycentrics perspective-correct.
fn raster(cam: &Camera, v: [Projected; 3], px: f32, origin: Vec2, size: (usize, usize), mut f: impl FnMut(usize, usize, [f32; 3], f32)) {
    let p: Vec<Vec2> = v.iter().map(|q| (q.page - origin) * px).collect();
    let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
    if area.abs() < 1e-6 {
        return;
    }
    let (lo, hi) = (p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO), p[0].max(p[1]).max(p[2]).ceil().min(Vec2::new(size.0 as f32, size.1 as f32)));
    for y in lo.y as usize..hi.y as usize {
        for x in lo.x as usize..hi.x as usize {
            let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = (p[2] - p[1]).perp_dot(c - p[1]) / area;
            let w1 = (p[0] - p[2]).perp_dot(c - p[2]) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let (bary, depth) = if cam.orthographic {
                ([w0, w1, w2], w0 * v[0].depth + w1 * v[1].depth + w2 * v[2].depth)
            } else {
                let inv = [w0 / v[0].depth, w1 / v[1].depth, w2 / v[2].depth];
                let sum = inv[0] + inv[1] + inv[2];
                ([inv[0] / sum, inv[1] / sum, inv[2] / sum], 1.0 / sum)
            };
            f(x, y, bary, depth);
        }
    }
}

fn triangles(s: &Surface) -> impl Iterator<Item = [usize; 3]> + '_ {
    s.indices.as_chunks::<3>().0.iter().map(|t| t.map(|i| i as usize))
}

/// The depth buffer of everything (figures by their picture's shape).
fn depth_buffer(page: [f32; 2], m: &Mapping, cam: &Camera, surfaces: &[Surface]) -> Depth {
    let (w, h) = ((page[0] * PX) as usize, (page[1] * PX) as usize);
    let mut depth = Depth { w, h, z: vec![f32::INFINITY; w * h] };
    for s in surfaces {
        let projected: Vec<Option<Projected>> = s.positions.iter().map(|p| project(m, cam, *p)).collect();
        for t in triangles(s) {
            let (Some(a), Some(b), Some(c)) = (projected[t[0]], projected[t[1]], projected[t[2]]) else { continue };
            let uv = (!s.uvs.is_empty()).then(|| t.map(|i| s.uvs[i]));
            raster(cam, [a, b, c], PX, Vec2::ZERO, (w, h), |x, y, bary, z| {
                if let (Some(tex), Some(uv)) = (&s.texture, uv)
                    && alpha_at(tex, uv[0] * bary[0] + uv[1] * bary[1] + uv[2] * bary[2]) < 128
                {
                    return;
                }
                let cell = &mut depth.z[y * w + x];
                if z < *cell {
                    *cell = z;
                }
            });
        }
    }
    depth
}

/// How far something must be in front to hide a line (meters at that depth).
fn tolerance(depth: f32) -> f32 {
    0.03 + depth * 0.006
}

/// Edges where the surface turns away from the camera (outlines of round and
/// closed things), besides the creases and borders the model has.
fn silhouettes(s: &Surface, cam: &Camera) -> Vec<[Vec3; 2]> {
    let world_from_view = cam.view_from_world.inverse();
    let eye = world_from_view.transform_point3(Vec3::ZERO);
    let forward = world_from_view.transform_vector3(Vec3::NEG_Z);
    let q = |v: Vec3| (v * 1000.0).round().as_ivec3();
    let mut faces: HashMap<(IVec3, IVec3), (Vec3, Vec3, u8, u8)> = HashMap::new();
    for t in triangles(s) {
        let v = t.map(|i| s.positions[i]);
        let n = (v[1] - v[0]).cross(v[2] - v[0]);
        let to_eye = if cam.orthographic { -forward } else { eye - v[0] };
        let front = n.dot(to_eye) > 0.0;
        for (a, b) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
            let (ka, kb) = (q(a), q(b));
            let key = if (ka.x, ka.y, ka.z) <= (kb.x, kb.y, kb.z) { (ka, kb) } else { (kb, ka) };
            let e = faces.entry(key).or_insert((a, b, 0, 0));
            if front {
                e.2 += 1;
            } else {
                e.3 += 1;
            }
        }
    }
    faces.into_values().filter(|e| e.2 == 1 && e.3 == 1).map(|e| [e.0, e.1]).collect()
}

/// Splits an edge into visible and hidden runs (page points).
fn runs(m: &Mapping, cam: &Camera, depth: &Depth, e: [Vec3; 2]) -> Vec<(bool, Vec<[f32; 2]>)> {
    let (Some(a), Some(b)) = (project(m, cam, e[0]), project(m, cam, e[1])) else { return Vec::new() };
    let length = (b.page - a.page).length();
    if length < 0.05 {
        return Vec::new();
    }
    let steps = ((length / 0.25).ceil() as usize).clamp(1, 4000);
    let mut out: Vec<(bool, Vec<[f32; 2]>)> = Vec::new();
    let mut outside = true;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = a.page.lerp(b.page, t);
        // Beyond the camera's picture the line stops.
        if !m.contains(p.to_array()) {
            outside = true;
            continue;
        }
        let z = depth_between(cam, a.depth, b.depth, t);
        let visible = depth.near(p.to_array()) >= z - tolerance(z);
        if std::mem::take(&mut outside) {
            out.push((visible, vec![p.to_array(), p.to_array()]));
            continue;
        }
        match out.last_mut() {
            Some((v, pts)) if *v == visible => {
                // Straight runs need only their ends.
                if pts.len() >= 2 {
                    pts.pop();
                }
                pts.push(p.to_array());
            }
            _ => {
                let start = out.last().and_then(|(_, pts)| pts.last().copied()).unwrap_or(p.to_array());
                out.push((visible, vec![start, p.to_array()]));
            }
        }
    }
    out.retain(|(_, pts)| pts.len() >= 2 && (Vec2::from(pts[0]) - Vec2::from(pts[pts.len() - 1])).length() > 0.05);
    out
}

/// Douglas–Peucker: drops points closer than `tol` to the line through their neighbours.
fn simplify(points: &[Vec2], tol: f32) -> Vec<Vec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let dist = |p: Vec2| if (b - a).length() < 1e-6 { (p - a).length() } else { (b - a).perp_dot(p - a).abs() / (b - a).length() };
    let (i, d) = points.iter().enumerate().skip(1).take(points.len() - 2).map(|(i, p)| (i, dist(*p))).fold((0, 0.0), |m, x| if x.1 > m.1 { x } else { m });
    if d <= tol {
        return vec![a, b];
    }
    let mut left = simplify(&points[..=i], tol);
    left.pop();
    left.extend(simplify(&points[i..], tol));
    left
}

/// Outlines of the opaque pixels of a mask (marching squares), as closed loops
/// in pixel units.
fn outlines(mask: &[bool], w: usize, h: usize) -> Vec<Vec<Vec2>> {
    let at = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize];
    // Edges between an opaque pixel and a clear one, directed with the opaque side on the left.
    let mut next: HashMap<(isize, isize), (isize, isize)> = HashMap::new();
    for y in -1..=h as isize {
        for x in -1..=w as isize {
            if !at(x, y) {
                continue;
            }
            if !at(x, y - 1) {
                next.insert((x + 1, y), (x, y));
            }
            if !at(x, y + 1) {
                next.insert((x, y + 1), (x + 1, y + 1));
            }
            if !at(x - 1, y) {
                next.insert((x, y), (x, y + 1));
            }
            if !at(x + 1, y) {
                next.insert((x + 1, y + 1), (x + 1, y));
            }
        }
    }
    let mut loops = Vec::new();
    while let Some(&start) = next.keys().next() {
        let mut lp = vec![Vec2::new(start.0 as f32, start.1 as f32)];
        let mut at_point = start;
        while let Some(to) = next.remove(&at_point) {
            lp.push(Vec2::new(to.0 as f32, to.1 as f32));
            at_point = to;
            if to == start {
                break;
            }
        }
        if lp.len() > 4 {
            loops.push(lp);
        }
    }
    loops
}

/// The camera looks down more than ahead: face-me figures are edge-on.
fn from_above(cam: &Camera) -> bool {
    cam.view_from_world.inverse().transform_vector3(Vec3::NEG_Z).normalize_or_zero().y < -0.7
}

/// A figure from above: a circle where it stands and its name, beside the
/// circle where no other name is yet.
fn marker(page: &mut Page, m: &Mapping, cam: &Camera, s: &Surface, name: &str, names: &mut Vec<(Vec2, Vec2)>) {
    let foot = s.positions.iter().fold(Vec3::ZERO, |a, p| a + *p) / s.positions.len().max(1) as f32;
    let Some(c) = project(m, cam, foot).map(|c| c.page).filter(|c| m.contains(c.to_array())) else { return };
    page.circle(c.to_array(), 2.2, Some(Stroke::solid(Color::BLACK, 0.3)), Some(Color::WHITE));
    let size = 2.2;
    let w = super::drawing::text_width(name, size, false);
    // Below, above, right, left: the first free place.
    let places = [(Vec2::new(-w / 2.0, 3.0), Anchor::Center), (Vec2::new(-w / 2.0, -3.0 - size), Anchor::Center), (Vec2::new(3.0, -size / 2.0), Anchor::Left), (Vec2::new(-3.0 - w, -size / 2.0), Anchor::Right)];
    let rect = |o: Vec2| (c + o, c + o + Vec2::new(w, size));
    let free = |r: &(Vec2, Vec2)| !names.iter().any(|n| r.0.x < n.1.x && n.0.x < r.1.x && r.0.y < n.1.y && n.0.y < r.1.y);
    let (offset, anchor) = places.iter().copied().find(|(o, _)| free(&rect(*o))).unwrap_or(places[0]);
    let r = rect(offset);
    let at = match anchor {
        Anchor::Center => [c.x, r.1.y - 0.4],
        Anchor::Left => [r.0.x, r.1.y - 0.4],
        Anchor::Right => [r.1.x, r.1.y - 0.4],
    };
    page.text(at, size, name.to_string(), anchor, false);
    names.push(r);
}

/// A figure: its picture where nothing stands in front, and its outline.
fn figure(page: &mut Page, m: &Mapping, cam: &Camera, depth: &Depth, s: &Surface, name: &str, names: &mut Vec<(Vec2, Vec2)>) {
    let projected: Vec<Option<Projected>> = s.positions.iter().map(|p| project(m, cam, *p)).collect();
    let pts: Vec<Vec2> = projected.iter().flatten().map(|p| p.page).collect();
    if pts.is_empty() {
        return;
    }
    let (lo, hi) = pts.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let Some(tex) = &s.texture else { return };
    if from_above(cam) || (hi - lo).min_element() < 1.5 || s.uvs.is_empty() {
        marker(page, m, cam, s, name, names);
        return;
    }
    let (w, h) = (((hi.x - lo.x) * FIGURE_PX).ceil() as usize + 1, ((hi.y - lo.y) * FIGURE_PX).ceil() as usize + 1);
    if w * h > 4_000_000 {
        return;
    }
    let mut img = image::RgbaImage::new(w as u32, h as u32);
    let mut mask = vec![false; w * h];
    for t in triangles(s) {
        let (Some(a), Some(b), Some(c)) = (projected[t[0]], projected[t[1]], projected[t[2]]) else { continue };
        let uv = t.map(|i| s.uvs[i]);
        raster(cam, [a, b, c], FIGURE_PX, lo, (w, h), |x, y, bary, z| {
            let px = rgba_at(tex, uv[0] * bary[0] + uv[1] * bary[1] + uv[2] * bary[2]);
            let at = [lo.x + (x as f32 + 0.5) / FIGURE_PX, lo.y + (y as f32 + 0.5) / FIGURE_PX];
            // Hidden where something is in front of the figure (or outside the picture).
            if px[3] < 128 || !m.contains(at) || depth.near(at) < z - tolerance(z) {
                return;
            }
            img.put_pixel(x as u32, y as u32, image::Rgba(px));
            mask[y * w + x] = true;
        });
    }
    if !mask.iter().any(|m| *m) {
        return;
    }
    let corner = |x: f32, y: f32| [lo.x + x / FIGURE_PX, lo.y + y / FIGURE_PX];
    page.items.push(Item::Image { image: Arc::new(img), top_left: corner(0.0, 0.0), top_right: corner(w as f32, 0.0), bottom_left: corner(0.0, h as f32) });
    for lp in outlines(&mask, w, h) {
        let simple = simplify(&lp, 0.12 * FIGURE_PX);
        page.polyline(simple.iter().map(|p| corner(p.x, p.y)).collect(), true, Some(Stroke::solid(Color::BLACK, 0.2)), None);
    }
}

/// Draws the set on the page in a style.
pub fn draw(page: &mut Page, m: &Mapping, cam: &Camera, surfaces: &[Surface], style: Style) {
    let depth = depth_buffer(page.size, m, cam, surfaces);
    let mut names = Vec::new();
    for s in surfaces {
        if let Some(name) = &s.figure {
            if style.figures {
                figure(page, m, cam, &depth, s, name, &mut names);
            }
            continue;
        }
        let mut edges = s.edges.clone();
        edges.extend(silhouettes(s, cam));
        for e in edges {
            for (visible, pts) in runs(m, cam, &depth, e) {
                match (visible, style.hidden) {
                    (true, _) => page.polyline(pts, false, Some(style.visible), None),
                    (false, Some(h)) => page.polyline(pts, false, Some(h), None),
                    (false, None) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::drawing::A3;
    use crate::export::plan::PlanView;

    /// A cube from 0 to 1 (Bevy world), creases on every edge.
    fn cube(at: Vec3) -> Surface {
        let c = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y, Vec3::Z, Vec3::new(1.0, 0.0, 1.0), Vec3::ONE, Vec3::new(0.0, 1.0, 1.0)].map(|p| p + at);
        let quads = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [0, 4, 7, 3]];
        let indices = quads.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect();
        let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)].map(|(a, b)| [c[a], c[b]]).to_vec();
        Surface { positions: c.to_vec(), indices, uvs: Vec::new(), texture: None, edges, figure: None }
    }

    fn front_view() -> (PlanView, Camera) {
        let view_from_world = bevy::prelude::Transform::from_xyz(0.5, 0.5, 20.0).looking_at(Vec3::new(0.5, 0.5, 0.0), Vec3::Y).to_matrix().inverse();
        let proj = Mat4::from_diagonal(bevy::math::Vec4::new(1.0 / 4.0, 1.0 / 2.5, -0.005, 1.0));
        (
            PlanView { clip_from_world: proj * view_from_world, aspect: 1.6, ortho_half_height: Some(2.5), title: String::new(), subtitle: String::new(), date: String::new() },
            Camera { view_from_world, orthographic: true },
        )
    }

    fn lines(page: &Page) -> (usize, usize) {
        page.items.iter().fold((0, 0), |(v, h), i| match i {
            Item::Path { stroke: Some(s), .. } if s.dash.is_none() => (v + 1, h),
            Item::Path { stroke: Some(_), .. } => (v, h + 1),
            _ => (v, h),
        })
    }

    #[test]
    fn a_cube_behind_a_cube_is_dashed() {
        let (view, cam) = front_view();
        let m = Mapping::fit(&view, &[Vec2::new(-1.0, -1.0), Vec2::new(1.0, 1.0)], A3, 20.0, None);
        let mut page = Page::new(A3);
        draw(&mut page, &m, &cam, &[cube(Vec3::ZERO)], Style::drawing(true));
        let (visible, hidden) = lines(&page);
        // Seen straight from the front: the front square shows, the back one is behind it.
        assert!(visible >= 4, "{visible}");
        assert!(hidden >= 1, "{hidden}");

        let mut omitted = Page::new(A3);
        draw(&mut omitted, &m, &cam, &[cube(Vec3::ZERO), cube(Vec3::new(0.0, 0.0, -3.0))], Style::drawing(false));
        assert_eq!(lines(&omitted).1, 0, "hidden lines left out");
    }

    #[test]
    fn outlines_and_simplifying() {
        // A 3 x 2 block of pixels: one loop around it.
        let mut mask = vec![false; 25];
        for y in 1..3 {
            for x in 1..4 {
                mask[y * 5 + x] = true;
            }
        }
        let loops = outlines(&mask, 5, 5);
        assert_eq!(loops.len(), 1);
        let simple = simplify(&loops[0], 0.1);
        // The rectangle's four corners, and its start (on an edge) twice.
        for corner in [Vec2::new(1.0, 1.0), Vec2::new(4.0, 1.0), Vec2::new(4.0, 3.0), Vec2::new(1.0, 3.0)] {
            assert!(simple.contains(&corner), "{simple:?}");
        }
        assert!(simple.len() <= 6, "{simple:?}");
    }
}
