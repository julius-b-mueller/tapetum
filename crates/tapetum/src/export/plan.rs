//! The fixture plan: every fixture as a symbol where the camera sees it, with
//! a label (ID and name, type, universe and address, channels) and, for
//! moving heads, the arrow of their middle direction. Orthographic views come
//! out to a standard scale with a scale bar.

use bevy::math::{Mat4, Vec2, Vec3};
use bk_olga::FixtureClass;

use super::drawing::{A3, Anchor, Color, Page, Stroke, text_width};
use super::linework::{Camera, Style, Surface, draw, picture_points};

/// What the plan shows of a fixture (positions and directions: Bevy world).
#[derive(Clone, Debug)]
pub struct PlanFixture {
    pub id: i64,
    pub name: String,
    pub type_name: String,
    /// Universe and address, if patched.
    pub patch: Option<(String, u16)>,
    pub channels: u16,
    pub class: FixtureClass,
    pub position: Vec3,
    /// Where the beam points (at rest); for a moving head its middle direction.
    pub forward: Vec3,
    /// The fixture's own x axis (a bar's length).
    pub across: Vec3,
}

/// The view and what goes in the title block.
#[derive(Clone)]
pub struct PlanView {
    pub clip_from_world: Mat4,
    /// Width / height of the picture.
    pub aspect: f32,
    /// Orthographic: meters from the middle to the top edge of the picture.
    pub ortho_half_height: Option<f32>,
    pub title: String,
    pub subtitle: String,
    pub date: String,
}

const MARGIN: f32 = 12.0;
/// The title block's height at the bottom.
pub const FOOTER: f32 = 22.0;
/// Room around the drawing for labels.
const LABEL_ROOM: f32 = 22.0;
const SYMBOL: f32 = 2.6;
const TEXT: f32 = 2.1;
const SCALES: [f32; 10] = [10.0, 20.0, 25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 250.0, 500.0];

/// From the camera's picture (x right in units of half its height, y down) to the page.
pub struct Mapping {
    scale: f32,
    offset: Vec2,
    /// The part of the page the camera's picture covers (min, max): the drawing stops there.
    clip: (Vec2, Vec2),
    /// 1:n, orthographic only.
    pub ratio: Option<f32>,
    clip_from_world: Mat4,
    aspect: f32,
}

impl Mapping {
    /// Only for picture coordinates (to fit a page with them).
    pub fn probe(view: &PlanView) -> Mapping {
        Mapping { scale: 1.0, offset: Vec2::ZERO, clip: (Vec2::splat(f32::MIN), Vec2::splat(f32::MAX)), ratio: None, clip_from_world: view.clip_from_world, aspect: view.aspect }
    }

    /// Inside the camera's picture?
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.clip.0.x && p[1] >= self.clip.0.y && p[0] <= self.clip.1.x && p[1] <= self.clip.1.y
    }

    /// Picture coordinates of a world point; `None` behind the camera.
    pub fn picture(&self, p: Vec3) -> Option<Vec2> {
        let c = self.clip_from_world * p.extend(1.0);
        if c.w <= 1e-5 {
            return None;
        }
        Some(Vec2::new(c.x / c.w * self.aspect, -c.y / c.w))
    }

    pub fn page(&self, p: Vec3) -> Option<[f32; 2]> {
        self.picture(p).map(|q| (q * self.scale + self.offset).to_array())
    }

    /// Fits the points (picture coordinates) into the drawing area of a page;
    /// orthographic at the largest standard scale that fits.
    pub fn fit(view: &PlanView, points: &[Vec2], page: [f32; 2], footer: f32) -> Mapping {
        let (lo, hi) = points.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        // What the camera sees of it (the picture spans ±aspect × ±1).
        let frame = Vec2::new(view.aspect, 1.0);
        let (lo, hi) = if lo.x > hi.x { (-frame, frame) } else { (lo.clamp(-frame, frame), hi.clamp(-frame, frame)) };
        let area_min = Vec2::splat(MARGIN + LABEL_ROOM);
        let area_max = Vec2::new(page[0], page[1] - footer) - Vec2::splat(MARGIN + LABEL_ROOM);
        let size = (hi - lo).max(Vec2::splat(1e-3));
        let fit = ((area_max - area_min) / size).min_element();
        let (scale, ratio) = match view.ortho_half_height {
            // A picture unit is `half` meters; 1:n puts 1000/n mm on a meter.
            Some(half) => {
                let n = SCALES.iter().copied().find(|n| 1000.0 / n * half <= fit).unwrap_or_else(|| (1000.0 * half / fit).ceil());
                (1000.0 / n * half, Some(n))
            }
            None => (fit, None),
        };
        let offset = (area_min + area_max) / 2.0 - (lo + hi) / 2.0 * scale;
        Mapping { scale, offset, clip: (lo * scale + offset, hi * scale + offset), ratio, clip_from_world: view.clip_from_world, aspect: view.aspect }
    }
}

