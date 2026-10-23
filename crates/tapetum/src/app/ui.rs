use bevy::prelude::*;
use crate::i18n::tr;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use bevy_egui::{EguiContexts, EguiTextureHandle, egui};
use tapetum_project::{FixtureOverride, Quality};

use super::gizmo::GizmoState;
use super::scene::View3d;
use super::objects::{SceneRuntime, StageObjects, file_color, file_reflectance, is_igz, object_key, split_key, stored_path};
use super::undo::{Step, UndoHistory};
use super::state::{BkLink, BkStatus, Dmx, GoboImport, LibraryDone, LibraryJobs, LibraryUpdates, ObjectImport, PlacementSync, RangeSync, ShowState};
use super::type_editor;
use super::onboarding::SetupAction;
use crate::bk_link::{Edit, HostInfo};
use crate::routing::format_port_address;

static HIDE_SIDEBAR_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Start with the sidebar hidden (for screenshots).
pub fn hide_sidebar_at_start() {
    HIDE_SIDEBAR_AT_START.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Tab pressed this frame, for the sidebar.
#[derive(Resource, Default)]
pub struct TabKey(pub bool);

/// Takes Tab away from egui (which would cycle the focus through the widgets
/// and then count as busy with the keyboard) and hands it to the sidebar;
/// not while typing into a text field.
pub fn take_tab(mut contexts: Query<(&mut bevy_egui::EguiContext, &mut bevy_egui::EguiInput), With<bevy_egui::PrimaryEguiContext>>, mut tab: ResMut<TabKey>) {
    tab.0 = false;
    for (mut ctx, mut input) in &mut contexts {
        if ctx.get_mut().text_edit_focused() {
            continue;
        }
        input.events.retain(|e| match e {
            egui::Event::Key { key: egui::Key::Tab, pressed, repeat, modifiers, .. } if !modifiers.any() => {
                tab.0 |= *pressed && !*repeat;
                false
            }
            _ => true,
        });
    }
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
    (mut object_import, stage_objects, mut scene_rt, spacemouse): (ResMut<ObjectImport>, Res<StageObjects>, ResMut<SceneRuntime>, Res<super::spacemouse::SpaceMouse>),
    (mut import, mut library, mut offers): (ResMut<GoboImport>, ResMut<LibraryJobs>, ResMut<LibraryUpdates>),
    view: Res<View3d>,
    mut ui_rects: ResMut<UiRects>,
    time: Res<Time>,
    mut fps: Local<f32>,
    (mut sidebar_hidden, mut slow_since, tab): (Local<bool>, Local<Option<f32>>, Res<TabKey>),
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
    if tab.0 {
        *sidebar_hidden = !*sidebar_hidden;
    }
    if !ctx.text_edit_focused() {
        let (redo, undo_key) = ctx.input(|i| {
            let z = i.modifiers.command && i.key_pressed(egui::Key::Z);
            (z && i.modifiers.shift || i.modifiers.ctrl && i.key_pressed(egui::Key::Y), z && !i.modifiers.shift)
        });
        if redo {
            undo.request = Some(Step::Redo);
        } else if undo_key {
            undo.request = Some(Step::Undo);
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
    // Esc clears the selection (not while typing, where it ends the edit).
    if !ctx.text_edit_focused() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        pick = Some(Pick::Clear);
    }
    if let Some(file) = object_import.poll() {
        let (unit_scale, up) = crate::stage::defaults_for(&file);
        let id = project.objects.iter().map(|o| o.id).max().map_or(1, |m| m + 1);
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| trf!("Datei {id}", "File {id}", id = id));
        project.objects.push(tapetum_project::StageObject { id, name, model: stored_path(&state, &file), unit_scale, up, ..Default::default() });
    }
    library_offers(ctx, &mut offers, &mut project, &mut state);
    export_notice(ctx);
    match library.poll() {
        Some(LibraryDone::Exported(short, file, source, modified, mochi_error)) => {
            let short_for_notice = short.clone();
            let applied = crate::fixture_library::fingerprint(&project.fixture_types.get(&short).cloned().unwrap_or_default());
            project.library_links.insert(short, tapetum_project::LibraryLink { file: file.to_string_lossy().into_owned(), applied, source, seen: modified });
            let name = state.show.fixture_types.iter().find(|t| t.short_name == short_for_notice).map_or_else(|| short_for_notice.clone(), |t| t.name.clone());
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("export_notice"), (name, file.clone(), mochi_error)));
        }
        Some(LibraryDone::Picked(short, file)) => match crate::fixture_library::take_over(&mut project, &short, &file, &state.project_dir()) {
            Ok(()) => state.status = Some(trf!("Aus der Gerätebibliothek geladen: {}", "Loaded from the fixture library: {}", file.display())),
            Err(e) => state.status = Some(trf!("Gerätedatei: {e}", "Fixture file: {e}", e = e)),
        },
        Some(LibraryDone::Failed(e)) => state.status = Some(trf!("Gerätebibliothek: {e}", "Fixture library: {e}", e = e)),
        None => {}
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

    // What arrives from BlinderKitten (for the sidebar and the getting-started window).
    let listen_port = dmx.receiver.local_addr.port();
    let mut universes: Vec<(u16, String, f32)> =
        dmx.receiver.store.snapshot().into_iter().map(|(pa, u)| (pa, u.source.to_string(), u.last_seen.elapsed().as_secs_f32())).collect();
    universes.sort_by_key(|u| u.0);
    let receiving = universes.iter().any(|u| u.2 < 2.0);
    if let Some(a) = super::onboarding::window(ctx, &state, &link, bk.as_ref(), listen_port, receiving) {
        setup_action = Some(a);
    }

    // Over the 3D view: light and haze below, the scenes above.
    let mut rects = Vec::new();
    let spacemouse_on = spacemouse.device().is_some() && project.spacemouse.enabled;
    rects.push(super::overlays::faders(ctx, &mut project, spacemouse_on));
    let (scene_pick, bar) = super::overlays::scene_bar(ctx, &state, &mut project, &scene_rt);
    rects.push(bar);
    match scene_pick {
        // While the main cuelist runs, a scene chosen by hand holds until its next cue.
        Some(super::overlays::ScenePick::Show(scene)) => match scene_rt.following_cue {
            // The scene the cuelist shows anyway is in step; any other holds until the next cue.
            Some(cue) if project.scene_for_cue(cue) == scene => scene_rt.detached = None,
            Some(cue) => scene_rt.detached = Some((scene, cue)),
            None => {
                manual_scene = scene;
                scene_rt.detached = None;
            }
        },
        Some(super::overlays::ScenePick::Follow) => scene_rt.detached = None,
        None => {}
    }

    let mut root = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    let mut hide = false;
    if HIDE_SIDEBAR_AT_START.swap(false, std::sync::atomic::Ordering::Relaxed) {
        *sidebar_hidden = true;
    }
    // The sidebar slides in and out (Tab, or its « button).
    let mut expanded = !*sidebar_hidden;
    let left = egui::Panel::left("panel").resizable(true).default_size(310.0).show_collapsible(&mut root, &mut expanded, |ui| {
            // The header stays put; everything below scrolls.
            ui.horizontal(|ui| {
                ui.heading("Tapetum");
                ui.weak(format!("{:.0} fps", *fps));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                super::settings::section(ui, &mut project, spacemouse.device());
                if let Some(s) = &state.status {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, s);
                }
                for w in &state.show.warnings {
                    ui.colored_label(egui::Color32::LIGHT_RED, w);
                }
                // Running slowly for a while: offer the next lower quality.
                let r = &mut project.render;
                if *slow_since.get_or_insert(0.0) > 5.0 && r.quality != Quality::Low {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(egui::Color32::LIGHT_YELLOW, trf!("Läuft langsam ({:.0} fps).", "Running slowly ({:.0} fps).", *fps));
                        let lower = if r.quality == Quality::High { Quality::Medium } else { Quality::Low };
                        if ui.small_button(trf!("Auf „{}“ stellen", "Switch to “{}”", super::settings::quality_label(lower))).clicked() {
                            r.quality = lower;
                            *slow_since = Some(0.0);
                        }
                    });
                }

                ui.separator();
                // The connection: opens by itself when something is wrong, closes when all works.
                let done = super::onboarding::all_done(&state, bk.as_ref(), listen_port, receiving);
                let done_id = egui::Id::new("connection_done");
                let was_done = ui.ctx().data(|d| d.get_temp::<bool>(done_id));
                ui.ctx().data_mut(|d| d.insert_temp(done_id, done));
                let title = if done { tr("BlinderKitten ✔", "BlinderKitten ✔") } else { tr("BlinderKitten – Verbindung prüfen", "BlinderKitten – check the connection") };
                egui::CollapsingHeader::new(egui::RichText::new(title).strong().color(if done { egui::Color32::LIGHT_GREEN } else { egui::Color32::LIGHT_RED }))
                    .id_salt("connection")
                    .default_open(!done)
                    .open((was_done != Some(done)).then_some(!done))
                    .show(ui, |ui| {
                        ui.label(trf!("Art-Net-Empfang auf Port {}", "Art-Net received on port {}", listen_port));
                        for (pa, source, age) in &universes {
                            let color = if *age < 1.0 { egui::Color32::LIGHT_GREEN } else { egui::Color32::GRAY };
                            ui.colored_label(color, trf!("Universe {}  ·  {}  ·  vor {:.1}s", "Universe {}  ·  {}  ·  {:.1}s ago", format_port_address(*pa), source, age));
                        }
                        if let Some(a) = super::onboarding::steps(ui, &state, &link, bk.as_ref(), listen_port, receiving, false) {
                            setup_action = Some(a);
                        }
                        if ui.button(tr("Erste Schritte öffnen …", "Open getting started …")).clicked() {
                            super::onboarding::open(ui.ctx());
                        }
                    });


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

    match left {
        Some(left) => rects.push(left.response.rect),
        None => {
            // Fully in: a small handle to bring it back.
            let handle = egui::Area::new("sidebar_handle".into()).fixed_pos(egui::pos2(8.0, 8.0)).show(ctx, |ui| {
                if ui.button("»").on_hover_text(tr("Seitenleiste einblenden (Tab)", "Show the sidebar (Tab)")).clicked() {
                    expanded = true;
                }
            });
            rects.push(handle.response.rect);
        }
    }
    *sidebar_hidden = !expanded || hide;

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
    let object = state.selected_object.clone().filter(|key| split_key(key).is_some_and(|(f, _)| project.objects.iter().any(|o| o.id == f)));
    let mut open = object.is_some();
    let right = egui::Panel::right("object").resizable(true).default_size(340.0).show_collapsible(&mut root, &mut open, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(key) = &object
                && object_editor(ui, &state, &mut project, key, &stage_objects, &scene_rt)
            {
                close = true;
            }
        });
    });
    if let Some(right) = right {
        rects.push(right.response.rect);
    }
    // Dragged shut.
    close |= object.is_some() && !open;

    // Settings of the selected fixture's type, in their own panel on the right.
    let mut open = !state.selection.is_empty();
    let had_selection = open;
    let right = egui::Panel::right("fixture").resizable(true).default_size(340.0).show_collapsible(&mut root, &mut open, |ui| {
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
                    library_row(ui, &state, &project, ty, &mut library);
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
                        project.library_links.remove(&ty.short_name);
                        let ids: Vec<i64> = state.show.fixtures.iter().filter(|g| g.fixture_type == f.fixture_type).map(|g| g.id).collect();
                        project.fixtures.retain(|id, _| !ids.contains(id));
                    }
                }
            });
    });
    if let Some(right) = right {
        rects.push(right.response.rect);
    }
    close |= had_selection && !open;
    if ui_rects.0 != rects {
        ui_rects.0 = rects;
    }
    if close {
        pick = Some(Pick::Clear);
    }

    match setup_action {
        Some(SetupAction::Edit(edit, offline)) => {
            // Mirrors set up for the port Tapetum listens on: that one is the project's from now on.
            if let Edit::AddMirrors { listen_port } = edit {
                project.artnet.listen_port = listen_port;
            }
            link.start(&state, edit, offline);
        }
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
        // A fixture type's settings changed here: note when (taking a library
        // file over sets that file's time itself).
        let now = tapetum_project::now();
        let types: std::collections::BTreeSet<String> = project.fixture_types.keys().chain(state.project.fixture_types.keys()).cloned().collect();
        for ty in types {
            if project.fixture_types.get(&ty) != state.project.fixture_types.get(&ty) && project.fixture_types_changed.get(&ty) == state.project.fixture_types_changed.get(&ty) {
                project.fixture_types_changed.insert(ty, now);
            }
        }
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
                    let locked = over.locked.unwrap_or(false);
                    if lock_toggle(ui, locked)
                        .on_hover_text(if locked {
                            tr("Gesperrt: im 3D-Fenster nicht auswählbar. Klick entsperrt.", "Locked: can't be picked in the 3D view. Click to unlock.")
                        } else {
                            tr("Sperren: im 3D-Fenster nicht mehr auswählbar (etwa den Zuschauerraum).", "Lock: no longer picked in the 3D view (the auditorium, say).")
                        })
                        .clicked()
                    {
                        over.locked = (!locked).then_some(true);
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

/// A padlock, drawn (the font's emoji looked like a note): closed and amber
/// when locked, open and grey when not.
fn lock_toggle(ui: &mut egui::Ui, locked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 16.0), egui::Sense::click());
    let color = if locked {
        egui::Color32::from_rgb(255, 194, 77)
    } else if resp.hovered() {
        egui::Color32::LIGHT_GRAY
    } else {
        egui::Color32::from_gray(110)
    };
    let painter = ui.painter();
    // On whole pixels: a 10 × 7 body, the shackle one even path above its middle.
    let (x, y) = (rect.left().round(), rect.top().round());
    let body = egui::Rect::from_min_max(egui::pos2(x + 2.0, y + 8.0), egui::pos2(x + 12.0, y + 15.0));
    painter.rect_filled(body, egui::CornerRadius::same(2), color);
    let (cx, r) = (x + 7.0, 2.75);
    // Unlocked, the shackle is lifted: its left leg ends above the body, the right one stays in.
    let lift = if locked { 0.0 } else { 2.5 };
    let shoulder = y + 6.0 - lift;
    let left_end = if locked { body.top() + 1.0 } else { shoulder + 1.0 };
    let mut path = vec![egui::pos2(cx - r, left_end), egui::pos2(cx - r, shoulder)];
    path.extend((1..16).map(|i| {
        let a = std::f32::consts::PI * i as f32 / 16.0;
        egui::pos2(cx - r * a.cos(), shoulder - r * a.sin())
    }));
    path.push(egui::pos2(cx + r, shoulder));
    path.push(egui::pos2(cx + r, body.top() + 1.0));
    painter.add(egui::Shape::line(path, egui::Stroke::new(1.5, color)));
    resp
}

