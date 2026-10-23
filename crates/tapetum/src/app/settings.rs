//! The settings window (☰ → Settings …): what isn't needed all the time.

use bevy_egui::egui;
use tapetum_project::{Project, Quality};

use crate::i18n::tr;

fn open_id() -> egui::Id {
    egui::Id::new("settings_open")
}

static OPEN_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Open the settings at start (for screenshots).
pub fn open_at_start() {
    OPEN_AT_START.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn open(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(open_id(), true));
}

/// A folder chosen in the dialog, on its way from the dialog's thread.
static PICKED: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Where exported fixtures go and where new projects look for them.
fn library_folder(ui: &mut egui::Ui) {
    if let Some(dir) = PICKED.lock().ok().and_then(|mut p| p.take()) {
        crate::fixture_library::set_dir(Some(dir));
    }
    ui.strong(tr("Gerätebibliothek", "Fixture library"));
    ui.colored_label(egui::Color32::GRAY, tr("Exportierte Geräte landen hier. Projekte übernehmen sie automatisch für Gerätetypen gleichen Namens, die dort noch nicht eingestellt sind.", "Exported fixtures go here. Projects take them over automatically for fixture types of the same name not set up there yet."));
    let dir = crate::fixture_library::dir();
    ui.label(dir.as_ref().map_or_else(|| tr("kein Ordner", "no folder").to_string(), |d| d.display().to_string()));
    ui.horizontal(|ui| {
        if ui.button(tr("Ändern …", "Change …")).clicked() {
            std::thread::spawn(|| {
                if let Some(d) = rfd::FileDialog::new().set_title(tr("Ordner der Gerätebibliothek", "Fixture library folder")).pick_folder()
                    && let Ok(mut p) = PICKED.lock()
                {
                    *p = Some(d);
                }
            });
        }
        if let Some(d) = &dir
            && ui.button(tr("Im Dateimanager zeigen", "Show in file manager")).clicked()
        {
            let _ = std::fs::create_dir_all(d);
            crate::example::reveal(d);
        }
        let custom = crate::i18n::UserSettings::load().fixture_library.is_some();
        if custom && ui.button(tr("Standard", "Default")).on_hover_text(crate::fixture_library::default_dir().map(|d| d.display().to_string()).unwrap_or_default()).clicked() {
            crate::fixture_library::set_dir(None);
        }
    });
}

pub fn window(ctx: &egui::Context, project: &mut Project, spacemouse: Option<String>) {
    let mut open = ctx.data(|d| d.get_temp::<bool>(open_id()).unwrap_or(false)) || OPEN_AT_START.swap(false, std::sync::atomic::Ordering::Relaxed);
    if !open {
        return;
    }
    egui::Window::new(tr("Einstellungen", "Settings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(380.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
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
                ui.horizontal(|ui| {
                    ui.label(tr("Farbe des Haze", "Haze colour"));
                    ui.color_edit_button_rgb(&mut r.haze.color);
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Farbe des Saallichts", "House light colour"));
                    let mut c = r.house_color.unwrap_or_else(|| bk_olga::light::kelvin_to_rgb(3000.0));
                    if ui.color_edit_button_rgb(&mut c).changed() {
                        r.house_color = Some(c);
                    }
                    if r.house_color.is_some() && ui.small_button(tr("Warmweiß", "Warm white")).on_hover_text(tr("Zurück auf warmweißes Arbeitslicht (3000 K)", "Back to warm white work light (3000 K)")).clicked() {
                        r.house_color = None;
                    }
                });
                ui.separator();
                library_folder(ui);
                ui.separator();
                spacemouse_section(ui, &mut project.spacemouse, spacemouse);
            });
        });
    ctx.data_mut(|d| d.insert_temp(open_id(), open));
}

pub fn quality_label(q: Quality) -> &'static str {
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
