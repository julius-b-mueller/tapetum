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
fn openness(ctx: &egui::Context, id: egui::Id, also: bool) -> f32 {
    let rect: Option<egui::Rect> = ctx.data(|d| d.get_temp(id.with("rect")));
    let was_open = ctx.data(|d| d.get_temp::<bool>(id.with("open")).unwrap_or(false));
    let over = ctx.pointer_hover_pos().zip(rect).is_some_and(|(p, r)| r.expand(10.0).contains(p));
    let busy = was_open && (ctx.dragged_id().is_some() || egui::Popup::is_any_open(ctx));
    let open = over || busy || also || OPEN_FOR_SCREENSHOTS.load(std::sync::atomic::Ordering::Relaxed);
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
    let t = openness(ctx, id, false);
    let screen = ctx.content_rect();
    let height: f32 = ctx.data(|d| d.get_temp(id.with("height")).unwrap_or(200.0));
    // The faders' width from last frame: the tab centres over them.
    let width: f32 = ctx.data(|d| d.get_temp(id.with("width")).unwrap_or(0.0));
    const TAB: f32 = 26.0;
    let hidden = (height - TAB) * (1.0 - t);
    // Centred on the window, so it stays put when the side panels come and go.
    let area = egui::Area::new(id).order(egui::Order::Foreground).constrain(false).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, hidden)).show(ctx, |ui| {
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

/// The scenes in their order: by the cue of the main cuelist each is bound to
/// (as in BlinderKitten); one without a cue stays behind the scene before it
/// in the project, where it was added. Indices into `project.scenes`.
pub fn scene_order(project: &Project) -> Vec<usize> {
    let mut key = f64::NEG_INFINITY;
    let mut keyed: Vec<(f64, usize)> = project
        .scenes
        .iter()
        .enumerate()
        .map(|(i, sc)| {
            if let Some(c) = sc.cue_id {
                key = c;
            }
            (sc.cue_id.unwrap_or(key), i)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    keyed.into_iter().map(|(_, i)| i).collect()
}

/// Where the strip of tabs stands, gliding from one scene to the next while
/// the bar is out (and jumping while it is in).
fn glide(ctx: &egui::Context, id: egui::Id, target: f32, out: bool) -> f32 {
    const TIME: f64 = 0.25;
    let now = ctx.input(|i| i.time);
    let (from, to, start): (f32, f32, f64) = ctx.data(|d| d.get_temp(id)).unwrap_or((target, target, now));
    let x = ((now - start) / TIME).clamp(0.0, 1.0) as f32;
    let here = from + (to - from) * (x * x * (3.0 - 2.0 * x));
    let state = if !out {
        (target, target, now)
    } else if (to - target).abs() > 1e-3 {
        (here, target, now)
    } else {
        (from, to, start)
    };
    ctx.data_mut(|d| d.insert_temp(id, state));
    if x < 1.0 {
        ctx.request_repaint();
    }
    if out { here } else { target }
}

/// The scenes as bookmarks from the top edge: the current one in the middle,
/// always showing; three on either side come down while the mouse is over the
/// bar – the farther from the current one, the less. Changing the scene while
/// they are out slides them along. "+" beside the current one adds a scene
/// before or after it; a right click edits one; arrow keys step through them.
pub fn scene_bar(ctx: &egui::Context, state: &ShowState, project: &mut Project, rt: &SceneRuntime) -> (Option<ScenePick>, egui::Rect) {
    let id = egui::Id::new("scene_bar");
    let editing_id = id.with("editing");
    let editing: Option<u64> = ctx.data(|d| d.get_temp(editing_id)).filter(|sid| project.scenes.iter().any(|s| s.id == *sid));
    // While a scene is edited, the bar stays out.
    if editing.is_some() {
        ctx.data_mut(|d| d.insert_temp(id.with("open"), true));
    }
    // ← and → (not while typing) bring the scenes out for a moment, as if the
    // mouse were there, so the change is seen sliding.
    let now = ctx.input(|i| i.time);
    let (step_left, step_right) = if ctx.text_edit_focused() { (false, false) } else { ctx.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight))) };
    if step_left || step_right {
        ctx.data_mut(|d| d.insert_temp(id.with("peek"), now + 1.2));
    }
    let peeking = ctx.data(|d| d.get_temp::<f64>(id.with("peek"))).is_some_and(|until| now < until);
    if peeking {
        ctx.request_repaint();
    }
    let t = openness(ctx, id, peeking).max(if editing.is_some() { 1.0 } else { 0.0 });
    let screen = ctx.content_rect();
    let middle = screen.center().x;
    let mut pick = None;
    const TAB_H: f32 = 30.0;
    const DROP: f32 = 12.0;
    const HOVER_DROP: f32 = 8.0;
    const GAP: f32 = 4.0;
    const PLUS: f32 = 22.0;
    const SIDE: f32 = 3.0;
    let detached = rt.detached.is_some();

    let order = scene_order(project);
    let mut tabs: Vec<(Option<u64>, Option<usize>, String, Option<f64>)> = vec![(None, None, tr("Grundszene", "Base scene").to_string(), None)];
    for &i in &order {
        let sc = &project.scenes[i];
        let label = match sc.cue_id {
            Some(c) => format!("{}  ·  {}", sc.name, fmt_cue(c)),
            None => sc.name.clone(),
        };
        tabs.push((Some(sc.id), Some(i), label, sc.cue_id));
    }
    let n = tabs.len();
    let current = tabs.iter().position(|tab| tab.0 == rt.active).unwrap_or(0);

    if step_left && current > 0 {
        pick = Some(ScenePick::Show(tabs[current - 1].0));
    } else if step_right && current + 1 < n {
        pick = Some(ScenePick::Show(tabs[current + 1].0));
    }

    // The strip of tabs, placed so that the (gliding) current position sits in the middle.
    let font = egui::FontId::proportional(14.0);
    let widths: Vec<f32> = tabs.iter().map(|tab| (ctx.fonts_mut(|f| f.layout_no_wrap(tab.2.clone(), font.clone(), egui::Color32::WHITE).size().x) + 28.0).max(70.0)).collect();
    let mut strip = vec![0.0f32; n];
    for k in 1..n {
        strip[k] = strip[k - 1] + widths[k - 1] + GAP;
    }
    let centre = |k: usize| strip[k] + widths[k] / 2.0;
    let pos = glide(ctx, id.with("glide"), current as f32, t > 0.05 || peeking);
    let (a, b) = (pos.floor() as usize, (pos.ceil() as usize).min(n - 1));
    let at = centre(a) + (centre(b) - centre(a)) * (pos - pos.floor());
    let shift = middle - at;
    // How far a tab comes down at distance `d` (in tabs) from the current position.
    let current_reach = TAB_H + DROP + HOVER_DROP * t;
    let reach = |d: f32| -> f32 {
        let side = TAB_H * t * (1.0 - 0.15 * (d - 1.0).max(0.0));
        let r = if d < 1.0 { current_reach + (side - current_reach) * d } else { side };
        // Three on either side; the fourth only fades in while sliding.
        r * (SIDE + 1.0 - d).clamp(0.0, 1.0)
    };
    let visible = |rect: egui::Rect| rect.intersect(egui::Rect::from_min_max(egui::pos2(rect.left(), 0.0), rect.max)).intersect(screen);
    let places: Vec<Option<(egui::Rect, egui::Rect)>> = (0..n)
        .map(|k| {
            let d = (k as f32 - pos).abs();
            let r = reach(d);
            let full = current_reach.max(TAB_H);
            let rect = egui::Rect::from_min_size(egui::pos2(strip[k] + shift, r - full), egui::vec2(widths[k], full));
            (r >= 2.0 && visible(rect).is_positive()).then(|| (rect, visible(rect)))
        })
        .collect();
    // "+" beside the current scene, below its neighbours.
    let plus_y = TAB_H * t + 3.0;
    let cur_rect = egui::Rect::from_min_size(egui::pos2(strip[current] + shift, 0.0), egui::vec2(widths[current], 1.0));
    let plus_left = (current > 0 && t > 0.3).then(|| egui::Rect::from_min_size(egui::pos2(cur_rect.left() - GAP - PLUS, plus_y), egui::vec2(PLUS, PLUS)));
    let plus_right = (t > 0.3).then(|| egui::Rect::from_min_size(egui::pos2(cur_rect.right() + GAP, plus_y), egui::vec2(PLUS, PLUS)));
    let line_y = current_reach + 6.0;
    // Chosen by hand (it shows amber): while out, a button under it to follow BK again.
    let follow = (detached && t > 0.5).then(|| egui::Rect::from_center_size(egui::pos2(middle, line_y + 11.0), egui::vec2(150.0, 22.0)));
    let span = places
        .iter()
        .flatten()
        .map(|p| p.1)
        .chain(plus_left)
        .chain(plus_right)
        .chain(follow)
        .fold(egui::Rect::NOTHING, |a, b| a.union(b));
    if !span.is_positive() {
        return (pick, egui::Rect::NOTHING);
    }

    let mut add: Option<bool> = None;
    let mut covered = egui::Rect::NOTHING;
    egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(span.min).constrain(false).show(ctx, |ui| {
        ui.allocate_exact_size(span.size(), egui::Sense::hover());
        ui.set_clip_rect(screen);
        let visuals = ui.visuals().clone();
        let mut hint: Option<(egui::Rect, String)> = None;
        for (k, (scene, _, label, cue)) in tabs.iter().enumerate() {
            let Some((rect, shown)) = places[k] else { continue };
            let active = *scene == rt.active;
            let resp = ui.interact(shown, id.with(("tab", k)), egui::Sense::click());
            let fill = if active {
                if detached { ACCENT } else { egui::Color32::from_rgb(61, 139, 255) }
            } else if resp.hovered() {
                visuals.widgets.hovered.bg_fill
            } else {
                visuals.window_fill
            };
            ui.painter().rect(rect, egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, fill, visuals.window_stroke, egui::StrokeKind::Inside);
            let text_color = if active { egui::Color32::BLACK } else { visuals.text_color() };
            ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - TAB_H / 2.0), egui::Align2::CENTER_CENTER, label, font.clone(), text_color);
            covered = covered.union(shown);
            if resp.hovered() {
                let text = match cue {
                    Some(c) => trf!("ab Cue {} der Main-Cuelist", "from cue {} of the main cuelist", fmt_cue(*c)),
                    None if scene.is_none() => tr("die Dateien, wie sie sind", "the files as they are").to_string(),
                    None => tr("ohne Cue – nur von Hand", "no cue – by hand only").to_string(),
                };
                hint = Some((rect, text));
            }
            if resp.clicked() {
                pick = Some(ScenePick::Show(*scene));
            }
            if resp.secondary_clicked()
                && let Some(sid) = scene
            {
                ctx.data_mut(|d| d.insert_temp(editing_id, *sid));
            }
        }
        for (rect, before) in [(plus_left, true), (plus_right, false)].into_iter().filter_map(|(r, b)| r.map(|r| (r, b))) {
            let resp = ui.interact(rect, id.with(("plus", before)), egui::Sense::click());
            let fill = if resp.hovered() { visuals.widgets.hovered.bg_fill } else { visuals.window_fill };
            ui.painter().rect(rect, egui::CornerRadius::same(3), fill, visuals.window_stroke, egui::StrokeKind::Inside);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", font.clone(), visuals.text_color());
            covered = covered.union(rect);
            let text = if before { tr("Neue Szene davor", "New scene before") } else { tr("Neue Szene danach", "New scene after") };
            if resp.hovered() {
                hint = Some((rect, text.to_string()));
            }
            if resp.clicked() {
                add = Some(before);
            }
        }
        // A hint centred under what the mouse is on.
        if let Some((rect, text)) = hint {
            let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(12.0), visuals.text_color());
            let size = galley.size() + egui::vec2(14.0, 8.0);
            // Under the follow button when that is there, below the current scene.
            let top = follow.filter(|f| (f.center().x - rect.center().x).abs() < 1.0).map_or(rect.bottom(), |f| f.bottom());
            let below = egui::Rect::from_center_size(egui::pos2(rect.center().x, top + 6.0 + size.y / 2.0), size);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("hint")));
            painter.rect(below, egui::CornerRadius::same(4), visuals.window_fill, visuals.window_stroke, egui::StrokeKind::Inside);
            painter.galley(below.min + egui::vec2(7.0, 4.0), galley, visuals.text_color());
        }
        if let Some(rect) = follow {
            let resp = ui.put(rect, egui::Button::new(tr("BlinderKitten folgen", "Follow BlinderKitten")).small());
            covered = covered.union(resp.rect);
            if resp.on_hover_text(tr("Die Szene wieder von der Main-Cuelist bestimmen lassen (sonst ab dem nächsten Cue).", "Let the main cuelist choose the scene again (else from the next cue).")).clicked() {
                pick = Some(ScenePick::Follow);
            }
        } else if rt.moving() && t > 0.5 {
            ui.painter().text(egui::pos2(middle, line_y), egui::Align2::CENTER_TOP, tr("Übergang läuft …", "Transition running …"), egui::FontId::proportional(12.0), egui::Color32::LIGHT_YELLOW);
        }
    });

    // A new scene beside the current one: right after it in the order, or right before it.
    if let Some(before) = add {
        let sid = project.scenes.iter().map(|s| s.id).max().map_or(1, |m| m + 1);
        let scene = tapetum_project::Scene { id: sid, name: trf!("Szene {id}", "Scene {id}", id = sid), cue_id: None, ..Default::default() };
        // Without a cue it stays behind the scene before it in the project (see `scene_order`).
        let at = match (before, tabs[current].1) {
            (false, Some(i)) => i + 1,
            (false, None) => 0,
            (true, Some(i)) => order.iter().position(|&o| o == i).and_then(|p| p.checked_sub(1)).map_or(0, |p| order[p] + 1),
            (true, None) => 0,
        };
        project.scenes.insert(at.min(project.scenes.len()), scene);
        pick = Some(ScenePick::Show(Some(sid)));
    }
    let edit = scene_editor(ctx, state, project, editing);
    if let Some(sid) = edit {
        project.scenes.retain(|s| s.id != sid);
        pick = Some(ScenePick::Show(None));
    }
    let covered = if covered.is_positive() { covered } else { egui::Rect::NOTHING };
    remember(ctx, id, covered);
    (pick, covered)
}

