use bevy::prelude::*;
use crate::i18n::tr;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use bevy_egui::{EguiContexts, EguiTextureHandle, egui};
use tapetum_project::{FixtureOverride, Quality};
use bk_olga::InterfaceKind;

use super::gizmo::GizmoState;
use super::scene::View3d;
use super::objects::{SceneRuntime, StageObjects, file_color, file_reflectance, is_igz, object_key, split_key, stored_path};
use super::undo::{Step, UndoHistory};
use super::state::{BkLink, BkStatus, Dmx, GoboImport, ObjectImport, PlacementSync, RangeSync, ShowState};
use super::type_editor;
use crate::bk_link::{Edit, HostInfo};
use crate::routing::format_port_address;

static HIDE_SIDEBAR_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Start with the sidebar hidden (for screenshots).
pub fn hide_sidebar_at_start() {
    HIDE_SIDEBAR_AT_START.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn panel(
    mut contexts: EguiContexts,
    mut state: ResMut<ShowState>,
    mut dmx: ResMut<Dmx>,
    mut link: ResMut<BkLink>,
    bk_status: Res<BkStatus>,
    (mut sync, mut undo): (ResMut<RangeSync>, ResMut<UndoHistory>),
    mut placement_sync: ResMut<PlacementSync>,
    mut gizmo_state: ResMut<GizmoState>,
    (mut object_import, stage_objects, scene_rt, spacemouse): (ResMut<ObjectImport>, Res<StageObjects>, Res<SceneRuntime>, Res<super::spacemouse::SpaceMouse>),
    mut import: ResMut<GoboImport>,
    view: Res<View3d>,
    mut ui_rects: ResMut<UiRects>,
    time: Res<Time>,
    mut fps: Local<f32>,
    (mut sidebar_hidden, mut slow_since): (Local<bool>, Local<Option<f32>>),
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) -> Result {
    link.poll();
    let bk = bk_status.info();
    let mut setup_action = None;
    let dt = time.delta_secs().max(1e-4);
    *fps = if *fps == 0.0 { 1.0 / dt } else { *fps * 0.95 + 0.05 / dt };
    // Seconds spent below 25 fps in a row.
    let slow = slow_since.get_or_insert(0.0);
    *slow = if *fps < 25.0 { *slow + dt } else { 0.0 };
    let view_texture = contexts.add_image(EguiTextureHandle::Weak(view.image.id()));
    let logo_texture = contexts.add_image(EguiTextureHandle::Weak(view.logo.id()));
    let ctx = contexts.ctx_mut()?;
    super::about::window(ctx, logo_texture);
    // The 3D view, stretched over the whole window underneath the panels.
    ctx.layer_painter(egui::LayerId::background()).image(
        view_texture,
        ctx.viewport_rect(),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    // Tab hides the sidebar, F11 toggles fullscreen, Cmd/Ctrl+Z undoes; not
    // while typing into a field (it has its own undo there).
    if !ctx.egui_wants_keyboard_input() {
        let (redo, undo_key) = ctx.input(|i| {
            let z = i.modifiers.command && i.key_pressed(egui::Key::Z);
            (z && i.modifiers.shift || i.modifiers.ctrl && i.key_pressed(egui::Key::Y), z && !i.modifiers.shift)
        });
        if redo {
            undo.request = Some(Step::Redo);
        } else if undo_key {
            undo.request = Some(Step::Undo);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            *sidebar_hidden = !*sidebar_hidden;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F11))
            && let Ok(mut window) = windows.single_mut()
        {
            window.mode = match window.mode {
                WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
                _ => WindowMode::Windowed,
            };
        }
    }

    // Edit a copy so change detection on `ShowState` only fires on real edits.
    let mut project = state.project.clone();
    let selected = state.selected;
    let mut pick: Option<Pick> = None;
    let mut brush = state.led_brush;
    let mut manual_scene = state.manual_scene;
    if let Some(file) = object_import.poll() {
        let (unit_scale, up) = crate::stage::defaults_for(&file);
        let id = project.objects.iter().map(|o| o.id).max().map_or(1, |m| m + 1);
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| trf!("Datei {id}", "File {id}", id = id));
        project.objects.push(tapetum_project::StageObject { id, name, model: stored_path(&state, &file), unit_scale, up, ..Default::default() });
    }
    match import.poll(&state) {
        Some((ty, slot, Ok(rel))) if slot == type_editor::FIXED_GOBO => {
            project.fixture_types.entry(ty).or_default().fixed_gobo = Some(tapetum_project::Gobo::Image(rel));
        }
        Some((ty, slot, Ok(rel))) => {
            if let Some(s) = project.fixture_types.get_mut(&ty).and_then(|o| o.gobo_wheel.as_mut()).and_then(|w| w.get_mut(slot)) {
                s.gobo = tapetum_project::Gobo::Image(rel);
            }
        }
        Some((_, _, Err(e))) => state.status = Some(trf!("Gobo-Bild: {e}", "Gobo picture: {e}", e = e)),
        None => {}
    }

    let mut root = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    let mut rects = Vec::new();
    let mut hide = false;
    if HIDE_SIDEBAR_AT_START.swap(false, std::sync::atomic::Ordering::Relaxed) {
        *sidebar_hidden = true;
    }
    if *sidebar_hidden {
        // Just a small handle to bring the sidebar back.
        let handle = egui::Area::new("sidebar_handle".into()).fixed_pos(egui::pos2(8.0, 8.0)).show(ctx, |ui| {
            if ui.button("»").on_hover_text(tr("Seitenleiste einblenden (Tab)", "Show the sidebar (Tab)")).clicked() {
                *sidebar_hidden = false;
            }
        });
        rects.push(handle.response.rect);
    } else {
        let left = egui::Panel::left("panel").resizable(true).default_size(310.0).show(&mut root, |ui| {
            // The header stays put; everything below scrolls.
            ui.horizontal(|ui| {
                ui.heading("Tapetum");
                ui.weak(format!("{:.0} fps", *fps));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    super::about::menu(ui);
                    if ui.small_button("«").on_hover_text(tr("Seitenleiste ausblenden (Tab)\nVollbild: F11", "Hide the sidebar (Tab)\nFull screen: F11")).clicked() {
                        hide = true;
                    }
                    let cmd = if cfg!(target_os = "macos") { "Cmd" } else { tr("Strg", "Ctrl") };
                    if ui.add_enabled(undo.can_redo(&state.project), egui::Button::new("⟳").small()).on_hover_text(trf!("Wiederholen ({cmd}+Shift+Z)", "Redo ({cmd}+Shift+Z)", cmd = cmd)).clicked() {
                        undo.request = Some(Step::Redo);
                    }
                    if ui.add_enabled(undo.can_undo(&state.project), egui::Button::new("⟲").small()).on_hover_text(trf!("Rückgängig ({cmd}+Z)", "Undo ({cmd}+Z)", cmd = cmd)).clicked() {
                        undo.request = Some(Step::Undo);
                    }
                });
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(state.show_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                if let Some(s) = &state.status {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, s);
                }
                for w in &state.show.warnings {
                    ui.colored_label(egui::Color32::LIGHT_RED, w);
                }

                ui.separator();
                ui.strong("Art-Net");
                ui.label(trf!("Empfang auf {}", "Receiving on {}", dmx.receiver.local_addr));
                let mut universes: Vec<_> = dmx.receiver.store.snapshot().into_iter().collect();
                universes.sort_by_key(|(pa, _)| *pa);
                if universes.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, tr("Kein Signal von BlinderKitten.", "No signal from BlinderKitten."));
                }
                let mut receiving = false;
                for (pa, u) in universes {
                    let age = u.last_seen.elapsed().as_secs_f32();
                    receiving |= age < 2.0;
                    let color = if age < 1.0 { egui::Color32::LIGHT_GREEN } else { egui::Color32::GRAY };
                    ui.colored_label(color, trf!("Universe {}  ·  {}  ·  vor {:.1}s", "Universe {}  ·  {}  ·  {:.1}s ago", format_port_address(pa), u.source, age));
                }
                ui.add_space(4.0);
                setup_action = setup_section(ui, &state, &link, bk.as_ref(), dmx.receiver.local_addr.port(), receiving);

                ui.separator();
                ui.strong(tr("Darstellung", "Display"));
                let r = &mut project.render;
                egui::ComboBox::from_label(tr("Qualität", "Quality"))
                    .selected_text(quality_label(r.quality))
                    .show_ui(ui, |ui| {
                        for q in [Quality::Low, Quality::Medium, Quality::High] {
                            ui.selectable_value(&mut r.quality, q, quality_label(q)).on_hover_text(quality_hint(q));
                        }
                    })
                    .response
                    .on_hover_text(quality_hint(r.quality));
                // Running slowly for a while: offer the next lower quality.
                if *slow_since.get_or_insert(0.0) > 5.0 && r.quality != Quality::Low {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(egui::Color32::LIGHT_YELLOW, trf!("Läuft langsam ({:.0} fps).", "Running slowly ({:.0} fps).", *fps));
                        let lower = if r.quality == Quality::High { Quality::Medium } else { Quality::Low };
                        if ui.small_button(trf!("Auf „{}“ stellen", "Switch to “{}”", quality_label(lower))).clicked() {
                            r.quality = lower;
                            *slow_since = Some(0.0);
                        }
                    });
                }
                let res_label = |v: Option<f32>| match v {
                    None => tr("Automatisch", "Automatic").to_string(),
                    Some(f) => format!("{:.0} %", f * 100.0),
                };
                egui::ComboBox::from_label(tr("3D-Auflösung", "3D resolution"))
                    .selected_text(res_label(r.resolution))
                    .show_ui(ui, |ui| {
                        for v in [None, Some(0.5), Some(0.75), Some(1.0)] {
                            ui.selectable_value(&mut r.resolution, v, res_label(v));
                        }
                    })
                    .response
                    .on_hover_text(tr("Auflösung der 3D-Ansicht (Haze-Berechnung) im Verhältnis zu den Bildschirmpixeln. Automatisch: halbe Auflösung auf Retina-Displays. Die Bedienoberfläche bleibt immer scharf.", "Resolution of the 3D view (haze computation) relative to the screen pixels. Automatic: half resolution on Retina displays. The user interface always stays sharp."));
                ui.add(egui::Slider::new(&mut r.exposure_ev, 0.0..=12.0).text(tr("Belichtung (EV)", "Exposure (EV)")).step_by(0.1));
                ui.add(egui::Slider::new(&mut r.ambient, 0.0..=1.0).text(tr("Saallicht", "House light")));
                ui.add_enabled(r.quality != Quality::Low, egui::Checkbox::new(&mut r.bloom, "Bloom"))
                    .on_disabled_hover_text(tr("Auf „Niedrig“ aus, um schwache Grafik zu entlasten.", "Off on “Low” to spare weak graphics."));
                ui.checkbox(&mut r.haze.enabled, "Haze");
                ui.add_enabled_ui(r.haze.enabled, |ui| {
                    ui.add(egui::Slider::new(&mut r.haze.density, 0.0..=0.5).text(tr("Dichte", "Density")));
                    ui.add(egui::Slider::new(&mut r.haze.anisotropy, -0.9..=0.95).text(tr("Streuung vorwärts", "Forward scattering")));
                    ui.horizontal(|ui| {
                        ui.label(tr("Farbe", "Colour"));
                        ui.color_edit_button_rgb(&mut r.haze.color);
                    });
                });
                spacemouse_section(ui, &mut project.spacemouse, spacemouse.device());

                ui.separator();
                scenes_section(ui, &state, &mut project, &scene_rt, &mut manual_scene);
                ui.separator();
                if let Some(p) = objects_section(ui, &state, &mut project, &stage_objects, &mut object_import) {
                    pick = Some(p);
                }

                if !state.show.groups.is_empty() {
                    ui.separator();
                    ui.strong(tr("Gruppen", "Groups"));
                    let (shift, cmd) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
                    for (gi, g) in state.show.groups.iter().enumerate() {
                        let text = format!("{:>3}  {}  ·  {}", g.id, g.name, devices(g.fixtures.len()));
                        if ui.selectable_label(state.selection_group == Some(gi), text).clicked() {
                            pick = Some(match (shift, cmd, state.anchor_group) {
                                (true, _, Some(a)) => Pick::GroupRange(a, gi),
                                (_, true, _) => Pick::AddGroup(gi, None),
                                _ if state.selection_group == Some(gi) => Pick::Clear,
                                _ => Pick::Group(gi),
                            });
                        }
                    }
                }

                ui.separator();
                ui.strong("Fixtures");
                let (shift, cmd) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
                for (i, f) in state.show.fixtures.iter().enumerate() {
                    let class = state.specs.get(i).and_then(|s| s.as_ref()).map_or("?", |s| s.class.label());
                    let text = format!("{:>3}  {}  ·  {}", f.id, f.name, class);
                    if ui.selectable_label(state.selection.contains(&i), text).clicked() {
                        pick = Some(match (shift, cmd, state.anchor_fixture) {
                            (true, _, Some(a)) => Pick::FixtureRange(a, i),
                            (_, true, _) => Pick::Toggle(i),
                            _ if state.selection == [i] => Pick::Clear,
                            _ => Pick::Fixture(i),
                        });
                    }
                }
                ui.colored_label(
                    egui::Color32::GRAY,
                    tr("Listen: Shift wählt einen Bereich, Cmd/Strg ergänzt einzeln. Im 3D-Fenster: Klick wählt ein Gerät, Rechtsklick seine Gruppe, Shift oder Cmd/Strg ergänzt.", "Lists: Shift selects a range, Cmd/Ctrl adds single entries. In the 3D view: a click selects a fixture, a right click its group, Shift or Cmd/Ctrl adds."),
                );

            });
        });

        rects.push(left.response.rect);
    }
    if hide {
        *sidebar_hidden = true;
    }

    // Right click on a fixture in several groups: which one, next to the mouse.
    if let Some(menu) = gizmo_state.menu.clone()
        && let Some(f) = state.show.fixtures.get(menu.fixture)
    {
        let area = egui::Area::new("group_menu".into()).fixed_pos(egui::pos2(menu.pos.x + 12.0, menu.pos.y)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.strong(format!("{} – {}", f.id, f.name));
                for gi in state.groups_of(menu.fixture) {
                    let g = &state.show.groups[gi];
                    if ui.button(trf!("Gruppe {} ({})", "Group {} ({})", g.name, devices(g.fixtures.len()))).clicked() {
                        pick = Some(if menu.add { Pick::AddGroup(gi, Some(menu.fixture)) } else { Pick::GroupWith(gi, menu.fixture) });
                    }
                }
            });
        });
        rects.push(area.response.rect);
        if pick.is_some() {
            gizmo_state.menu = None;
        }
    }

    // A selected stage object gets its editor on the right.
    let mut close = false;
    if let Some(key) = state.selected_object.clone()
        && split_key(&key).is_some_and(|(f, _)| project.objects.iter().any(|o| o.id == f))
    {
        let right = egui::Panel::right("object").resizable(true).default_size(340.0).show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if object_editor(ui, &state, &mut project, &key, &stage_objects, &scene_rt) {
                    close = true;
                }
            });
        });
        rects.push(right.response.rect);
    }

    // Settings of the selected fixture's type, in their own panel on the right.
    if !state.selection.is_empty() {
        let right = egui::Panel::right("fixture").resizable(true).default_size(340.0).show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if state.selection.len() > 1 {
                    ui.horizontal(|ui| {
                        let title = match state.selection_group.and_then(|g| state.show.groups.get(g)) {
                            Some(g) => trf!("Gruppe {} – {}", "Group {} – {}", g.id, g.name),
                            None => trf!("{} Geräte", "{} fixtures", state.selection.len()),
                        };
                        ui.heading(title);
                        if ui.small_button("✖").on_hover_text(tr("Auswahl aufheben", "Clear selection")).clicked() {
                            close = true;
                        }
                    });
                    let names: Vec<String> = state.selection.iter().filter_map(|&i| state.show.fixtures.get(i)).map(|f| format!("{} {}", f.id, f.name)).collect();
                    ui.label(names.join(", "));
                    group_placement_editor(ui, &state, &mut project, &mut placement_sync);
                    ui.separator();
                }
                let same_type = selected.is_some_and(|p| {
                    let ty = state.show.fixtures[p].fixture_type;
                    state.selection.iter().all(|&i| state.show.fixtures[i].fixture_type == ty)
                });
                if !same_type {
                    ui.colored_label(egui::Color32::GRAY, tr("Verschiedene Gerätetypen: für Typ-Einstellungen ein einzelnes Gerät wählen.", "Different fixture types: select a single fixture for type settings."));
                }
                if let Some(i) = selected.filter(|_| same_type)
                    && let Some(f) = state.show.fixtures.get(i)
                    && let Some(ty) = state.show.fixture_type(f)
                {
                    if state.selection.len() == 1 {
                        ui.horizontal(|ui| {
                            ui.heading(format!("{} – {}", f.id, f.name));
                            if ui.small_button("✖").on_hover_text(tr("Schließen", "Close")).clicked() {
                                close = true;
                            }
                        });
                        placement_editor(ui, &state, &mut project, i, &mut placement_sync, bk.as_ref());
                        ui.separator();
                    }
                    let devices = state.show.fixtures.iter().filter(|g| g.fixture_type == f.fixture_type).count();
                    let entry = project.fixture_types.entry(ty.short_name.clone()).or_default();
                    let mut p = type_editor::Painter { brush };
                    let ev = type_editor::show(ui, ty, devices, entry, &mut p);
                    brush = p.brush;
                    for (attr, range) in ev.ranges {
                        sync.queue(&ty.short_name, attr, range);
                    }
                    if let Some(slot) = ev.import_gobo {
                        import.open(ty.short_name.clone(), slot);
                    }
                    ui.add_space(6.0);
                    if ui.button(tr("Alle Einstellungen dieses Typs zurücksetzen", "Reset all settings of this type")).clicked() {
                        project.fixture_types.remove(&ty.short_name);
                        let ids: Vec<i64> = state.show.fixtures.iter().filter(|g| g.fixture_type == f.fixture_type).map(|g| g.id).collect();
                        project.fixtures.retain(|id, _| !ids.contains(id));
                    }
                }
            });
        });
        rects.push(right.response.rect);
    }
    if ui_rects.0 != rects {
        ui_rects.0 = rects;
    }
    if close {
        pick = Some(Pick::Clear);
    }

    match setup_action {
        Some(SetupAction::Edit(edit, offline)) => link.start(&state, edit, offline),
        Some(SetupAction::ChangePort(port)) => match dmx.restart(port) {
            Ok(()) => {
                project.artnet.listen_port = port;
                // Existing mirrors must send to the new port.
                if state.show.interfaces.iter().any(|i| bk_olga::mirror::is_mirror_name(&i.name)) {
                    link.start(&state, Edit::AddMirrors { listen_port: port }, false);
                }
            }
            Err(e) => state.status = Some(trf!("Port {port} ist belegt: {e}", "Port {port} is in use: {e}", port = port, e = e)),
        },
        Some(SetupAction::RemotePort(port)) => {
            project.artnet.bk_remote_port = port;
            bk_status.set_port(port);
        }
        None => {}
    }

    // Viewing a fixture must not leave empty entries behind.
    project.fixtures.retain(|_, o| *o != FixtureOverride::default());
    project.fixture_types.retain(|_, o| *o != FixtureOverride::default());
    match pick {
        Some(Pick::Fixture(i)) => {
            state.select_fixture(i);
            state.anchor_fixture = Some(i);
        }
        Some(Pick::FixtureRange(a, b)) => {
            let n = state.show.fixtures.len();
            let range: Vec<usize> = (a.min(b)..=a.max(b)).filter(|&i| i < n).collect();
            state.select_fixtures(range, b);
        }
        Some(Pick::Group(g)) => {
            state.select_group(g, None);
            state.anchor_group = Some(g);
        }
        Some(Pick::GroupRange(a, b)) => {
            let range: Vec<usize> = (a.min(b)..=a.max(b)).filter(|&g| g < state.show.groups.len()).collect();
            state.select_groups(&range, b);
        }
        Some(Pick::GroupWith(g, i)) => state.select_group(g, Some(i)),
        Some(Pick::AddGroup(g, i)) => {
            state.add_group(g, i);
            state.anchor_group = Some(g);
        }
        Some(Pick::Toggle(i)) => {
            state.toggle_fixture(i);
            state.anchor_fixture = Some(i);
        }
        Some(Pick::Object(key)) => {
            state.anchor_object = Some(key.clone());
            state.select_object(key);
        }
        Some(Pick::ToggleObject(key)) => {
            state.anchor_object = Some(key.clone());
            state.toggle_object(key);
        }
        Some(Pick::ObjectRange(a, b)) => {
            // In the order the panel lists them: files, then their objects.
            let order: Vec<String> = state.project.objects.iter().flat_map(|f| stage_objects.objects(f.id).iter().map(|o| object_key(f.id, &o.key))).collect();
            if let (Some(ia), Some(ib)) = (order.iter().position(|k| *k == a), order.iter().position(|k| *k == b)) {
                let range = order[ia.min(ib)..=ia.max(ib)].to_vec();
                state.select_objects(range, b);
            }
        }
        Some(Pick::Clear) => state.clear_selection(),
        None => {}
    }
    if brush != state.led_brush {
        state.led_brush = brush;
    }
    if manual_scene != state.manual_scene {
        state.manual_scene = manual_scene;
    }
    if project != state.project {
        // Fixture models depend on these; render settings apply live.
        let rebuild = project.render.quality != state.project.render.quality
            || project.fixtures != state.project.fixtures
            || project.fixture_types != state.project.fixture_types;
        state.project = project;
        if rebuild {
            state.generation += 1;
        }
        state.save_project();
    }
    Ok(())
}

