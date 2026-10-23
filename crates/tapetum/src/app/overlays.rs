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
    const TAB: f32 = 26.0;
    let hidden = (height - TAB) * (1.0 - t);
    let area = egui::Area::new(id).order(egui::Order::Foreground).constrain(false).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, hidden)).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            // The tab that always shows: where the faders live.
            egui::Frame::popup(ui.style()).corner_radius(egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }).inner_margin(egui::Margin::symmetric(14, 3)).show(ui, |ui| {
                // A sun and a little arrow up (drawn: the font has no arrow glyph).
                let r = ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("☀").color(ACCENT));
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    let c = rect.center();
                    let tip = if t > 0.5 { 3.5 } else { -3.5 };
                    ui.painter().add(egui::Shape::convex_polygon(vec![c + egui::vec2(0.0, tip), c + egui::vec2(-4.5, -tip), c + egui::vec2(4.5, -tip)], ACCENT, egui::Stroke::NONE));
                });
                r.response.on_hover_text(tr("Licht und Haze", "Light and haze"));
            });
            egui::Frame::popup(ui.style()).corner_radius(egui::CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 }).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
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

/// The scenes as bookmarks from the top edge: the current one always shows,
/// the others while the mouse is over the bar. A right click edits a scene.
pub fn scene_bar(ctx: &egui::Context, state: &ShowState, project: &mut Project, rt: &SceneRuntime) -> (Option<ScenePick>, egui::Rect) {
    let id = egui::Id::new("scene_bar");
    let t = openness(ctx, id);
    let screen = ctx.content_rect();
    let mut pick = None;
    const TAB_H: f32 = 30.0;
    const DROP: f32 = 12.0;
    let detached = rt.detached.is_some();
    let mut order: Vec<usize> = (0..project.scenes.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&project.scenes[a], &project.scenes[b]);
        sa.cue_id.unwrap_or(f64::MAX).total_cmp(&sb.cue_id.unwrap_or(f64::MAX)).then(sa.id.cmp(&sb.id))
    });
    let cues: Vec<(f64, String)> = state.show.conductor().map(|c| c.cues.iter().map(|q| (q.id, q.name.clone())).collect()).unwrap_or_default();
    let mut delete = None;
    let mut add = false;

    let area = egui::Area::new(id).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Every tab keeps its place (so the current one doesn't jump when the
            // others come in); hidden ones are just not drawn.
            let tab = |ui: &mut egui::Ui, scene: Option<u64>, label: String, cue: Option<f64>| -> Option<egui::Response> {
                let active = rt.active == scene;
                let font = egui::FontId::proportional(14.0);
                let width = ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), font, egui::Color32::WHITE).size().x) + 28.0;
                let (slot, _) = ui.allocate_exact_size(egui::vec2(width.max(70.0), TAB_H + DROP), egui::Sense::hover());
                // The current tab hangs down fully; the others drop in while open.
                let shown = if active { 1.0 } else { t };
                if shown <= 0.01 {
                    return None;
                }
                let height = TAB_H + if active { DROP } else { 0.0 };
                let rect = egui::Rect::from_min_size(slot.min - egui::vec2(0.0, (1.0 - shown) * height), egui::vec2(slot.width(), height));
                let fill = if active { if detached { ACCENT } else { egui::Color32::from_rgb(61, 139, 255) } } else { ui.visuals().window_fill };
                let text_color = if active { egui::Color32::BLACK } else { ui.visuals().text_color() };
                let mut text = egui::RichText::new(&label).color(text_color);
                if active {
                    text = text.strong();
                }
                let button = egui::Button::new(text).fill(fill).corner_radius(egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 });
                let mut resp = ui.put(rect, button);
                if let Some(c) = cue {
                    resp = resp.on_hover_text(trf!("ab Cue {} der Main-Cuelist", "from cue {} of the main cuelist", fmt_cue(c)));
                }
                Some(resp)
            };
            if let Some(r) = tab(ui, None, tr("Grundszene", "Base scene").to_string(), None)
                && r.clicked()
            {
                pick = Some(ScenePick::Show(None));
            }
            for &i in &order {
                let (sid, name, cue) = (project.scenes[i].id, project.scenes[i].name.clone(), project.scenes[i].cue_id);
                let label = match cue {
                    Some(c) => format!("{name}  ·  {}", fmt_cue(c)),
                    None => name,
                };
                let Some(r) = tab(ui, Some(sid), label, cue) else { continue };
                if r.clicked() {
                    pick = Some(ScenePick::Show(Some(sid)));
                }
                r.context_menu(|ui| {
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
            let (slot, _) = ui.allocate_exact_size(egui::vec2(36.0, TAB_H + DROP), egui::Sense::hover());
            if t > 0.01 {
                let rect = egui::Rect::from_min_size(slot.min - egui::vec2(0.0, (1.0 - t) * TAB_H), egui::vec2(slot.width(), TAB_H));
                let button = egui::Button::new("+").corner_radius(egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 });
                if ui.put(rect, button).on_hover_text(tr("Neue Szene; zeigt zunächst die Grundszene. Rechtsklick auf eine Szene: Name, Cue, Übergang.", "New scene; starts out as the base scene. Right click a scene: name, cue, transition.")).clicked() {
                    add = true;
                }
            }
        });
        // Not in step with BlinderKitten: say so, and offer to follow again.
        if detached {
            ui.horizontal(|ui| {
                ui.colored_label(ACCENT, tr("Von Hand gewählt – nicht synchron mit BlinderKitten. Beim nächsten Cue folgt Tapetum wieder.", "Chosen by hand – not in step with BlinderKitten. Tapetum follows again at the next cue."));
                if ui.small_button(tr("Jetzt folgen", "Follow now")).clicked() {
                    pick = Some(ScenePick::Follow);
                }
            });
        } else if rt.moving() && t > 0.5 {
            ui.colored_label(egui::Color32::LIGHT_YELLOW, tr("Übergang läuft …", "Transition running …"));
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
    let visible = area.response.rect.intersect(screen);
    remember(ctx, id, visible);
    (pick, visible)
}