/// "3 days ago", or the date for older times (Unix seconds; 0 = unknown).
fn ago(t: u64) -> String {
    if t == 0 {
        return tr("älter", "older").into();
    }
    let s = tapetum_project::now().saturating_sub(t);
    match s {
        0..60 => tr("gerade eben", "just now").into(),
        60..3600 => trf!("vor {} min", "{} min ago", s / 60),
        3600..86400 => trf!("vor {} h", "{} h ago", s / 3600),
        86400..2_592_000 => match s / 86400 {
            1 => tr("gestern", "yesterday").into(),
            d => trf!("vor {d} Tagen", "{d} days ago", d = d),
        },
        _ => {
            // The date (UTC), from days since 1970 (Howard Hinnant's civil_from_days).
            let z = (t / 86400) as i64 + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            if crate::i18n::english() { format!("{y}-{m:02}-{d:02}") } else { format!("{d}.{m}.{y}") }
        }
    }
}

/// After exporting a fixture: where it went, and the .mochi next to it.
fn export_notice(ctx: &egui::Context) {
    let id = egui::Id::new("export_notice");
    let Some((name, file, mochi_error)) = ctx.data(|d| d.get_temp::<(String, std::path::PathBuf, Option<String>)>(id)) else { return };
    let mut close = false;
    egui::Window::new(egui::RichText::new(tr("Gerät exportiert", "Fixture exported")).size(18.0).strong())
        .id(id.with("window"))
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(440.0);
            let dir = file.parent().map(|d| d.display().to_string()).unwrap_or_default();
            ui.label(trf!("„{name}“ ist gespeichert unter", "“{name}” is saved in", name = name));
            ui.label(egui::RichText::new(&dir).monospace());
            ui.add_space(6.0);
            match &mochi_error {
                None => ui.label(tr(
                    "Hinweis: Im selben Ordner liegt zusätzlich eine .mochi-Datei. Sie lässt sich in einem anderen Projekt in BlinderKitten importieren (File → Import…) – Tapetum übernimmt dann diese Einstellungen für das Gerät.",
                    "Note: the same folder also holds a .mochi file. It can be imported into another project in BlinderKitten (File → Import…) – Tapetum then takes these settings over for the fixture.",
                )),
                Some(e) => ui.colored_label(egui::Color32::LIGHT_YELLOW, trf!("Die .mochi-Datei für BlinderKitten konnte nicht geschrieben werden: {e}", "The .mochi file for BlinderKitten couldn't be written: {e}", e = e)),
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("OK").strong()).clicked() {
                    close = true;
                }
                if ui.button(tr("Im Dateimanager zeigen", "Show in file manager")).clicked() {
                    crate::example::reveal(&file);
                }
            });
        });
    if close {
        ctx.data_mut(|d| d.remove::<(String, std::path::PathBuf, Option<String>)>(id));
    }
}