/// Scenes: the base scene (the files as they are) and scenes bound to cues
/// of the main cuelist. While that cuelist runs in BK it picks the scene.
fn scenes_section(ui: &mut egui::Ui, state: &ShowState, project: &mut tapetum_project::Project, rt: &SceneRuntime, manual: &mut Option<u64>) {
    ui.horizontal(|ui| {
        ui.strong(tr("Szenen", "Scenes"));
        if ui.button(tr("+ Szene", "+ Scene")).on_hover_text(tr("Neue Szene; zeigt zunächst die Grundszene. Objekte darin verschieben, drehen oder ausblenden.", "New scene; starts out as the base scene. Move, turn or hide objects in it.")).clicked() {
            let id = project.scenes.iter().map(|s| s.id).max().map_or(1, |m| m + 1);
            project.scenes.push(tapetum_project::Scene { id, name: trf!("Szene {id}", "Scene {id}", id = id), cue_id: rt.following_cue, ..Default::default() });
            *manual = Some(id);
        }
    });
    match rt.following_cue {
        Some(cue) => ui.colored_label(egui::Color32::LIGHT_GREEN, trf!("Folgt der Main-Cuelist (Cue {})", "Follows the main cuelist (cue {})", fmt_cue(cue))),
        None => ui.colored_label(egui::Color32::GRAY, tr("Main-Cuelist läuft nicht – Szene hier wählen.", "The main cuelist isn't running – choose the scene here.")),
    };
    if rt.moving() {
        ui.colored_label(egui::Color32::LIGHT_YELLOW, tr("Übergang läuft …", "Transition running …"));
    }
    if ui.selectable_label(rt.active.is_none(), tr("Grundszene (wie in den Dateien)", "Base scene (as in the files)")).clicked() {
        *manual = None;
    }
    let mut order: Vec<usize> = (0..project.scenes.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&project.scenes[a], &project.scenes[b]);
        sa.cue_id.unwrap_or(f64::MAX).total_cmp(&sb.cue_id.unwrap_or(f64::MAX)).then(sa.id.cmp(&sb.id))
    });
    let mut delete = None;
    let cues: Vec<(f64, String)> = state.show.conductor().map(|c| c.cues.iter().map(|q| (q.id, q.name.clone())).collect()).unwrap_or_default();
    for i in order {
        let id = project.scenes[i].id;
        let active = rt.active == Some(id);
        let label = match project.scenes[i].cue_id {
            Some(c) => trf!("{}  ·  ab Cue {}", "{}  ·  from cue {}", project.scenes[i].name, fmt_cue(c)),
            None => trf!("{}  ·  ohne Cue", "{}  ·  no cue", project.scenes[i].name),
        };
        if ui.selectable_label(active, label).clicked() {
            *manual = Some(id);
        }
        if !active {
            continue;
        }
        // The active scene is the one being edited.
        let scene = &mut project.scenes[i];
        ui.indent(("scene", id), |ui| {
            ui.add(egui::TextEdit::singleline(&mut scene.name).desired_width(200.0));
            let cue_text = scene.cue_id.map_or(tr("keine (nur von Hand)", "none (manual only)").to_string(), |c| {
                let name = cues.iter().find(|(q, _)| (q - c).abs() < 1e-6).map_or(String::new(), |(_, n)| format!(" – {n}"));
                trf!("Cue {}{name}", "Cue {}{name}", fmt_cue(c), name = name)
            });
            egui::ComboBox::from_id_salt(("cue", id)).selected_text(cue_text).width(220.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut scene.cue_id, None, tr("keine (nur von Hand)", "none (manual only)"));
                for (q, n) in &cues {
                    ui.selectable_value(&mut scene.cue_id, Some(*q), trf!("Cue {} – {n}", "Cue {} – {n}", fmt_cue(*q), n = n));
                }
            });
            ui.add(egui::DragValue::new(&mut scene.transition).range(0.0..=60.0).speed(0.1).suffix(" s").prefix(tr("Übergang ", "Transition ")));
            if ui.small_button(tr("Szene löschen", "Delete scene")).clicked() {
                delete = Some(id);
            }
        });
    }
    if let Some(id) = delete {
        project.scenes.retain(|s| s.id != id);
        if *manual == Some(id) {
            *manual = None;
        }
    }
}

