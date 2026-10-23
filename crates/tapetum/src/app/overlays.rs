//! Controls over the 3D view that stay out of the way: the light and haze
//! faders slide up from the bottom edge, the scenes hang down from the top
//! like bookmarks. Both come out while the mouse is over them.

use bevy_egui::egui;
use tapetum_project::{Project, Quality};

use super::objects::SceneRuntime;
use super::state::ShowState;
use crate::i18n::tr;

static OPEN_FOR_SCREENSHOTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keep both overlays out (for screenshots, which have no mouse).
pub fn keep_open() {
    OPEN_FOR_SCREENSHOTS.store(true, std::sync::atomic::Ordering::Relaxed);
}

const ACCENT: egui::Color32 = egui::Color32::from_rgb(255, 194, 77);
const SLIDE: f32 = 0.18;

/// How far an overlay is out (0 hidden … 1 out): out while the mouse is over
/// what it showed last frame, or while one of its controls is in use.
fn openness(ctx: &egui::Context, id: egui::Id) -> f32 {
    let rect: Option<egui::Rect> = ctx.data(|d| d.get_temp(id.with("rect")));
    let was_open = ctx.data(|d| d.get_temp::<bool>(id.with("open")).unwrap_or(false));
    let over = ctx.pointer_hover_pos().zip(rect).is_some_and(|(p, r)| r.expand(10.0).contains(p));
    let busy = was_open && (ctx.dragged_id().is_some() || egui::Popup::is_any_open(ctx));
    let open = over || busy || OPEN_FOR_SCREENSHOTS.load(std::sync::atomic::Ordering::Relaxed);
    ctx.data_mut(|d| d.insert_temp(id.with("open"), open));
    ctx.animate_bool_with_time(id, open, SLIDE)
}

/// The part of the window the 3D view shows between the side panels (as of
/// last frame); the overlays centre on it.
pub fn set_free_view(ctx: &egui::Context, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("free_view"), rect));
}

fn free_view(ctx: &egui::Context) -> egui::Rect {
    ctx.data(|d| d.get_temp(egui::Id::new("free_view"))).unwrap_or_else(|| ctx.content_rect())
}

fn remember(ctx: &egui::Context, id: egui::Id, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(id.with("rect"), rect));
}

/// One labelled vertical fader.
fn fader(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, label: &str, enabled: bool) {
    ui.vertical(|ui| {
        ui.set_width(64.0);
        ui.add_enabled_ui(enabled, |ui| {
            ui.spacing_mut().slider_width = 110.0;
            ui.add(egui::Slider::new(value, range).vertical().max_decimals(2));
        });
        ui.label(egui::RichText::new(label).small());
    });
}