fn rotate(v: [f32; 2], c: [f32; 2], dir: Vec2) -> [f32; 2] {
    [c[0] + v[0] * dir.x - v[1] * dir.y, c[1] + v[0] * dir.y + v[1] * dir.x]
}

/// Direction on the page of a world direction at a point (unit), if it isn't
/// pointing at the viewer.
fn page_dir(m: &Mapping, at: Vec3, dir: Vec3) -> Option<Vec2> {
    let (a, b) = (m.page(at)?, m.page(at + dir.normalize_or_zero() * 0.5)?);
    let d = Vec2::from(b) - Vec2::from(a);
    (d.length() > 0.4).then(|| d.normalize())
}

/// How far a symbol reaches from its middle, in mm.
fn reach(class: FixtureClass) -> f32 {
    match class {
        FixtureClass::Bar => 4.8,
        FixtureClass::Spot => 3.6,
        FixtureClass::Wash | FixtureClass::Conventional => 3.2,
        _ => SYMBOL,
    }
}

fn symbol(page: &mut Page, f: &PlanFixture, c: [f32; 2], m: &Mapping) {
    let line = Stroke::solid(Color::BLACK, 0.3);
    let fill = Some(Color::WHITE);
    let forward = page_dir(m, f.position, f.forward);
    let along = forward.unwrap_or(Vec2::X);
    let shape = |page: &mut Page, pts: &[[f32; 2]], dir: Vec2| page.polyline(pts.iter().map(|p| rotate(*p, c, dir)).collect(), true, Some(line), fill);
    match f.class {
        FixtureClass::MovingHead => {
            page.circle(c, SYMBOL, Some(line), fill);
            page.circle(c, SYMBOL * 0.45, None, Some(Color::BLACK));
            if let Some(d) = forward {
                // The middle direction: an arrow out of the symbol.
                let (a, b) = ((Vec2::from(c) + d * SYMBOL).to_array(), (Vec2::from(c) + d * (SYMBOL + 6.0)).to_array());
                page.line(a, b, Stroke::solid(Color::BLACK, 0.35));
                let n = Vec2::new(-d.y, d.x);
                let tip = Vec2::from(b);
                page.polyline(vec![b, (tip - d * 1.6 + n * 0.9).to_array(), (tip - d * 1.6 - n * 0.9).to_array()], true, None, Some(Color::BLACK));
            }
        }
        FixtureClass::Bar => {
            let across = page_dir(m, f.position, f.across).unwrap_or(Vec2::X);
            shape(page, &[[-4.5, -1.2], [4.5, -1.2], [4.5, 1.2], [-4.5, 1.2]], across);
        }
        FixtureClass::Spot => {
            // A profile: long body, the lens at the front.
            shape(page, &[[-3.2, -1.5], [2.4, -1.5], [3.2, -1.0], [3.2, 1.0], [2.4, 1.5], [-3.2, 1.5]], along);
            page.line(rotate([2.4, -1.5], c, along), rotate([2.4, 1.5], c, along), line);
        }
        FixtureClass::Wash => {
            shape(page, &[[-2.4, -2.4], [2.4, -2.4], [2.4, 2.4], [-2.4, 2.4]], along);
            page.circle(c, 1.5, Some(line), None);
        }
        FixtureClass::Conventional => {
            shape(page, &[[-2.6, -1.8], [2.0, -1.8], [2.0, 1.8], [-2.6, 1.8]], along);
            page.line(rotate([2.0, -2.4], c, along), rotate([2.0, 2.4], c, along), Stroke::solid(Color::BLACK, 0.45));
        }
        FixtureClass::Unknown => page.rect([c[0] - 1.6, c[1] - 1.6], [c[0] + 1.6, c[1] + 1.6], Some(line), fill),
    }
}

/// The three lines of a label.
fn label_lines(f: &PlanFixture, channels: &str) -> [String; 3] {
    let patch = f.patch.as_ref().map_or_else(|| "–".to_string(), |(u, a)| format!("{u} / {a}"));
    [format!("{} {}", f.id, f.name), f.type_name.clone(), format!("{patch} · {} {channels}", f.channels)]
}