fn fmt_cue(c: f64) -> String {
    if c.fract().abs() < 1e-9 { format!("{c:.0}") } else { format!("{c}") }
}

/// Model files and the objects in them.
fn objects_section(ui: &mut egui::Ui, state: &ShowState, project: &mut tapetum_project::Project, stage: &StageObjects, import: &mut ObjectImport) -> Option<Pick> {
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.strong(tr("Bühnenobjekte", "Stage objects"));
        let label = if import.busy() { tr("Datei wird gewählt …", "Choosing a file …") } else { tr("Importieren …", "Import …") };
        if ui
            .add_enabled(!import.busy(), egui::Button::new(label))
            .on_hover_text(tr("IngeTrazo (.igz), glTF/GLB, OBJ oder STL. Änderungen an der Datei werden automatisch übernommen.", "IngeTrazo (.igz), glTF/GLB, OBJ or STL. Changes to the file are picked up automatically."))
            .clicked()
        {
            import.open();
        }
    });
    for file in &mut project.objects {
        let status = if stage.loading(file.id) {
            tr(" · lädt …", " · loading …")
        } else if stage.errors.contains_key(&file.id) {
            tr(" · Fehler", " · error")
        } else {
            ""
        };
        ui.horizontal(|ui| {
            ui.checkbox(&mut file.visible, "").on_hover_text(tr("Datei anzeigen", "Show file"));
            ui.label(egui::RichText::new(format!("{}{status}", file.name)).strong());
        });
        if let Some(e) = stage.errors.get(&file.id) {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        }
        ui.indent(("file", file.id), |ui| {
            for mo in stage.objects(file.id) {
                let key = object_key(file.id, &mo.key);
                ui.horizontal(|ui| {
                    let over = file.parts.entry(mo.key.clone()).or_default();
                    let mut visible = over.visible.unwrap_or(true);
                    if ui.checkbox(&mut visible, "").on_hover_text(tr("Objekt anzeigen", "Show object")).changed() {
                        over.visible = (!visible).then_some(false);
                    }
                    let selected = state.selected_objects.contains(&key);
                    if ui.selectable_label(selected, &mo.name).clicked() {
                        let (shift, cmd) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
                        pick = Some(match (shift, cmd, &state.anchor_object) {
                            (true, _, Some(a)) => Pick::ObjectRange(a.clone(), key.clone()),
                            (_, true, _) => Pick::ToggleObject(key.clone()),
                            _ if state.selected_objects == [key.clone()] => Pick::Clear,
                            _ => Pick::Object(key.clone()),
                        });
                    }
                });
            }
        });
        // Don't keep empty override entries around.
        file.parts.retain(|_, o| *o != tapetum_project::PartOverride::default());
    }
    let _ = state;
    pick
}