/// Light and haze at the bottom edge. Returns the area it covers.
pub fn faders(ctx: &egui::Context, project: &mut Project, spacemouse: bool) -> egui::Rect {
    let id = egui::Id::new("faders");
    let t = openness(ctx, id);
    let screen = ctx.content_rect();
    let height: f32 = ctx.data(|d| d.get_temp(id.with("height")).unwrap_or(200.0));
    // The faders' width from last frame: the tab centres over them.
    let width: f32 = ctx.data(|d| d.get_temp(id.with("width")).unwrap_or(0.0));
    const TAB: f32 = 26.0;
    let hidden = (height - TAB) * (1.0 - t);
    let shift = free_view(ctx).center().x - screen.center().x;
    let area = egui::Area::new(id).order(egui::Order::Foreground).constrain(false).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(shift, hidden)).show(ctx, |ui| {
        ui.set_min_width(width);
        ui.vertical_centered(|ui| {
            // The tab that always shows: where the faders live. Drawn by hand
            // at a fixed size, so the centred layout really centres it.
            let (tab, resp) = ui.allocate_exact_size(egui::vec2(64.0, TAB), egui::Sense::hover());
            let visuals = ui.visuals();
            ui.painter().rect(tab, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, visuals.window_fill, visuals.window_stroke, egui::StrokeKind::Inside);
            let sun = tab.center() - egui::vec2(9.0, 0.0);
            ui.painter().text(sun, egui::Align2::CENTER_CENTER, "☀", egui::FontId::proportional(14.0), ACCENT);
            let c = tab.center() + egui::vec2(11.0, 0.0);
            // A little arrow (the font has none): up while closed, down while open.
            let tip = if t > 0.5 { 3.5 } else { -3.5 };
            ui.painter().add(egui::Shape::convex_polygon(vec![c + egui::vec2(0.0, tip), c + egui::vec2(-4.5, -tip), c + egui::vec2(4.5, -tip)], ACCENT, egui::Stroke::NONE));
            resp.on_hover_text(tr("Licht und Haze", "Light and haze"));
            ui.add_space(-ui.spacing().item_spacing.y);
            let panel = egui::Frame::popup(ui.style()).corner_radius(egui::CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 }).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                let r = &mut project.render;
                ui.horizontal(|ui| {
                    fader(ui, &mut r.ambient, 0.0..=1.0, tr("Saallicht", "House light"), true);
                    fader(ui, &mut r.exposure_ev, 0.0..=12.0, tr("Belichtung (EV)", "Exposure (EV)"), true);
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(64.0);
                        ui.add_space(4.0);
                        ui.toggle_value(&mut r.haze.enabled, "Haze");
                        ui.add_space(8.0);
                        ui.add_enabled_ui(r.quality != Quality::Low, |ui| ui.toggle_value(&mut r.bloom, "Bloom"))
                            .response
                            .on_disabled_hover_text(tr("Auf „Niedrig“ aus, um schwache Grafik zu entlasten.", "Off on “Low” to spare weak graphics."));
                    });
                    fader(ui, &mut r.haze.density, 0.0..=0.5, tr("Dichte", "Density"), r.haze.enabled);
                    fader(ui, &mut r.haze.anisotropy, -0.9..=0.95, tr("Streuung vorwärts", "Forward scattering"), r.haze.enabled);
                    if spacemouse {
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.set_width(110.0);
                            ui.add_space(4.0);
                            ui.toggle_value(&mut project.spacemouse.keep_in_background, tr("SpaceMouse im Hintergrund", "SpaceMouse in background"))
                                .on_hover_text(tr("Die SpaceMouse steuert Tapetum weiter, während du BlinderKitten bedienst. Andere Programme bekommen sie dann nicht.", "The SpaceMouse keeps controlling Tapetum while you operate BlinderKitten. Other programs then don't get it."));
                        });
                    }
                });
            });
            ctx.data_mut(|d| d.insert_temp(id.with("width"), panel.response.rect.width()));
        });
    });
    let rect = area.response.rect;
    ctx.data_mut(|d| d.insert_temp(id.with("height"), rect.height()));
    let visible = rect.intersect(screen);
    remember(ctx, id, visible);
    visible
}

/// What a click in the scene bar asks for.
pub enum ScenePick {
    /// Show this scene (`None` = the base scene).
    Show(Option<u64>),
    /// Follow the main cuelist again.
    Follow,
}

fn fmt_cue(c: f64) -> String {
    if c.fract().abs() < 1e-9 { format!("{c:.0}") } else { format!("{c}") }
}