#[derive(Clone, Copy)]
struct Rect {
    min: Vec2,
    max: Vec2,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.min.x < o.max.x && o.min.x < self.max.x && self.min.y < o.max.y && o.min.y < self.max.y
    }
}

/// Places the labels: around each symbol where they hit no other label or
/// symbol, farther out with a leader line if they must.
fn labels(page: &mut Page, items: &[([f32; 2], f32, [String; 3])], area: Rect) {
    let line_h = TEXT * 1.18;
    let size = |lines: &[String; 3]| Vec2::new(lines.iter().enumerate().map(|(i, l)| text_width(l, TEXT, i == 0)).fold(0.0, f32::max), line_h * 3.0);
    let mut taken: Vec<Rect> = items.iter().map(|(c, r, _)| Rect { min: Vec2::from(*c) - *r - 0.6, max: Vec2::from(*c) + *r + 0.6 }).collect();
    let directions = [(1.0, 0.0), (-1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    for (c, r, lines) in items {
        let (c, r) = (Vec2::from(*c), *r);
        let s = size(lines);
        // Top left corner of a label next to the symbol in a direction, `gap` away.
        let place = |(dx, dy): (f32, f32), gap: f32| {
            let x = match dx {
                d if d > 0.0 => c.x + gap,
                d if d < 0.0 => c.x - gap - s.x,
                _ => c.x - s.x / 2.0,
            };
            let y = match dy {
                d if d > 0.0 => c.y + gap,
                d if d < 0.0 => c.y - gap - s.y,
                _ => c.y - s.y / 2.0,
            };
            Rect { min: Vec2::new(x, y), max: Vec2::new(x, y) + s }
        };
        let inside = |r: &Rect| r.min.x >= area.min.x && r.min.y >= area.min.y && r.max.x <= area.max.x && r.max.y <= area.max.y;
        let free = |r: &Rect| inside(r) && !taken.iter().any(|t| t.overlaps(r));
        let gaps = [r + 1.2, r + 6.0, r + 12.0];
        let chosen = gaps.iter().flat_map(|g| directions.iter().map(move |d| (*d, *g))).map(|(d, g)| (place(d, g), g)).find(|(r, _)| free(r));
        let (rect, gap) = chosen.unwrap_or((place((1.0, 0.0), gaps[0]), gaps[0]));
        if gap > gaps[0] {
            // A leader from the symbol to the nearest point of the label.
            let near = c.clamp(rect.min, rect.max);
            let start = c + (near - c).normalize_or_zero() * r;
            page.line(start.to_array(), near.to_array(), Stroke::solid(Color::gray(0.35), 0.15));
        }
        page.rect((rect.min - 0.4).to_array(), (rect.max + 0.4).to_array(), None, Some(Color::WHITE));
        for (i, l) in lines.iter().enumerate() {
            page.text([rect.min.x, rect.min.y + line_h * (i as f32 + 1.0) - 0.45], TEXT, l.clone(), Anchor::Left, i == 0);
        }
        taken.push(rect);
    }
}

/// Title block and, orthographic, scale and scale bar.
pub fn footer(page: &mut Page, view: &PlanView, ratio: Option<f32>) {
    let [w, h] = page.size;
    let top = h - MARGIN - FOOTER + 6.0;
    page.line([MARGIN, top], [w - MARGIN, top], Stroke::solid(Color::BLACK, 0.3));
    page.text([MARGIN, top + 7.0], 4.2, view.title.clone(), Anchor::Left, true);
    page.text([MARGIN, top + 12.5], 3.0, view.subtitle.clone(), Anchor::Left, false);
    page.text([w - MARGIN, top + 7.0], 3.0, view.date.clone(), Anchor::Right, false);
    if let Some(n) = ratio {
        page.text([w - MARGIN, top + 12.5], 3.0, format!("1:{n}"), Anchor::Right, true);
        // A bar of a round number of meters, about 40 mm long.
        let per_m = 1000.0 / n;
        let meters = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0].into_iter().find(|m| m * per_m >= 25.0).unwrap_or(20.0);
        let (x1, y) = (w - MARGIN - 30.0, top + 11.5);
        let x0 = x1 - meters * per_m;
        for i in 0..2 {
            let half = (x1 - x0) / 2.0;
            let fill = if i == 0 { Color::BLACK } else { Color::WHITE };
            page.rect([x0 + half * i as f32, y - 1.4], [x0 + half * (i + 1) as f32, y], Some(Stroke::solid(Color::BLACK, 0.2)), Some(fill));
        }
        page.text([x0, y - 2.2], 2.4, "0", Anchor::Center, false);
        page.text([x1, y - 2.2], 2.4, format!("{} m", if meters.fract() == 0.0 { format!("{meters:.0}") } else { format!("{meters}") }), Anchor::Center, false);
    }
}

/// The fixture plan on an A3 page, the set faint underneath; `channels` is
/// the word for channels.
pub fn fixture_plan(view: &PlanView, fixtures: &[PlanFixture], set: Option<(&Camera, &[Surface])>, channels: &str) -> Page {
    let mut page = Page::new(A3);
    let probe = Mapping::probe(view);
    let mut points: Vec<Vec2> = fixtures.iter().filter_map(|f| probe.picture(f.position)).collect();
    if let Some((_, surfaces)) = set {
        points.extend(picture_points(&probe, surfaces));
    }
    let m = Mapping::fit(view, &points, page.size, FOOTER);
    if let Some((cam, surfaces)) = set {
        draw(&mut page, &m, cam, surfaces, Style::background());
    }
    // Far ones first, so the nearer symbols lie on top.
    let mut order: Vec<(usize, f32)> = fixtures.iter().enumerate().filter_map(|(i, f)| m.page(f.position).map(|_| (i, (view.clip_from_world * f.position.extend(1.0)).w))).collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut placed = Vec::new();
    for (i, _) in &order {
        let f = &fixtures[*i];
        let Some(c) = m.page(f.position).filter(|c| m.contains(*c)) else { continue };
        symbol(&mut page, f, c, &m);
        placed.push((c, reach(f.class), label_lines(f, channels)));
    }
    let area = Rect { min: Vec2::splat(MARGIN), max: Vec2::new(page.size[0] - MARGIN, page.size[1] - MARGIN - FOOTER) };
    labels(&mut page, &placed, area);
    footer(&mut page, view, m.ratio);
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::drawing::Item;

    fn top_view() -> PlanView {
        // Orthographic from above, 10 m from the middle to the top edge.
        let view = bevy::prelude::Transform::from_xyz(0.0, 50.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z).to_matrix().inverse();
        let proj = Mat4::from_diagonal(bevy::math::Vec4::new(1.0 / 16.0, 1.0 / 10.0, -0.005, 1.0));
        PlanView { clip_from_world: proj * view, aspect: 1.6, ortho_half_height: Some(10.0), title: "Show".into(), subtitle: "Oben".into(), date: "2026-10-24".into() }
    }

    fn head(id: i64, x: f32) -> PlanFixture {
        PlanFixture { id, name: format!("MH {id}"), type_name: "Moving Head Wash".into(), patch: Some(("0:0:0".into(), 1)), channels: 36, class: FixtureClass::MovingHead, position: Vec3::new(x, 6.0, 0.0), forward: Vec3::Z, across: Vec3::X }
    }

    #[test]
    fn a_plan_to_scale_with_every_label() {
        let fixtures: Vec<PlanFixture> = (0..8).map(|i| head(i, -3.5 + i as f32)).collect();
        let page = fixture_plan(&top_view(), &fixtures, None, "Kanäle");
        let texts: Vec<&str> = page.items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect();
        for i in 0..8 {
            assert!(texts.contains(&format!("{i} MH {i}").as_str()), "{texts:?}");
        }
        assert!(texts.contains(&"0:0:0 / 1 · 36 Kanäle"));
        // 7 m of heads: 350 mm at 1:20 fit the 352 mm A3 leaves for the drawing.
        assert!(texts.contains(&"1:20"), "{texts:?}");
    }

    #[test]
    fn labels_dont_overlap() {
        let fixtures: Vec<PlanFixture> = (0..12).map(|i| head(i, -1.1 + i as f32 * 0.2)).collect();
        let page = fixture_plan(&top_view(), &fixtures, None, "ch");
        // The white backgrounds of the labels.
        let boxes: Vec<(f32, f32, f32, f32)> = page
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Path { points, fill: Some(c), stroke: None, closed: true } if *c == Color::WHITE && points.len() == 4 => Some((points[0][0], points[0][1], points[2][0], points[2][1])),
                _ => None,
            })
            .collect();
        assert_eq!(boxes.len(), 12);
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let overlap = a.0 < b.2 - 0.9 && b.0 < a.2 - 0.9 && a.1 < b.3 - 0.9 && b.1 < a.3 - 0.9;
                assert!(!overlap, "{a:?} {b:?}");
            }
        }
    }
}