/// Settings of one stage object; returns true to close the editor.
fn object_editor(ui: &mut egui::Ui, state: &ShowState, project: &mut tapetum_project::Project, key: &str, stage: &StageObjects, rt: &SceneRuntime) -> bool {
    use tapetum_project::{Surface, UpAxis};
    let mut close = false;
    let Some((file_id, part_key)) = split_key(key) else { return true };
    let Some(mo) = stage.object(key) else {
        ui.label(tr("Objekt wird geladen …", "Loading object …"));
        return false;
    };
    let scene_name = rt.active.and_then(|id| project.scenes.iter().find(|s| s.id == id)).map(|s| s.name.clone());
    let Some(file) = project.objects.iter_mut().find(|o| o.id == file_id) else { return true };
    let igz = is_igz(file);
    ui.horizontal(|ui| {
        ui.heading(&mo.name);
        if ui.small_button("✖").on_hover_text(tr("Schließen", "Close")).clicked() {
            close = true;
        }
    });
    let size = mo.bounds().map(|(lo, hi)| hi - lo).map_or(String::new(), |s| format!(" · {:.2} × {:.2} × {:.2} m", s.x, s.y, s.z));
    ui.colored_label(egui::Color32::GRAY, trf!("aus {} · {} Dreiecke{size}", "from {} · {} triangles{size}", file.name, mo.triangle_count(), size = size));
    let multi = state.selected_objects.len() > 1;
    if multi {
        ui.colored_label(
            egui::Color32::LIGHT_YELLOW,
            trf!("{} Objekte ausgewählt: Material-Änderungen gelten für alle, das Werkzeug verschiebt alle.", "{} objects selected: material changes apply to all, the tool moves all.", state.selected_objects.len()),
        );
    }

    // Position: per scene; in the base scene only files that aren't IngeTrazo's.
    ui.separator();
    match rt.active.zip(scene_name).filter(|_| !multi) {
        Some((scene_id, name)) => {
            ui.strong(trf!("Position in „{name}“", "Position in “{name}”", name = name));
            if let Some(scene) = project.scenes.iter_mut().find(|s| s.id == scene_id) {
                let mut st = scene.objects.get(key).copied().unwrap_or_default();
                let before = st;
                egui::Grid::new("scene_tf").num_columns(4).show(ui, |ui| {
                    ui.label(tr("Versatz", "Offset"));
                    for v in &mut st.offset {
                        ui.add(egui::DragValue::new(v).speed(0.01).suffix(" m").max_decimals(3));
                    }
                    ui.end_row();
                    ui.label(tr("Drehung", "Rotation"));
                    for v in &mut st.rotation {
                        ui.add(egui::DragValue::new(v).speed(0.5).suffix("°").max_decimals(1));
                    }
                    ui.end_row();
                });
                ui.checkbox(&mut st.visible, tr("In dieser Szene sichtbar", "Visible in this scene"));
                if ui.small_button(tr("Wie in der Grundszene", "As in the base scene")).clicked() {
                    st = tapetum_project::SceneObjectState::default();
                }
                if st != before {
                    if st == tapetum_project::SceneObjectState::default() {
                        scene.objects.remove(key);
                    } else {
                        scene.objects.insert(key.to_string(), st);
                    }
                }
                ui.colored_label(egui::Color32::GRAY, tr("Im 3D-Fenster: Pfeile verschieben (mit Shift in 10-cm-Schritten), Ringe drehen um den Fußpunkt (in 15°-Schritten, mit Shift frei).", "In the 3D view: arrows move (with Shift in 10 cm steps), rings turn around the foot point (in 15° steps, freely with Shift)."));
            }
        }
        None if igz => {
            ui.colored_label(egui::Color32::GRAY, tr("Grundszene: Position wie in IngeTrazo. Verschieben lässt sich das Objekt in Szenen.", "Base scene: position as in IngeTrazo. The object can be moved in scenes."));
        }
        None => {
            ui.strong(tr("Datei in der Grundszene", "File in the base scene"));
            egui::Grid::new("file_tf").num_columns(4).show(ui, |ui| {
                ui.label("Position");
                for v in &mut file.transform.translation {
                    ui.add(egui::DragValue::new(v).speed(0.01).suffix(" m").max_decimals(3));
                }
                ui.end_row();
                ui.label(tr("Drehung", "Rotation"));
                for v in &mut file.transform.rotation {
                    ui.add(egui::DragValue::new(v).speed(0.5).suffix("°").max_decimals(1));
                }
                ui.end_row();
                ui.label(tr("Skalierung", "Scale"));
                for v in &mut file.transform.scale {
                    ui.add(egui::DragValue::new(v).speed(0.01).range(0.001..=1000.0).max_decimals(3));
                }
                ui.end_row();
            });
            ui.colored_label(egui::Color32::GRAY, tr("Verschiebt alle Objekte der Datei. Einzelne Objekte in Szenen verschieben.", "Moves all objects of the file. Move single objects in scenes."));
        }
    }

    // Material: from the file unless overridden here (the file isn't changed).
    ui.separator();
    ui.strong("Material");
    let over = file.parts.entry(part_key.to_string()).or_default();
    let before = *over;
    ui.horizontal(|ui| {
        let mut own = over.color.is_some();
        if ui.checkbox(&mut own, tr("Eigene Farbe", "Own colour")).changed() {
            over.color = own.then(|| file_color(mo));
        }
        match &mut over.color {
            Some(c) => {
                ui.color_edit_button_rgb(c);
            }
            None => {
                ui.colored_label(egui::Color32::GRAY, tr("aus der Datei", "from the file"));
            }
        }
    });
    ui.horizontal(|ui| {
        let mut own = over.reflectance.is_some();
        if ui.checkbox(&mut own, tr("Reflexionsgrad", "Reflectance")).changed() {
            over.reflectance = own.then(|| file_reflectance(mo).clamp(0.01, 1.0));
        }
        match &mut over.reflectance {
            Some(r) => {
                let mut pct = *r * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 1.0..=95.0).suffix(" %")).changed() {
                    *r = pct / 100.0;
                }
            }
            None => {
                ui.colored_label(egui::Color32::GRAY, trf!("aus der Datei (~{:.0} %)", "from the file (~{:.0} %)", file_reflectance(mo) * 100.0));
            }
        }
    });
    ui.colored_label(egui::Color32::GRAY, tr("Reflexionsgrad: wie viel Licht die Oberfläche zurückwirft – Molton ≈ 3 %, Holz ≈ 30 %, weißer Tanzteppich ≈ 75 %.", "Reflectance: how much light the surface reflects – molton ≈ 3 %, wood ≈ 30 %, white dance floor ≈ 75 %."));
    let current = over.surface.unwrap_or(Surface::Opaque);
    let mut translucent = matches!(current, Surface::Translucent { .. });
    ui.horizontal(|ui| {
        if ui.radio_value(&mut translucent, false, tr("Opak", "Opaque")).changed() || ui.radio_value(&mut translucent, true, tr("Transluzent", "Translucent")).changed() {
            over.surface = translucent.then_some(Surface::Translucent { transmission: 0.6, diffusion: 0.8 });
        }
    });
    if let Some(Surface::Translucent { transmission, diffusion }) = &mut over.surface {
        ui.add(egui::Slider::new(transmission, 0.0..=1.0).text(tr("Lichtdurchlässigkeit", "Transmission")));
        ui.add(egui::Slider::new(diffusion, 0.0..=1.0).text(tr("Streuung", "Diffusion")));
        ui.colored_label(
            egui::Color32::GRAY,
            tr("Von vorn wird die Fläche normal beleuchtet. Licht von hinten scheint durch: mit wenig Streuung sieht man klar hindurch (Gaze, Glas), mit viel Streuung leuchtet die Fläche diffus und man erkennt nichts dahinter (Operafolie).", "From the front the surface is lit as usual. Light from behind shines through: with little diffusion you see clearly through it (gauze, glass), with a lot of diffusion the surface glows evenly and hides what's behind (opera foil)."),
        );
    }
    if *over != tapetum_project::PartOverride::default() && ui.small_button(tr("Material wie in der Datei", "Material as in the file")).clicked() {
        let visible = over.visible;
        *over = tapetum_project::PartOverride { visible, ..Default::default() };
    }
    let after = *over;

    // The file.
    ui.separator();
    ui.strong(trf!("Datei {}", "File {}", file.name));
    ui.colored_label(egui::Color32::GRAY, &file.model);
    if !super::objects::model_path(state, file).exists() {
        ui.colored_label(egui::Color32::LIGHT_RED, tr("Datei nicht gefunden.", "File not found."));
    }
    ui.colored_label(egui::Color32::GRAY, tr("Änderungen an der Datei werden automatisch übernommen; Tapetum ändert sie nie.", "Changes to the file are picked up automatically; Tapetum never changes it."));
    if !igz {
        let units: [(&str, f32); 5] = [(tr("Meter", "Metres"), 1.0), (tr("Zentimeter", "Centimetres"), 0.01), (tr("Millimeter", "Millimetres"), 0.001), (tr("Zoll", "Inches"), 0.0254), (tr("Fuß", "Feet"), 0.3048)];
        let unit_label = units.iter().find(|(_, f)| (f - file.unit_scale).abs() < 1e-6).map_or(tr("eigene", "custom").to_string(), |(n, _)| n.to_string());
        egui::ComboBox::from_label(tr("Einheit", "Unit")).selected_text(unit_label).show_ui(ui, |ui| {
            for (name, f) in units {
                ui.selectable_value(&mut file.unit_scale, f, name);
            }
        });
        let gltf = file.model.to_ascii_lowercase().ends_with(".glb") || file.model.to_ascii_lowercase().ends_with(".gltf");
        ui.add_enabled_ui(!gltf, |ui| {
            egui::ComboBox::from_label(tr("Oben ist", "Up is"))
                .selected_text(match file.up {
                    UpAxis::Z => "Z (CAD)",
                    UpAxis::Y => tr("Y (OBJ-Export)", "Y (OBJ export)"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut file.up, UpAxis::Z, "Z (CAD)");
                    ui.selectable_value(&mut file.up, UpAxis::Y, tr("Y (OBJ-Export)", "Y (OBJ export)"));
                });
        });
    }
    if ui.button(tr("Datei entfernen", "Remove file")).clicked() {
        project.objects.retain(|o| o.id != file_id);
        for s in &mut project.scenes {
            s.objects.retain(|k, _| split_key(k).is_none_or(|(f, _)| f != file_id));
        }
        return true;
    }
    // A material change applies to every selected object, field by field.
    if after != before {
        for other in state.selected_objects.iter().filter(|k| k.as_str() != key) {
            let Some((f, part)) = split_key(other) else { continue };
            let Some(o) = project.objects.iter_mut().find(|o| o.id == f).map(|o| o.parts.entry(part.to_string()).or_default()) else { continue };
            if after.color != before.color {
                o.color = after.color;
            }
            if after.reflectance != before.reflectance {
                o.reflectance = after.reflectance;
            }
            if after.surface != before.surface {
                o.surface = after.surface;
            }
        }
    }
    close
}

