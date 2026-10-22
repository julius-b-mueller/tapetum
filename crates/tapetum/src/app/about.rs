//! The menu in the sidebar header (language, about) and the about window
//! with the open source licenses.

use std::sync::OnceLock;

use bevy_egui::egui;

use crate::i18n::{Language, UserSettings, tr};

/// Versions Tapetum was tested with.
pub const TESTED_BLINDERKITTEN: &str = "1.0.1b98";
pub const TESTED_INGETRAZO: &str = "0.5.7";
pub const WEBSITE: &str = "https://tapetum.org";
pub const REPOSITORY: &str = "https://github.com/julius-b-mueller/tapetum";

const LICENSES: &str = include_str!("../../assets/third-party-licenses.txt");
const OWN_LICENSE: &str = include_str!("../../../../LICENSE");

static OPEN_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn open_at_start() {
    OPEN_AT_START.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn about_id() -> egui::Id {
    egui::Id::new("about_open")
}

/// The "☰" menu: language and the about window.
pub fn menu(ui: &mut egui::Ui) {
    ui.menu_button("☰", |ui| {
        ui.label(tr("Sprache", "Language"));
        for l in Language::ALL {
            if ui.radio(Language::current() == l, l.name()).clicked() {
                crate::i18n::set(l);
                let mut s = UserSettings::load();
                s.language = Some(l);
                s.save();
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tr("Andere Show öffnen …", "Open another show …")).clicked() {
            open_other_show();
            ui.close();
        }
        if ui.button(tr("Über Tapetum …", "About Tapetum …")).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(about_id(), true));
            ui.close();
        }
    })
    .response
    .on_hover_text(tr("Menü", "Menu"));
}

/// Picks a show and starts Tapetum anew with it (everything is saved as it changes).
fn open_other_show() {
    std::thread::spawn(|| {
        let Some(show) = crate::pick_show() else { return };
        let mut s = UserSettings::load();
        s.last_show = Some(show.clone());
        s.save();
        if let Ok(exe) = std::env::current_exe()
            && std::process::Command::new(exe).arg(&show).spawn().is_ok()
        {
            std::process::exit(0);
        }
    });
}

pub fn window(ctx: &egui::Context, logo: egui::TextureId) {
    let mut open = ctx.data(|d| d.get_temp::<bool>(about_id()).unwrap_or(false)) || OPEN_AT_START.swap(false, std::sync::atomic::Ordering::Relaxed);
    if !open {
        return;
    }
    egui::Window::new(tr("Über Tapetum", "About Tapetum"))
        .open(&mut open)
        .collapsible(false)
        .default_width(520.0)
        .default_height(560.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Image::new((logo, egui::vec2(88.0, 88.0))));
                ui.vertical(|ui| {
                    ui.heading(format!("Tapetum {}", env!("CARGO_PKG_VERSION")));
                    ui.label(tr("3D-Lichtvisualisierung für BlinderKitten", "3D lighting visualizer for BlinderKitten"));
                    ui.hyperlink(WEBSITE);
                    ui.hyperlink(REPOSITORY);
                });
            });
            ui.add_space(6.0);
            ui.label(trf!(
                "Getestet mit BlinderKitten {} und IngeTrazo {} (.igz-Dateiformat bis 3). Andere Versionen funktionieren wahrscheinlich, sind aber nicht geprüft.",
                "Tested with BlinderKitten {} and IngeTrazo {} (.igz file format up to 3). Other versions probably work but haven't been checked.",
                TESTED_BLINDERKITTEN,
                TESTED_INGETRAZO
            ));
            ui.add_space(4.0);
            ui.label(tr(
                "Tapetum wurde vollständig mit KI programmiert („vibecoded“, mit Claude von Anthropic). Das Logo wurde mit ChatGPT generiert.",
                "Tapetum was written entirely with AI (“vibe-coded”, with Claude by Anthropic). The logo was generated with ChatGPT.",
            ));
            ui.add_space(4.0);
            ui.label(tr(
                "Freie Software unter der MIT-Lizenz. Ohne jede Gewährleistung.",
                "Free software under the MIT License. Without any warranty.",
            ));
            ui.separator();
            egui::CollapsingHeader::new(tr("Lizenz von Tapetum (MIT)", "Tapetum's license (MIT)")).id_salt("own_license").show(ui, |ui| {
                lines(ui, "own", OWN_LICENSE);
            });
            egui::CollapsingHeader::new(tr("Open-Source-Lizenzen der verwendeten Bibliotheken", "Open source licenses of the libraries used"))
                .id_salt("licenses")
                .show(ui, |ui| {
                    lines(ui, "third_party", LICENSES);
                });
        });
    ctx.data_mut(|d| d.insert_temp(about_id(), open));
}

/// A long text, laid out only where it is visible.
fn lines(ui: &mut egui::Ui, id: &str, text: &'static str) {
    static OWN: OnceLock<Vec<&'static str>> = OnceLock::new();
    static THIRD: OnceLock<Vec<&'static str>> = OnceLock::new();
    let cell = if id == "own" { &OWN } else { &THIRD };
    let all = cell.get_or_init(|| text.lines().collect());
    let row = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both().id_salt(id).max_height(320.0).auto_shrink([false, true]).show_rows(ui, row, all.len(), |ui, range| {
        for l in &all[range] {
            ui.add(egui::Label::new(egui::RichText::new(*l).monospace().small()).extend());
        }
    });
}