/// The scenes as bookmarks from the top edge: the current one in the middle,
/// always showing; the others beside it, in their order, come down while the
/// mouse is over the bar – the farther from the current one, the less. Arrow
/// keys step through them; a right click edits one.
pub fn scene_bar(ctx: &egui::Context, state: &ShowState, project: &mut Project, rt: &SceneRuntime) -> (Option<ScenePick>, egui::Rect) {
    let id = egui::Id::new("scene_bar");
    let t = openness(ctx, id);
    let screen = ctx.content_rect();
    let mut pick = None;
    const TAB_H: f32 = 30.0;
    const DROP: f32 = 12.0;
    const GAP: f32 = 4.0;
    const PLUS_W: f32 = 32.0;
    let detached = rt.detached.is_some();

    // Base scene first, then the scenes by cue.
    let mut order: Vec<usize> = (0..project.scenes.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&project.scenes[a], &project.scenes[b]);
        sa.cue_id.unwrap_or(f64::MAX).total_cmp(&sb.cue_id.unwrap_or(f64::MAX)).then(sa.id.cmp(&sb.id))
    });
    let mut tabs: Vec<(Option<u64>, Option<usize>, String, Option<f64>)> = vec![(None, None, tr("Grundszene", "Base scene").to_string(), None)];
    for &i in &order {
        let sc = &project.scenes[i];
        let label = match sc.cue_id {
            Some(c) => format!("{}  ·  {}", sc.name, fmt_cue(c)),
            None => sc.name.clone(),
        };
        tabs.push((Some(sc.id), Some(i), label, sc.cue_id));
    }
    let current = tabs.iter().position(|tab| tab.0 == rt.active).unwrap_or(0);

    // ← and → step through the scenes (not while typing).
    if !ctx.egui_wants_keyboard_input() {
        let (left, right) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight)));
        if left && current > 0 {
            pick = Some(ScenePick::Show(tabs[current - 1].0));
        } else if right && current + 1 < tabs.len() {
            pick = Some(ScenePick::Show(tabs[current + 1].0));
        }
    }

    // Places: the current tab centred, the others to its left and right, "+" last.
    let font = egui::FontId::proportional(14.0);
    let widths: Vec<f32> = tabs.iter().map(|tab| (ctx.fonts_mut(|f| f.layout_no_wrap(tab.2.clone(), font.clone(), egui::Color32::WHITE).size().x) + 28.0).max(70.0)).collect();
    let n = tabs.len();
    let mut left = vec![0.0f32; n];
    let middle = free_view(ctx).center().x;
    left[current] = middle - widths[current] / 2.0;
    for k in (0..current).rev() {
        left[k] = left[k + 1] - GAP - widths[k];
    }
    for k in current + 1..n {
        left[k] = left[k - 1] + widths[k - 1] + GAP;
    }
    let plus_left = left[n - 1] + widths[n - 1] + GAP;
    // How far a tab comes down: the current one fully, its neighbours a bit
    // less, those beyond them less again.
    let reach = |k: usize| -> f32 {
        if k == current {
            return TAB_H + DROP;
        }
        let dist = k.abs_diff(current) as f32;
        TAB_H * t * (1.0 - 0.13 * (dist - 1.0)).max(0.5)
    };

    let mut delete = None;
    let mut add = false;
    let mut covered = egui::Rect::NOTHING;
    let cues: Vec<(f64, String)> = state.show.conductor().map(|c| c.cues.iter().map(|q| (q.id, q.name.clone())).collect()).unwrap_or_default();
    // Where each tab is (all of it) and the part of it on screen; first, so the
    // area can take exactly that room (egui hit-tests by an area's own rect).
    let visible = |rect: egui::Rect| rect.intersect(egui::Rect::from_min_max(egui::pos2(rect.left(), 0.0), rect.max)).intersect(screen);
    let places: Vec<Option<(egui::Rect, egui::Rect)>> = (0..n)
        .map(|k| {
            let reach = reach(k);
            let full = TAB_H + if k == current { DROP } else { 0.0 };
            let rect = egui::Rect::from_min_size(egui::pos2(left[k], reach - full), egui::vec2(widths[k], full));
            (reach >= 2.0 && visible(rect).is_positive()).then(|| (rect, visible(rect)))
        })
        .collect();
    let plus_reach = TAB_H * t * (1.0 - 0.13 * ((n - current) as f32 - 1.0)).max(0.5);
    let plus_rect = egui::Rect::from_min_size(egui::pos2(plus_left, plus_reach - TAB_H), egui::vec2(PLUS_W, TAB_H));
    let plus = (plus_reach >= 2.0 && visible(plus_rect).is_positive()).then(|| (plus_rect, visible(plus_rect)));
    let line = detached.then(|| egui::Rect::from_center_size(egui::pos2(middle, TAB_H + DROP + 15.0), egui::vec2(screen.width().min(760.0), 22.0)));
    let span = places.iter().flatten().map(|p| p.1).chain(plus.map(|p| p.1)).chain(line).fold(egui::Rect::NOTHING, |a, b| a.union(b));
    if !span.is_positive() {
        return (pick, egui::Rect::NOTHING);
    }
    egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(span.min).constrain(false).show(ctx, |ui| {
        ui.allocate_exact_size(span.size(), egui::Sense::hover());
        ui.set_clip_rect(screen);
        // Painted by hand: each tab slides down from above the edge (clipped
        // there), so nothing gets resized and nothing jumps.
        let visuals = ui.visuals().clone();
        for (k, (scene, index, label, cue)) in tabs.iter().enumerate() {
            let Some((rect, shown)) = places[k] else { continue };
            let active = k == current;
            let resp = ui.interact(shown, id.with(("tab", k)), egui::Sense::click());
            let fill = if active {
                if detached { ACCENT } else { egui::Color32::from_rgb(61, 139, 255) }
            } else if resp.hovered() {
                visuals.widgets.hovered.bg_fill
            } else {
                visuals.window_fill
            };
            let painter = ui.painter();
            painter.rect(rect, egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, fill, visuals.window_stroke, egui::StrokeKind::Inside);
            let text_color = if active { egui::Color32::BLACK } else { visuals.text_color() };
            let text_font = if active { egui::FontId::new(14.0, egui::FontFamily::Proportional) } else { font.clone() };
            painter.text(egui::pos2(rect.center().x, rect.bottom() - TAB_H / 2.0), egui::Align2::CENTER_CENTER, label, text_font, text_color);
            covered = covered.union(shown);
            let resp = match cue {
                Some(c) => resp.on_hover_text(trf!("ab Cue {} der Main-Cuelist", "from cue {} of the main cuelist", fmt_cue(*c))),
                None => resp,
            };
            if resp.clicked() {
                pick = Some(ScenePick::Show(*scene));
            }
            if let (Some(i), Some(sid)) = (*index, *scene) {
                resp.context_menu(|ui| {
                    let scene = &mut project.scenes[i];
                    ui.add(egui::TextEdit::singleline(&mut scene.name).desired_width(200.0));
                    let cue_text = scene.cue_id.map_or(tr("keine (nur von Hand)", "none (manual only)").to_string(), |c| {
                        let n = cues.iter().find(|(q, _)| (q - c).abs() < 1e-6).map_or(String::new(), |(_, n)| format!(" – {n}"));
                        trf!("ab Cue {}{n}", "from cue {}{n}", fmt_cue(c), n = n)
                    });
                    egui::ComboBox::from_id_salt(("cue", sid)).selected_text(cue_text).width(220.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut scene.cue_id, None, tr("keine (nur von Hand)", "none (manual only)"));
                        for (q, n) in &cues {
                            ui.selectable_value(&mut scene.cue_id, Some(*q), trf!("Cue {} – {n}", "Cue {} – {n}", fmt_cue(*q), n = n));
                        }
                    });
                    ui.add(egui::DragValue::new(&mut scene.transition).range(0.0..=60.0).speed(0.1).suffix(" s").prefix(tr("Übergang ", "Transition ")));
                    ui.colored_label(egui::Color32::GRAY, tr("Objekte der Szene: Szene wählen, dann Objekte im 3D-Fenster verschieben, drehen oder ausblenden.", "The scene's objects: choose the scene, then move, turn or hide objects in the 3D view."));
                    if ui.button(tr("Szene löschen", "Delete scene")).clicked() {
                        delete = Some(sid);
                        ui.close();
                    }
                });
            }
        }
        // "+" after the last scene.
        if let Some((rect, shown)) = plus {
            {
                let resp = ui.interact(shown, id.with("plus"), egui::Sense::click());
                let fill = if resp.hovered() { visuals.widgets.hovered.bg_fill } else { visuals.widgets.inactive.bg_fill };
                ui.painter().rect(rect, egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, fill, visuals.window_stroke, egui::StrokeKind::Inside);
                ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - TAB_H / 2.0), egui::Align2::CENTER_CENTER, "+", font.clone(), visuals.text_color());
                covered = covered.union(shown);
                if resp.on_hover_text(tr("Neue Szene; zeigt zunächst die Grundszene. Rechtsklick auf eine Szene: Name, Cue, Übergang. Pfeiltasten ← →: Szene wechseln.", "New scene; starts out as the base scene. Right click a scene: name, cue, transition. Arrow keys ← →: change scene.")).clicked() {
                    add = true;
                }
            }
        }
        // Not in step with BlinderKitten: say so under the current tab, and offer to follow again.
        let below = egui::pos2(middle, TAB_H + DROP + 4.0);
        if let Some(line) = line {
            let inner = ui.scope_builder(egui::UiBuilder::new().max_rect(line).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(ACCENT, tr("Von Hand gewählt – nicht synchron mit BlinderKitten. Beim nächsten Cue folgt Tapetum wieder.", "Chosen by hand – not in step with BlinderKitten. Tapetum follows again at the next cue."));
                        if ui.small_button(tr("Jetzt folgen", "Follow now")).clicked() {
                            pick = Some(ScenePick::Follow);
                        }
                    });
                })
                .response
                .rect
            });
            covered = covered.union(inner.inner);
        } else if rt.moving() && t > 0.5 {
            ui.painter().text(below + egui::vec2(0.0, 8.0), egui::Align2::CENTER_TOP, tr("Übergang läuft …", "Transition running …"), egui::FontId::proportional(12.0), egui::Color32::LIGHT_YELLOW);
        }
    });
    if add {
        let sid = project.scenes.iter().map(|s| s.id).max().map_or(1, |m| m + 1);
        project.scenes.push(tapetum_project::Scene { id: sid, name: trf!("Szene {id}", "Scene {id}", id = sid), cue_id: rt.following_cue, ..Default::default() });
        pick = Some(ScenePick::Show(Some(sid)));
    }
    if let Some(sid) = delete {
        project.scenes.retain(|s| s.id != sid);
        pick = Some(ScenePick::Show(None));
    }
    let covered = if covered.is_positive() { covered } else { egui::Rect::from_center_size(egui::pos2(screen.center().x, 0.0), egui::Vec2::ZERO) };
    remember(ctx, id, covered);
    (pick, covered)
}