fn devices(n: usize) -> String {
    if n == 1 { tr("1 Gerät", "1 fixture").into() } else { trf!("{n} Geräte", "{n} fixtures", n = n) }
}

/// Selection changes from the panels, applied after drawing.
enum Pick {
    Fixture(usize),
    Group(usize),
    /// Group chosen in the menu of this fixture.
    GroupWith(usize, usize),
    Object(String),
    /// Cmd: add or remove one; Shift: everything from the anchor to here.
    ToggleObject(String),
    ObjectRange(String, String),
    FixtureRange(usize, usize),
    GroupRange(usize, usize),
    /// Shift/Cmd: add a group or toggle a fixture.
    AddGroup(usize, Option<usize>),
    Toggle(usize),
    Clear,
}

/// Center of several fixtures as numbers; changing it moves all of them.
fn group_placement_editor(ui: &mut egui::Ui, state: &ShowState, project: &mut tapetum_project::Project, sync: &mut PlacementSync) {
    let placements = super::layout::placements(&state.show, project);
    let sel: Vec<usize> = state.selection.iter().copied().filter(|&i| i < placements.len()).collect();
    let n = sel.len().max(1) as f32;
    let center = sel.iter().fold([0.0f32; 3], |c, &i| {
        let p = placements[i].position;
        [c[0] + p[0] / n, c[1] + p[1] / n, c[2] + p[2] / n]
    });
    let mut moved = center;
    ui.horizontal(|ui| {
        ui.label(tr("Mitte", "Centre"));
        for v in &mut moved {
            ui.add(egui::DragValue::new(v).speed(0.01).suffix(" m").max_decimals(3));
        }
    });
    if moved != center {
        let d = [moved[0] - center[0], moved[1] - center[1], moved[2] - center[2]];
        for &i in &sel {
            let f = &state.show.fixtures[i];
            let mut p = placements[i];
            p.position = [p.position[0] + d[0], p.position[1] + d[1], p.position[2] + d[2]];
            project.pending_placements.insert(f.id, p);
            sync.queue(f.id, &f.short_name, p);
        }
    }
    ui.colored_label(egui::Color32::GRAY, tr("Im 3D-Fenster: Pfeile verschieben alle, Ringe drehen um die Mitte.", "In the 3D view: arrows move all, rings turn around the centre."));
}

