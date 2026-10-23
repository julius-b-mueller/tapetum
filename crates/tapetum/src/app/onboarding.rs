//! Getting started: the connection to BlinderKitten step by step, and the
//! built-in example. In full in a window in the middle of the screen; the
//! sidebar keeps a short version of the steps.

use bevy_egui::egui;
use bk_olga::InterfaceKind;

use super::state::{BkLink, ShowState};
use crate::bk_link::{Edit, HostInfo};
use crate::i18n::tr;
use crate::routing::format_port_address;

pub enum SetupAction {
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

fn hint(ui: &mut egui::Ui, detailed: bool, text: impl Into<String>) {
    if !detailed {
        return;
    }
    ui.indent("hint", |ui| {
        ui.colored_label(HINT, text.into());
    });
}

/// Checklist for connecting BK, with everything that has to be done by hand in BK.
/// The show's DMX interfaces with their mirrors (as text), and whether all of
/// them are mirrored to `listen_port`.
fn mirrors(show: &bk_olga::Show, listen_port: u16) -> (Vec<(String, Option<String>)>, bool, bool) {
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
    let all_mirrored = !mirrors.is_empty() && mirrors.iter().all(|(_, m)| m.is_some());
    let port_ok = show.interfaces.iter().filter(|i| bk_olga::mirror::is_mirror_name(&i.name)).all(|i| match &i.kind {
        InterfaceKind::ArtNet(a) => a.port == listen_port,
        _ => true,
    });
    (mirrors, all_mirrored, port_ok)
}

/// Everything set up and light arriving?
pub fn all_done(state: &ShowState, bk: Option<&HostInfo>, listen_port: u16, receiving: bool) -> bool {
    let (_, all_mirrored, port_ok) = mirrors(&state.show, listen_port);
    bk.is_some_and(|b| b.has_open(&state.show_path)) && all_mirrored && port_ok && receiving
}

/// The four steps of the connection to BlinderKitten, each with what is wrong
/// and the button that fixes it; `detailed` adds the explanations.
pub fn steps(ui: &mut egui::Ui, state: &ShowState, link: &BkLink, bk: Option<&HostInfo>, listen_port: u16, receiving: bool, detailed: bool) -> Option<SetupAction> {
    let show = &state.show;
    let remote_port = state.project.artnet.bk_remote_port;
    let mut action = None;
    let (mirrors, all_mirrored, mirror_port_ok) = mirrors(show, listen_port);
    let any_mirror = mirrors.iter().any(|(_, m)| m.is_some());
    let open_here = bk.is_some_and(|b| b.has_open(&state.show_path));
    let blockers: Vec<&str> = show
        .interfaces
        .iter()
        .filter(|i| matches!(&i.kind, InterfaceKind::ArtNet(a) if a.binds_6454))
        .map(|i| i.name.as_str())
        .collect();
    {
        {
            step(ui, bk.is_some(), tr("1. OSC Remote Control in BlinderKitten einschalten", "1. Turn on OSC Remote Control in BlinderKitten"));
            match bk {
                Some(b) => hint(ui, detailed, trf!("BlinderKitten {} antwortet auf Port {remote_port}.", "BlinderKitten {} answers on port {remote_port}.", b.version, remote_port = remote_port)),
                None => {
                    hint(
                        ui,
                        detailed,
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
                    hint(ui, detailed, trf!("Geöffnet ist {open}.", "Open is {open}.", open = open));
                    hint(ui, detailed, trf!("Gebraucht wird {}.", "Needed is {}.", state.show_path.display()));
                }
                Some(b) if b.unsaved => hint(ui, detailed, tr("Ungespeicherte Änderungen speichert der Einrichten-Knopf automatisch mit.", "The set-up button saves unsaved changes along with it.")),
                Some(_) => {}
                None => hint(ui, detailed, format!("{}", state.show_path.display())),
            }

            step(ui, all_mirrored && mirror_port_ok, tr("3. Tapetum-Interfaces anlegen", "3. Add the Tapetum interfaces"));
            let wanted = state.project.artnet.listen_port;
            if wanted != listen_port {
                ui.colored_label(
                    BAD,
                    trf!(
                        "Port {wanted} ist von einem anderen Programm belegt. Tapetum empfängt deshalb auf Port {listen_port}.",
                        "Port {wanted} is taken by another program, so Tapetum receives on port {listen_port}.",
                        wanted = wanted,
                        listen_port = listen_port
                    ),
                );
            }
            for (orig, m) in &mirrors {
                match m {
                    Some(m) => hint(ui, detailed, format!("{orig} » {m}")),
                    None => hint(ui, detailed, trf!("{orig} – noch nicht gespiegelt", "{orig} – not mirrored yet", orig = orig)),
                }
            }
            if !mirror_port_ok {
                ui.colored_label(BAD, trf!("Die Tapetum-Interfaces senden nicht an Port {listen_port} – bitte aktualisieren.", "The Tapetum interfaces don't send to port {listen_port} – please update them.", listen_port = listen_port));
            }
            ui.add_enabled_ui(!link.busy(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    let label = if any_mirror && !mirror_port_ok {
                        trf!("Tapetum-Interfaces auf Port {listen_port} umstellen", "Move the Tapetum interfaces to port {listen_port}", listen_port = listen_port)
                    } else if any_mirror {
                        tr("Tapetum-Interfaces aktualisieren", "Update the Tapetum interfaces").to_string()
                    } else {
                        tr("Tapetum-Interfaces in BK anlegen", "Add the Tapetum interfaces in BK").to_string()
                    };
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
                hint(ui, detailed, tr("In BlinderKitten danach nichts weiter einstellen: Die Show ist schon gespeichert. Die Tapetum-Interfaces bitte nicht umbenennen.", "Nothing more to set in BlinderKitten afterwards: the show is already saved. Please don't rename the Tapetum interfaces."));
            }

            step(ui, receiving, tr("4. Art-Net kommt an", "4. Art-Net arrives"));
            if !receiving {
                if any_mirror {
                    hint(ui, detailed, tr("Kommt nichts an, in BlinderKitten prüfen:", "If nothing arrives, check in BlinderKitten:"));
                    hint(ui, detailed, tr("• Die Tapetum-Interfaces und die Original-Interfaces sind aktiviert (Haken in der Interface-Liste).", "• The Tapetum interfaces and the original interfaces are enabled (tick in the interface list)."));
                    hint(ui, detailed, tr("• Bei den Original-Interfaces steht unter „Pass-through“ ein „Output Universe“ auf das Tapetum-Interface.", "• The original interfaces have an “Output Universe” under “Pass-through” pointing to the Tapetum interface."));
                    hint(ui, detailed, tr("• Es ist eine Cue aktiv oder im Programmer ein Wert gesetzt (ohne Werte sendet BK nur Nullen).", "• A cue is running or the programmer has a value (without values BK sends only zeros)."));
                }
                if cfg!(windows) {
                    hint(ui, detailed, trf!("• Windows-Firewall: eingehendes UDP auf Port {listen_port} für Tapetum erlauben.", "• Windows firewall: allow incoming UDP on port {listen_port} for Tapetum.", listen_port = listen_port));
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
                hint(ui, detailed, tr("Entweder in BK bei diesen Interfaces Input und „Force src port“ ausschalten, oder den Visualizer auf einen anderen Port legen:", "Either turn off Input and “Force src port” for these interfaces in BK, or move Tapetum to another port:"));
                if ui.add_enabled(!link.busy(), egui::Button::new(tr("Tapetum auf einen freien Port umstellen", "Move Tapetum to a free port"))).clicked() {
                    action = Some(SetupAction::ChangePort(bk_artnet::free_port(std::net::Ipv4Addr::UNSPECIFIED, listen_port.saturating_add(1))));
                }
            }
        }
    }
    action
}

const EXAMPLE_ACCENT: egui::Color32 = egui::Color32::from_rgb(255, 194, 77);

/// On the built-in example: save it into a folder to try it with BlinderKitten
/// and IngeTrazo. On a saved copy: where its files are and what to do with them.
pub fn example_banner(ui: &mut egui::Ui, show: &std::path::Path, project: &std::path::Path) {
    let internal = crate::example::is_internal(show);
    if !internal && !crate::example::is_saved_copy(show) {
        return;
    }
    let dismissed = egui::Id::new("example_banner_dismissed");
    if ui.ctx().data(|d| d.get_temp::<bool>(dismissed).unwrap_or(false)) {
        return;
    }
    egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.5, EXAMPLE_ACCENT)).fill(egui::Color32::from_rgba_unmultiplied(255, 194, 77, 18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let name = crate::example::NAME;
        if internal {
            ui.colored_label(EXAMPLE_ACCENT, egui::RichText::new(tr("Beispiel", "Example")).strong().size(15.0));
            ui.label(tr(
                "Ein kleines Theater mit Lampen und Cues. Speichere es in einen Ordner deiner Wahl – dann kannst du die Show in BlinderKitten und das Bühnenbild in IngeTrazo öffnen und alles live ausprobieren.",
                "A small theatre with fixtures and cues. Save it into a folder of your choice – then you can open the show in BlinderKitten and the set in IngeTrazo and try everything live.",
            ));
            ui.add_space(2.0);
            let button = egui::Button::new(egui::RichText::new(tr("Beispiel speichern unter …", "Save the example to …")).strong().color(egui::Color32::BLACK)).fill(EXAMPLE_ACCENT);
            if ui.add(button).clicked() {
                save_example(project.to_path_buf());
            }
        } else {
            ui.horizontal(|ui| {
                ui.colored_label(EXAMPLE_ACCENT, egui::RichText::new(tr("Beispiel gespeichert", "Example saved")).strong().size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text(tr("Ausblenden", "Hide")).clicked() {
                        ui.ctx().data_mut(|d| d.insert_temp(dismissed, true));
                    }
                });
            });
            if let Some(dir) = show.parent() {
                ui.weak(dir.display().to_string());
            }
            ui.label(trf!(
                "Öffne jetzt aus diesem Ordner:\n• {name}.olga mit BlinderKitten\n• {name}.igz mit IngeTrazo\nÄnderungen in beiden Programmen siehst du hier sofort.",
                "Now open from this folder:\n• {name}.olga with BlinderKitten\n• {name}.igz with IngeTrazo\nChanges in both programs show here at once.",
                name = name
            ));
            let button = egui::Button::new(egui::RichText::new(tr("Im Dateimanager zeigen", "Show in file manager")).strong().color(egui::Color32::BLACK)).fill(EXAMPLE_ACCENT);
            if ui.add(button).clicked() {
                crate::example::reveal(show);
            }
        }
    });
    ui.add_space(4.0);
}

/// Asks for a folder, writes the example there and opens it from there.
fn save_example(project: std::path::PathBuf) {
    std::thread::spawn(move || {
        let Some(dir) = rfd::FileDialog::new().set_title(tr("Ordner für das Beispiel wählen", "Choose a folder for the example")).pick_folder() else { return };
        let existing = crate::example::existing_in(&dir);
        if !existing.is_empty() {
            let names: Vec<String> = existing.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            let answer = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(tr("Dateien ersetzen?", "Replace files?"))
                .set_description(trf!("In diesem Ordner gibt es schon:\n{}\n\nErsetzen?", "This folder already has:\n{}\n\nReplace them?", names.join("\n")))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if answer != rfd::MessageDialogResult::Yes {
                return;
            }
        }
        match crate::example::save_to(&dir, &project) {
            Ok(show) => super::about::restart_with(&show),
            Err(e) => {
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title(tr("Speichern fehlgeschlagen", "Saving failed"))
                    .set_description(e.to_string())
                    .show();
            }
        }
    });
}

fn ids() -> (egui::Id, egui::Id, egui::Id) {
    let id = egui::Id::new("getting_started");
    (id.with("open"), id.with("auto"), id.with("seen"))
}

/// Opens the getting-started window (from the menu or the sidebar).
pub fn open(ctx: &egui::Context) {
    let (open, auto, _) = ids();
    ctx.data_mut(|d| {
        d.insert_temp(open, true);
        d.insert_temp(auto, false);
    });
}

/// The getting-started window in the middle of the screen. It comes up by
/// itself while the connection isn't complete and goes once everything works.
pub fn window(ctx: &egui::Context, state: &ShowState, link: &BkLink, bk: Option<&HostInfo>, listen_port: u16, receiving: bool) -> Option<SetupAction> {
    let (open_id, auto_id, seen_id) = ids();
    let done = all_done(state, bk, listen_port, receiving);
    // The example in Tapetum's folder always gets it: saving it is the next step.
    let example = crate::example::is_internal(&state.show_path);
    // A moment after the start (BK needs a poll to answer): open if there is something to do.
    let seen: f64 = ctx.data(|d| d.get_temp(seen_id)).unwrap_or_else(|| {
        let t = ctx.input(|i| i.time);
        ctx.data_mut(|d| d.insert_temp(seen_id, t));
        t
    });
    if seen >= 0.0 && ctx.input(|i| i.time) - seen > 1.5 {
        ctx.data_mut(|d| d.insert_temp(seen_id, -1.0));
        if !done || example {
            ctx.data_mut(|d| {
                d.insert_temp(open_id, true);
                d.insert_temp(auto_id, true);
            });
        }
    }
    let mut open = ctx.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    let auto = ctx.data(|d| d.get_temp::<bool>(auto_id).unwrap_or(false));
    // Opened by itself and now all works (and nothing to tell about the example): away with it.
    if open && auto && done && !example {
        open = false;
    }
    if !open {
        ctx.data_mut(|d| d.insert_temp(open_id, false));
        return None;
    }
    let mut action = None;
    let mut close = false;
    egui::Window::new(egui::RichText::new(tr("Erste Schritte", "Getting started")).size(20.0).strong())
        .id(egui::Id::new("getting_started_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(egui::RichText::new(tr(
                "Verbinde Tapetum mit BlinderKitten – dann siehst du jede Lichtänderung sofort, und Änderungen am Bühnenbild in IngeTrazo erscheinen beim Speichern.",
                "Connect Tapetum to BlinderKitten – then you see every change of light at once, and changes to the set in IngeTrazo show up as you save.",
            )).size(14.0));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.65).show(ui, |ui| {
                example_banner(ui, &state.show_path, &state.project_path);
                action = steps(ui, state, link, bk, listen_port, receiving, true);
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if done {
                    ui.colored_label(OK, egui::RichText::new(tr("✔ Alles verbunden.", "✔ All connected.")).strong());
                    if ui.button(tr("Los geht's", "Let's go")).clicked() {
                        close = true;
                    }
                } else if ui.button(tr("Später", "Later")).on_hover_text(tr("Wieder öffnen: in der Seitenleiste unter BlinderKitten oder unter ⚙ Einstellungen.", "Open again: in the sidebar under BlinderKitten or under ⚙ Settings.")).clicked() {
                    close = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.hyperlink_to(tr("Ausführliche Anleitung", "Full guide"), "https://tapetum.org/start.html");
                });
            });
        });
    ctx.data_mut(|d| d.insert_temp(open_id, open && !close));
    action
}