/// The window for editing a scene (right click on its bookmark): name, the
/// cue it starts at, transition. Returns a scene to delete.
fn scene_editor(ctx: &egui::Context, state: &ShowState, project: &mut Project, editing: Option<u64>) -> Option<u64> {
    let editing_id = egui::Id::new("scene_bar").with("editing");
    let sid = editing?;
    let i = project.scenes.iter().position(|s| s.id == sid)?;
    let cues: Vec<(f64, String)> = state.show.conductor().map(|c| c.cues.iter().map(|q| (q.id, q.name.clone())).collect()).unwrap_or_default();
    let mut open = true;
    let mut delete = None;
    egui::Window::new(tr("Szene bearbeiten", "Edit scene"))
        .id(egui::Id::new("scene_editor"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
        .show(ctx, |ui| {
            let scene = &mut project.scenes[i];
            egui::Grid::new("scene_editor_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr("Name", "Name"));
                ui.add(egui::TextEdit::singleline(&mut scene.name).desired_width(220.0));
                ui.end_row();
                ui.label(tr("Ab Cue", "From cue"));
                let cue_text = scene.cue_id.map_or(tr("keine (nur von Hand)", "none (by hand only)").to_string(), |c| {
                    let n = cues.iter().find(|(q, _)| (q - c).abs() < 1e-6).map_or(String::new(), |(_, n)| format!(" – {n}"));
                    trf!("Cue {}{n}", "Cue {}{n}", fmt_cue(c), n = n)
                });
                egui::ComboBox::from_id_salt(("cue", sid)).selected_text(cue_text).width(220.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut scene.cue_id, None, tr("keine (nur von Hand)", "none (by hand only)"));
                    for (q, n) in &cues {
                        ui.selectable_value(&mut scene.cue_id, Some(*q), trf!("Cue {} – {n}", "Cue {} – {n}", fmt_cue(*q), n = n));
                    }
                });
                ui.end_row();
                ui.label(tr("Übergang", "Transition"));
                ui.add(egui::DragValue::new(&mut scene.transition).range(0.0..=60.0).speed(0.1).suffix(" s"));
                ui.end_row();
            });
            ui.colored_label(egui::Color32::GRAY, tr("Die Szenen ordnen sich nach ihren Cues, wie in BlinderKitten. Objekte der Szene: Szene wählen, dann im 3D-Fenster verschieben, drehen oder ausblenden.", "The scenes line up by their cues, as in BlinderKitten. The scene's objects: choose the scene, then move, turn or hide them in the 3D view."));
            ui.horizontal(|ui| {
                if ui.button(tr("Fertig", "Done")).clicked() {
                    ctx.data_mut(|d| d.remove::<u64>(editing_id));
                }
                if ui.button(tr("Szene löschen", "Delete scene")).clicked() {
                    delete = Some(sid);
                    ctx.data_mut(|d| d.remove::<u64>(editing_id));
                }
            });
        });
    if !open {
        ctx.data_mut(|d| d.remove::<u64>(editing_id));
    }
    delete
}