/// Position and rotation of a fixture as numbers; the gizmo edits the same values.
fn placement_editor(ui: &mut egui::Ui, state: &ShowState, project: &mut tapetum_project::Project, i: usize, sync: &mut PlacementSync, bk: Option<&HostInfo>) {
    let f = &state.show.fixtures[i];
    let mut p = super::layout::placements(&state.show, project)[i];
    let before = p;
    egui::Grid::new("placement").num_columns(4).show(ui, |ui| {
        ui.label("Position");
        for v in &mut p.position {
            ui.add(egui::DragValue::new(v).speed(0.01).suffix(" m").max_decimals(3));
        }
        ui.end_row();
        ui.label(tr("Drehung", "Rotation"));
        for v in &mut p.rotation {
            ui.add(egui::DragValue::new(v).speed(0.5).suffix("°").max_decimals(1));
        }
        ui.end_row();
    });
    if p != before {
        project.pending_placements.insert(f.id, p);
        sync.queue(f.id, &f.short_name, p);
    }
    let connected = bk.is_some_and(|b| b.has_open(&state.show_path));
    let note = match (project.pending_placements.contains_key(&f.id), f.has_placement, connected) {
        (true, _, true) => tr("Verschoben – in BlinderKitten eingetragen. Dort speichern, damit es bleibt.", "Moved – entered in BlinderKitten. Save there to keep it."),
        (true, _, false) => tr("Verschoben – wird an BlinderKitten übertragen, sobald die Show dort offen ist.", "Moved – goes to BlinderKitten as soon as the show is open there."),
        (false, true, _) => tr("Position aus BlinderKitten.", "Position from BlinderKitten."),
        (false, false, _) => tr("Keine Position in BlinderKitten – automatisch aufgestellt.", "No position in BlinderKitten – placed automatically."),
    };
    ui.colored_label(egui::Color32::GRAY, note);
    ui.colored_label(egui::Color32::GRAY, tr("Im 3D-Fenster: Pfeile ziehen zum Verschieben (mit Shift in 10-cm-Schritten), Ringe zum Drehen (in 15°-Schritten, mit Shift frei).", "In the 3D view: drag the arrows to move (with Shift in 10 cm steps), the rings to turn (in 15° steps, freely with Shift)."));
}

/// Screen areas (egui points = logical pixels) covered by panels; the 3D
/// camera ignores the mouse there. The panels live on egui's background layer,
/// which egui itself doesn't count as "pointer over UI".
#[derive(Resource, Default)]
pub struct UiRects(pub Vec<egui::Rect>);