/// Newer fixture definitions in the library than in this project: in the
/// middle of the screen, to take over all, some or none of them.
fn library_offers(ctx: &egui::Context, offers: &mut LibraryUpdates, project: &mut tapetum_project::Project, state: &mut ShowState) {
    if offers.offers.is_empty() {
        return;
    }
    let mut done: Option<bool> = None;
    egui::Window::new(egui::RichText::new(tr("Neuere Gerätedefinitionen", "Newer fixture definitions")).size(18.0).strong())
        .id(egui::Id::new("library_offers"))
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(tr(
                "In der Gerätebibliothek gibt es für Geräte dieses Projekts neuere Einstellungen. Welche sollen übernommen werden? Die anderen bleiben, wie sie sind.",
                "The fixture library has newer settings for fixtures of this project. Which shall be taken over? The others stay as they are.",
            ));
            ui.add_space(6.0);
            egui::Grid::new("offers").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
                ui.label("");
                ui.strong(tr("Bibliothek", "Library"));
                ui.strong(tr("Projekt", "Project"));
                ui.end_row();
                for (u, on) in &mut offers.offers {
                    ui.checkbox(on, &u.type_name);
                    ui.label(ago(u.library));
                    ui.label(ago(u.project));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.small_button(tr("Alle auswählen", "Select all")).clicked() {
                    offers.offers.iter_mut().for_each(|(_, on)| *on = true);
                }
                if ui.small_button(tr("Keine auswählen", "Select none")).clicked() {
                    offers.offers.iter_mut().for_each(|(_, on)| *on = false);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let n = offers.offers.iter().filter(|(_, on)| *on).count();
                if ui.add_enabled(n > 0, egui::Button::new(egui::RichText::new(trf!("{n} übernehmen", "Take over {n}", n = n)).strong())).clicked() {
                    done = Some(true);
                }
                if ui.button(tr("Ignorieren", "Ignore")).on_hover_text(tr("Diese Fassungen werden nicht mehr angeboten; neuere schon.", "These versions aren't offered again; newer ones are.")).clicked() {
                    done = Some(false);
                }
            });
        });
    let Some(take) = done else { return };
    let mut taken = Vec::new();
    for (u, on) in std::mem::take(&mut offers.offers) {
        // What isn't taken over is declined in this version.
        if take && on && crate::fixture_library::take_over(project, &u.short_name, &u.file, &state.project_dir()).is_ok() {
            taken.push(u.type_name);
        } else {
            crate::fixture_library::decline(project, &u);
        }
    }
    if !taken.is_empty() {
        state.status = Some(trf!("Aus der Gerätebibliothek übernommen: {}", "Taken over from the fixture library: {}", taken.join(", ")));
    }
}