impl UiRects {
    pub fn contains(&self, p: Vec2) -> bool {
        self.0.iter().any(|r| r.contains(egui::pos2(p.x, p.y)))
    }
}

enum SetupAction {
    Edit(Edit, bool),
    ChangePort(u16),
    RemotePort(u16),
}

const OK: egui::Color32 = egui::Color32::LIGHT_GREEN;
const BAD: egui::Color32 = egui::Color32::LIGHT_RED;
const HINT: egui::Color32 = egui::Color32::GRAY;

fn step(ui: &mut egui::Ui, done: bool, title: &str) {
    ui.horizontal(|ui| {
        ui.colored_label(if done { OK } else { BAD }, if done { "✔" } else { "✖" });
        ui.strong(title);
    });
}

fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.indent("hint", |ui| {
        ui.colored_label(HINT, text.into());
    });
}

/// Checklist for connecting BK, with everything that has to be done by hand in BK.
fn setup_section(ui: &mut egui::Ui, state: &ShowState, link: &BkLink, bk: Option<&HostInfo>, listen_port: u16, receiving: bool) -> Option<SetupAction> {
    let show = &state.show;
    let remote_port = state.project.artnet.bk_remote_port;
    let mut action = None;

    let mirrors: Vec<(String, Option<String>)> = show
        .interfaces
        .iter()
        .enumerate()
        .filter(|(_, i)| !bk_olga::mirror::is_mirror_name(&i.name) && !matches!(i.kind, InterfaceKind::NonDmx(_)))
        .map(|(idx, i)| {
            let m = show.mirror_of(idx).map(|m| match &m.kind {
                InterfaceKind::ArtNet(a) => format!("{} ({}, Port {})", m.name, format_port_address(a.port_address()), a.port),
                _ => m.name.clone(),
            });
            (i.name.clone(), m)
        })
        .collect();
    let any_mirror = mirrors.iter().any(|(_, m)| m.is_some());
    let all_mirrored = !mirrors.is_empty() && mirrors.iter().all(|(_, m)| m.is_some());
    let mirror_port_ok = show.interfaces.iter().filter(|i| bk_olga::mirror::is_mirror_name(&i.name)).all(|i| match &i.kind {
        InterfaceKind::ArtNet(a) => a.port == listen_port,
        _ => true,
    });
    let open_here = bk.is_some_and(|b| b.has_open(&state.show_path));
    let blockers: Vec<&str> = show
        .interfaces
        .iter()
        .filter(|i| matches!(&i.kind, InterfaceKind::ArtNet(a) if a.binds_6454))
        .map(|i| i.name.as_str())
        .collect();
    let all_done = bk.is_some() && open_here && all_mirrored && mirror_port_ok && receiving;

    egui::CollapsingHeader::new(if all_done { tr("BlinderKitten-Verbindung ✔", "BlinderKitten connection ✔") } else { tr("BlinderKitten einrichten", "Set up BlinderKitten") })
        .id_salt("setup")
        .default_open(!all_done)
        // Reopen by itself when something stops working.
        .open((!all_done).then_some(true))
        .show(ui, |ui| {
            step(ui, bk.is_some(), tr("1. OSC Remote Control in BlinderKitten einschalten", "1. Turn on OSC Remote Control in BlinderKitten"));
            match bk {
                Some(b) => hint(ui, trf!("BlinderKitten {} antwortet auf Port {remote_port}.", "BlinderKitten {} answers on port {remote_port}.", b.version, remote_port = remote_port)),
                None => {
                    hint(
                        ui,
                        tr(
                            "In BlinderKitten: Menü File > Preferences > „OSC Remote Control“ aufklappen \
                             und „Enabled“ anhaken. Die Einstellung gilt für alle Shows und muss nur einmal \
                             gesetzt werden. „Local Port“ dort muss mit diesem Port übereinstimmen:",
                            "In BlinderKitten: menu File > Preferences, open “OSC Remote Control” and tick \
                             “Enabled”. The setting applies to all shows and is needed only once. \
                             “Local Port” there must match this port:",
                        ),
                    );
                    ui.indent("remote_port", |ui| {
                        let mut port = remote_port;
                        if ui.add(egui::DragValue::new(&mut port).range(1..=65535).prefix("Port ")).changed() {
                            action = Some(SetupAction::RemotePort(port));
                        }
                    });
                }
            }

            step(ui, open_here, tr("2. Diese Show in BlinderKitten öffnen", "2. Open this show in BlinderKitten"));
            match bk {
                Some(b) if !open_here => {
                    let open = if b.file_path.as_os_str().is_empty() { tr("eine ungespeicherte Show", "an unsaved show").to_string() } else { b.file_path.display().to_string() };
                    hint(ui, trf!("Geöffnet ist {open}.", "Open is {open}.", open = open));
                    hint(ui, trf!("Gebraucht wird {}.", "Needed is {}.", state.show_path.display()));
                }
                Some(b) if b.unsaved => hint(ui, tr("Ungespeicherte Änderungen speichert der Einrichten-Knopf automatisch mit.", "The set-up button saves unsaved changes along with it.")),
                Some(_) => {}
                None => hint(ui, format!("{}", state.show_path.display())),
            }

            step(ui, all_mirrored && mirror_port_ok, tr("3. Tapetum-Interfaces anlegen", "3. Add the Tapetum interfaces"));
            for (orig, m) in &mirrors {
                match m {
                    Some(m) => hint(ui, format!("{orig} » {m}")),
                    None => hint(ui, trf!("{orig} – noch nicht gespiegelt", "{orig} – not mirrored yet", orig = orig)),
                }
            }
            if !mirror_port_ok {
                ui.colored_label(BAD, trf!("Die Tapetum-Interfaces senden nicht an Port {listen_port} – bitte aktualisieren.", "The Tapetum interfaces don't send to port {listen_port} – please update them.", listen_port = listen_port));
            }
            ui.add_enabled_ui(!link.busy(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    let label = if any_mirror { tr("Tapetum-Interfaces aktualisieren", "Update the Tapetum interfaces") } else { tr("Tapetum-Interfaces in BK anlegen", "Add the Tapetum interfaces in BK") };
                    let tip = tr(
                        "Legt für jedes DMX-Interface ein Art-Net-Interface „Tapetum …“ an, das an Tapetum \
                         sendet, und trägt dort einen Pass-through ein. Patch und Hardware \
                         bleiben unverändert.\n\nBlinderKitten speichert dafür die Show und lädt sie neu. \
                         Eine Sicherung liegt danach als .olga.bak neben der Show.",
                        "Adds an Art-Net interface “Tapetum …” sending to Tapetum for every DMX interface, \
                         and a pass-through entry to it. Patch and hardware stay unchanged.\n\n\
                         BlinderKitten saves and reloads the show for this. \
                         A backup is left next to the show as .olga.bak.",
                    );
                    if ui.button(label).on_hover_text(tip).clicked() {
                        action = Some(SetupAction::Edit(Edit::AddMirrors { listen_port }, false));
                    }
                    if any_mirror && ui.button(tr("Entfernen", "Remove")).clicked() {
                        action = Some(SetupAction::Edit(Edit::RemoveMirrors, false));
                    }
                });
            });
            if let Some((msg, error)) = &link.message {
                ui.colored_label(if link.busy() { egui::Color32::LIGHT_YELLOW } else if *error { BAD } else { OK }, msg);
            }
            if let Some(edit) = link.needs_offline {
                ui.colored_label(BAD, tr("BlinderKitten antwortet nicht (Schritt 1).", "BlinderKitten doesn't answer (step 1)."));
                ui.label(tr("Ist BlinderKitten geschlossen, kann die Datei auch direkt geändert werden – sonst würde BK die Änderung beim nächsten Speichern überschreiben.", "With BlinderKitten closed, the file can also be changed directly – otherwise BK would overwrite the change the next time it saves."));
                ui.horizontal(|ui| {
                    if ui.button(tr("Erneut versuchen", "Try again")).clicked() {
                        action = Some(SetupAction::Edit(edit, false));
                    }
                    if ui.button(tr("Datei direkt ändern", "Change the file directly")).clicked() {
                        action = Some(SetupAction::Edit(edit, true));
                    }
                });
            }
            if any_mirror {
                hint(ui, tr("In BlinderKitten danach nichts weiter einstellen: Die Show ist schon gespeichert. Die Tapetum-Interfaces bitte nicht umbenennen.", "Nothing more to set in BlinderKitten afterwards: the show is already saved. Please don't rename the Tapetum interfaces."));
            }

            step(ui, receiving, tr("4. Art-Net kommt an", "4. Art-Net arrives"));
            if !receiving {
                if any_mirror {
                    hint(ui, tr("Kommt nichts an, in BlinderKitten prüfen:", "If nothing arrives, check in BlinderKitten:"));
                    hint(ui, tr("• Die Tapetum-Interfaces und die Original-Interfaces sind aktiviert (Haken in der Interface-Liste).", "• The Tapetum interfaces and the original interfaces are enabled (tick in the interface list)."));
                    hint(ui, tr("• Bei den Original-Interfaces steht unter „Pass-through“ ein „Output Universe“ auf das Tapetum-Interface.", "• The original interfaces have an “Output Universe” under “Pass-through” pointing to the Tapetum interface."));
                    hint(ui, tr("• Es ist eine Cue aktiv oder im Programmer ein Wert gesetzt (ohne Werte sendet BK nur Nullen).", "• A cue is running or the programmer has a value (without values BK sends only zeros)."));
                }
                if cfg!(windows) {
                    hint(ui, trf!("• Windows-Firewall: eingehendes UDP auf Port {listen_port} für Tapetum erlauben.", "• Windows firewall: allow incoming UDP on port {listen_port} for Tapetum.", listen_port = listen_port));
                }
            }

            if !blockers.is_empty() && listen_port == bk_artnet::DEFAULT_PORT {
                ui.add_space(4.0);
                ui.colored_label(
                    BAD,
                    trf!(
                        "{} belegt in BlinderKitten den Art-Net-Port 6454 (Art-Net-Input oder „Force src port“ aktiv). \
                         Auf demselben Rechner kann Tapetum dann nicht auf 6454 empfangen.",
                        "{} takes Art-Net port 6454 in BlinderKitten (Art-Net input or “Force src port” on). \
                         On the same computer Tapetum can't receive on 6454 then.",
                        blockers.join(", ")
                    ),
                );
                hint(ui, tr("Entweder in BK bei diesen Interfaces Input und „Force src port“ ausschalten, oder den Visualizer auf einen anderen Port legen:", "Either turn off Input and “Force src port” for these interfaces in BK, or move Tapetum to another port:"));
                if ui.add_enabled(!link.busy(), egui::Button::new(tr("Visualizer auf Port 6455 umstellen", "Move Tapetum to port 6455"))).clicked() {
                    action = Some(SetupAction::ChangePort(6455));
                }
            }
        });
    action
}

fn spacemouse_section(ui: &mut egui::Ui, s: &mut tapetum_project::SpaceMouseSettings, device: Option<String>) {
    let title = match &device {
        Some(name) => format!("SpaceMouse: {name}"),
        None => tr("SpaceMouse (nicht gefunden)", "SpaceMouse (not found)").to_string(),
    };
    egui::CollapsingHeader::new(title).id_salt("spacemouse").show(ui, |ui| {
        ui.checkbox(&mut s.enabled, tr("Kamera mit der SpaceMouse steuern", "Control the camera with the SpaceMouse"));
        ui.add_enabled_ui(s.enabled, |ui| {
            ui.add(egui::Slider::new(&mut s.speed, 0.1..=4.0).logarithmic(true).text(tr("Tempo", "Speed")));
            ui.horizontal(|ui| {
                ui.label(tr("Die Kappe bewegt", "The cap moves"));
                ui.selectable_value(&mut s.move_stage, true, tr("die Bühne", "the stage"));
                ui.selectable_value(&mut s.move_stage, false, tr("die Kamera", "the camera"));
            });
            ui.checkbox(&mut s.keep_in_background, tr("Auch steuern, während ein anderes Programm vorne ist", "Keep control while another program is in front"))
                .on_hover_text(tr("Zum Beispiel beim Bedienen von BlinderKitten. Andere Programme bekommen die SpaceMouse dann nicht, solange Tapetum läuft.", "For example while operating BlinderKitten. Other programs then don't get the SpaceMouse while Tapetum runs."));
        });
        ui.colored_label(
            egui::Color32::GRAY,
            tr("Schieben: seitlich und hoch/runter, Ziehen/Drücken: näher/weiter, Verdrehen: um die Bühne herum, Kippen: von oben/unten. Linke Taste: Zuschauersicht, rechte Taste: Draufsicht – jeweils so, dass alles zu sehen ist.", "Push: sideways and up/down, pull/press: closer/further, twist: around the stage, tilt: from above/below. Left button: audience view, right button: top view – each framing everything."),
        );
        if device.is_none() {
            ui.colored_label(
                egui::Color32::GRAY,
                if cfg!(target_os = "linux") {
                    tr("Unter Linux braucht der Zugriff auf /dev/hidraw* eine udev-Regel für 3Dconnexion (Hersteller 256f bzw. 046d).", "On Linux, access to /dev/hidraw* needs a udev rule for 3Dconnexion (vendor 256f or 046d).")
                } else {
                    tr("Wird automatisch erkannt, sobald sie angeschlossen ist.", "Detected automatically as soon as it's plugged in.")
                },
            );
        }
    });
}

fn quality_label(q: Quality) -> &'static str {
    match q {
        Quality::Low => tr("Niedrig – ältere Rechner", "Low – older computers"),
        Quality::Medium => tr("Mittel – Standard", "Medium – standard"),
        Quality::High => tr("Hoch – starke Grafik", "High – strong graphics"),
    }
}

fn quality_hint(q: Quality) -> &'static str {
    match q {
        Quality::Low => tr("Für ältere oder günstige Rechner und integrierte Grafik: einfachere Lichtstrahlen, keine Schatten, kein Bloom, Saallicht als gleichmäßiges Licht, Kantenglättung FXAA, 3D etwas unter der Bildschirmauflösung.", "For older or low-cost computers and integrated graphics: simpler beams, no shadows, no bloom, house light as even light, FXAA anti-aliasing, 3D a bit below screen resolution."),
        Quality::Medium => tr("Für die meisten Rechner: weiche Lichtstrahlen (TAA), Strahlen enden an Bühnenobjekten und werfen dort Schatten.", "For most computers: smooth beams (TAA), beams end at stage objects and cast shadows there."),
        Quality::High => tr("Für starke Grafikkarten: feinere Strahlen, Schatten auch für jede einzelne Zelle, 3D in voller (Retina-)Auflösung.", "For strong graphics cards: finer beams, shadows for every single cell too, 3D at full (Retina) resolution."),
    }
}