/// Where this type's settings stand with the fixture library, and its buttons.
fn library_row(ui: &mut egui::Ui, state: &ShowState, project: &tapetum_project::Project, ty: &bk_olga::FixtureType, library: &mut LibraryJobs) {
    let current = project.fixture_types.get(&ty.short_name).cloned().unwrap_or_default();
    let name_of = |p: &str| std::path::Path::new(p).file_name().map_or(p.to_string(), |n| n.to_string_lossy().into_owned());
    let status = match project.library_links.get(&ty.short_name) {
        Some(l) if crate::fixture_library::fingerprint(&current) == l.applied => trf!("aus der Gerätebibliothek ({})", "from the fixture library ({})", name_of(&l.file)),
        Some(l) => trf!("aus der Gerätebibliothek ({}), hier geändert", "from the fixture library ({}), changed here", name_of(&l.file)),
        None if crate::fixture_library::dir().is_some_and(|d| crate::fixture_library::file_for(&d, &ty.name).is_file()) => tr("in der Gerätebibliothek vorhanden – „Laden“ übernimmt sie", "in the fixture library – “Load” takes it over").to_string(),
        None => tr("nicht in der Gerätebibliothek", "not in the fixture library").to_string(),
    };
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(egui::Color32::GRAY, status);
        ui.add_enabled_ui(!library.busy(), |ui| {
            if ui
                .small_button(tr("Exportieren", "Export"))
                .on_hover_text(tr(
                    "Speichert die Einstellungen dieses Typs in der Gerätebibliothek. Neue Projekte mit einem Gerätetyp gleichen Namens übernehmen sie automatisch.\n\nDaneben liegt der Gerätetyp selbst als .mochi für BlinderKitten (dort File → Import…). Gibt es in der Show schon einen Typ dieses Namens, benennt BlinderKitten den importierten um – dann passt er nicht mehr zu den Einstellungen hier.",
                    "Saves this type's settings in the fixture library. New projects with a fixture type of the same name take them over automatically.\n\nNext to it, the fixture type itself as a .mochi for BlinderKitten (File → Import… there). If the show already has a type of that name, BlinderKitten renames the imported one – then it no longer matches the settings here.",
                ))
                .clicked()
            {
                let modified = project.fixture_types_changed.get(&ty.short_name).copied().unwrap_or_else(tapetum_project::now);
                library.export(ty.short_name.clone(), ty.name.clone(), current.clone(), state.project_dir(), state.show_path.clone(), modified);
            }
            if ui.small_button(tr("Laden …", "Load …")).on_hover_text(tr("Einstellungen aus einer Gerätedatei übernehmen (ersetzt die jetzigen).", "Take over the settings from a fixture file (replaces the current ones).")).clicked() {
                library.pick(ty.short_name.clone());
            }
        });
    });
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


#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn tab_goes_to_the_sidebar_not_to_egui() {
        let mut world = World::new();
        world.init_resource::<TabKey>();
        let input = bevy_egui::EguiInput(egui::RawInput { events: vec![press(egui::Key::Tab, egui::Modifiers::NONE), press(egui::Key::A, egui::Modifiers::NONE), press(egui::Key::Tab, egui::Modifiers::SHIFT)], ..Default::default() });
        let entity = world.spawn((bevy_egui::EguiContext::default(), input, bevy_egui::PrimaryEguiContext)).id();
        world.run_system_cached(take_tab).unwrap();
        assert!(world.resource::<TabKey>().0);
        // Shift+Tab and the other keys stay with egui.
        let events = &world.get::<bevy_egui::EguiInput>(entity).unwrap().events;
        assert_eq!(events.len(), 2);
        assert!(!events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Tab, modifiers, .. } if !modifiers.any())));
        world.run_system_cached(take_tab).unwrap();
        assert!(!world.resource::<TabKey>().0, "only in the frame it was pressed");
    }
}
