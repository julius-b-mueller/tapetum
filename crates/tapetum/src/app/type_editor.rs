//! Editor for the settings of one fixture type (they apply to all its fixtures).

use crate::i18n::tr;
use bevy_egui::egui;
use bk_olga::{Attribute, FixtureType};
use tapetum_project::{BeamKind, CellMode, ColorSlot, FixtureOverride, Gobo, GoboSlot, LedLayout, LedShape, distribute};

use super::fixtures::{CLASSES, FixtureSpec, auto_beam, auto_leds, bk_range, class_from_label, resolve};
use super::motion::Limits;
use super::gobo::BUILTIN;

/// Things the editor can't do on its own.
#[derive(Default)]
pub struct Events {
    /// Physical ranges to write to BlinderKitten.
    pub ranges: Vec<(Attribute, [f32; 2])>,
    /// Gobo slot that wants a picture from a file dialog ([`FIXED_GOBO`] for the fixed gobo).
    pub import_gobo: Option<usize>,
}

/// Brush of the LED painter: cell id, 0 = body.
pub struct Painter {
    pub brush: u32,
}

const GRAY: egui::Color32 = egui::Color32::GRAY;

pub fn show(ui: &mut egui::Ui, ty: &FixtureType, devices: usize, o: &mut FixtureOverride, painter: &mut Painter) -> Events {
    let mut ev = Events::default();
    let spec = resolve(ty, o);
    ui.label(trf!("Gilt für alle {devices} Geräte vom Typ „{}“.", "Applies to all {devices} fixtures of type “{}”.", ty.name, devices = devices));

    egui::CollapsingHeader::new(tr("Gerät", "Fixture")).default_open(true).show(ui, |ui| device(ui, ty, &spec, o));
    egui::CollapsingHeader::new(tr("Strahl", "Beam")).default_open(true).show(ui, |ui| beam(ui, ty, &spec, o, &mut ev));
    if ty.has(&Attribute::Pan) || ty.has(&Attribute::Tilt) {
        egui::CollapsingHeader::new("Pan / Tilt").default_open(true).show(ui, |ui| {
            if ty.has(&Attribute::Pan) {
                range_row(ui, "Pan", ty, &Attribute::Pan, spec.pan_range, &mut o.pan_range, &mut ev);
            }
            if ty.has(&Attribute::Tilt) {
                range_row(ui, "Tilt", ty, &Attribute::Tilt, spec.tilt_range, &mut o.tilt_range, &mut ev);
            }
            ui.add_space(4.0);
            motion(ui, ty, &spec, o);
        });
    }
    egui::CollapsingHeader::new(tr("LEDs und Zellen", "LEDs and cells")).default_open(true).show(ui, |ui| leds(ui, ty, &spec, o, painter));
    if ty.has(&Attribute::ColorWheel) {
        egui::CollapsingHeader::new(tr("Farbrad", "Colour wheel")).default_open(true).show(ui, |ui| color_wheel(ui, o));
    }
    if ty.has(&Attribute::Gobo) {
        egui::CollapsingHeader::new(tr("Goborad", "Gobo wheel")).default_open(true).show(ui, |ui| gobo_wheel(ui, o, &mut ev));
    } else {
        egui::CollapsingHeader::new(tr("Gobo (fest eingesetzt)", "Gobo (fixed)")).default_open(o.fixed_gobo.is_some()).show(ui, |ui| fixed_gobo(ui, o, &mut ev));
    }
    if ty.has(&Attribute::Strobe) {
        egui::CollapsingHeader::new("Strobe").show(ui, |ui| {
            let mut hz = spec.tuning.strobe_hz;
            ui.horizontal(|ui| {
                ui.label(tr("Frequenz von", "Frequency from"));
                let a = ui.add(egui::DragValue::new(&mut hz[0]).range(0.1..=50.0).speed(0.1).suffix(" Hz"));
                ui.label(tr("bis", "to"));
                let b = ui.add(egui::DragValue::new(&mut hz[1]).range(0.1..=50.0).speed(0.1).suffix(" Hz"));
                if a.changed() || b.changed() {
                    o.strobe_hz = Some(hz);
                }
            });
        });
    }
    ev
}

fn device(ui: &mut egui::Ui, ty: &FixtureType, spec: &FixtureSpec, o: &mut FixtureOverride) {
    let auto = trf!("Automatisch ({})", "Automatic ({})", ty.classify().label());
    egui::ComboBox::from_label(tr("Bauform", "Kind"))
        .selected_text(o.class.clone().unwrap_or_else(|| auto.clone()))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut o.class, None, auto);
            for c in CLASSES {
                ui.selectable_value(&mut o.class, Some(c.label().to_string()), c.label());
            }
        });
    debug_assert!(o.class.as_deref().is_none_or(|c| class_from_label(c).is_some()));

    let mut lumens = spec.lumens;
    if ui.add(egui::Slider::new(&mut lumens, 50.0..=40000.0).logarithmic(true).text("Lumen")).changed() {
        o.lumens = Some(lumens);
    }
    let mut scale = spec.body_scale;
    if ui.add(egui::Slider::new(&mut scale, 0.3..=3.0).text(tr("Gehäusegröße", "Body size"))).changed() {
        o.body_scale = Some(scale);
    }
    let mut custom_white = o.white_kelvin.is_some();
    ui.horizontal(|ui| {
        if ui.checkbox(&mut custom_white, tr("Weiß-Farbtemperatur", "White colour temperature")).changed() {
            o.white_kelvin = custom_white.then_some(5600.0);
        }
        if let Some(k) = &mut o.white_kelvin {
            ui.add(egui::DragValue::new(k).range(1800.0..=10000.0).speed(20.0).suffix(" K"));
        } else {
            ui.colored_label(GRAY, tr("automatisch", "automatic"));
        }
    });
}

fn beam(ui: &mut egui::Ui, ty: &FixtureType, spec: &FixtureSpec, o: &mut FixtureOverride, ev: &mut Events) {
    let label = |b: BeamKind| match b {
        BeamKind::Spot => tr("Spot (harte Kante)", "Spot (hard edge)"),
        BeamKind::Wash => tr("Wash (weiche Kante)", "Wash (soft edge)"),
    };
    let auto = trf!("Automatisch: {}", "Automatic: {}", label(auto_beam(ty)));
    egui::ComboBox::from_label(tr("Strahl", "Beam"))
        .selected_text(o.beam.map_or(auto.clone(), |b| label(b).to_string()))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut o.beam, None, auto);
            ui.selectable_value(&mut o.beam, Some(BeamKind::Spot), label(BeamKind::Spot));
            ui.selectable_value(&mut o.beam, Some(BeamKind::Wash), label(BeamKind::Wash));
        });
    match spec.zoom_range {
        Some(range) => range_row(ui, "Zoom", ty, &Attribute::Zoom, range, &mut o.zoom_range, ev),
        None => {
            let mut angle = spec.beam_angle;
            if ui.add(egui::Slider::new(&mut angle, 1.0..=360.0).logarithmic(true).text(tr("Abstrahlwinkel °", "Beam angle °"))).changed() {
                o.beam_angle = Some(angle);
            }
            ui.colored_label(GRAY, tr("Kein Zoom-Kanal: fester Winkel. Bis 360° (Glühbirne, Lichterkette); ab 120° ohne sichtbaren Strahl im Haze.", "No zoom channel: fixed angle. Up to 360° (light bulb, fairy lights); from 120° without a visible beam in the haze."));
        }
    }
}

/// Range in degrees at DMX 0 and full, synced with BK's "Physical range".
fn range_row(ui: &mut egui::Ui, name: &str, ty: &FixtureType, attr: &Attribute, current: [f32; 2], field: &mut Option<[f32; 2]>, ev: &mut Events) {
    let mut r = current;
    ui.horizontal(|ui| {
        ui.label(format!("{name}:"));
        let a = ui.add(egui::DragValue::new(&mut r[0]).range(-720.0..=720.0).speed(0.5).suffix("°")).on_hover_text(tr("bei DMX 0", "at DMX 0"));
        ui.label(tr("bis", "to"));
        let b = ui.add(egui::DragValue::new(&mut r[1]).range(-720.0..=720.0).speed(0.5).suffix("°")).on_hover_text(tr("bei DMX voll", "at DMX full"));
        if a.changed() || b.changed() {
            *field = Some(r);
            ev.ranges.push((attr.clone(), r));
        }
    });
    let bk = bk_range(ty, attr);
    let source = match (*field, bk) {
        (Some(v), b) if b != Some(v) => tr("eigener Wert – wird in BlinderKitten als „Physical range“ eingetragen", "own value – entered in BlinderKitten as “Physical range”"),
        (_, Some(_)) => tr("aus BlinderKitten („Physical range“ des Kanals)", "from BlinderKitten (the channel's “Physical range”)"),
        _ => tr("Standardwert – in BlinderKitten nicht gesetzt; Ändern trägt ihn dort ein", "default – not set in BlinderKitten; changing it enters it there"),
    };
    ui.indent(name, |ui| ui.colored_label(GRAY, source));
}

/// Top speeds and acceleration of the pan/tilt motors.
fn motion(ui: &mut egui::Ui, ty: &FixtureType, spec: &FixtureSpec, o: &mut FixtureOverride) {
    let mut inertia = spec.pan_limits.max_speed > 0.0 || spec.tilt_limits.max_speed > 0.0;
    if ui
        .checkbox(&mut inertia, tr("Trägheit (Kopf fährt statt zu springen)", "Inertia (the head travels instead of jumping)"))
        .on_hover_text(tr("Aus: der Kopf folgt den DMX-Werten sofort.", "Off: the head follows the DMX values at once."))
        .changed()
    {
        let (pan, tilt) = if inertia { (None, None) } else { (Some(0.0), Some(0.0)) };
        o.pan_speed = pan;
        o.tilt_speed = tilt;
    }
    if !inertia {
        return;
    }
    let speed = |ui: &mut egui::Ui, label: &str, limits: Limits, field: &mut Option<f32>| {
        let mut v = limits.max_speed;
        if ui.add(egui::Slider::new(&mut v, 10.0..=1000.0).logarithmic(true).suffix(" °/s").text(label)).changed() {
            *field = Some(v);
        }
    };
    if ty.has(&Attribute::Pan) {
        speed(ui, tr("Pan max.", "Pan max."), spec.pan_limits, &mut o.pan_speed);
    }
    if ty.has(&Attribute::Tilt) {
        speed(ui, tr("Tilt max.", "Tilt max."), spec.tilt_limits, &mut o.tilt_speed);
    }
    let mut accel = spec.pan_limits.acceleration;
    if ui.add(egui::Slider::new(&mut accel, 50.0..=5000.0).logarithmic(true).suffix(" °/s²").text(tr("Beschleunigung", "Acceleration"))).changed() {
        o.pt_acceleration = Some(accel);
    }
    if spec.pan_limits.max_speed > 0.0 {
        // Time for a full-range pan move, as datasheets state it.
        let range = (spec.pan_range[1] - spec.pan_range[0]).abs();
        let t = travel_time(range, spec.pan_limits);
        ui.colored_label(GRAY, trf!("Voller Pan ({range:.0}°) dauert {t:.1} s.", "A full pan ({range:.0}°) takes {t:.1} s.", range = range, t = t));
    }
}

/// Seconds to move `distance` degrees from rest to rest.
fn travel_time(distance: f32, l: Limits) -> f32 {
    let ramp = l.max_speed / l.acceleration;
    let ramp_distance = l.max_speed * ramp;
    if distance <= ramp_distance { 2.0 * (distance / l.acceleration).sqrt() } else { 2.0 * ramp + (distance - ramp_distance) / l.max_speed }
}

#[derive(PartialEq, Clone, Copy)]
enum Arrangement {
    Auto,
    Single,
    Rect,
    Circular,
}

fn leds(ui: &mut egui::Ui, ty: &FixtureType, spec: &FixtureSpec, o: &mut FixtureOverride, painter: &mut Painter) {
    let cells = ty.cells();
    let current = match &o.leds {
        None => Arrangement::Auto,
        Some(l) if l.shape.count() == 0 => Arrangement::Single,
        Some(LedLayout { shape: LedShape::Rect { .. }, .. }) => Arrangement::Rect,
        Some(_) => Arrangement::Circular,
    };
    let mut chosen = current;
    let auto_text = match auto_leds(spec.class, &cells) {
        None => tr("Automatisch (eine Linse)", "Automatic (one lens)").to_string(),
        Some(l) => trf!("Automatisch ({} LEDs)", "Automatic ({} LEDs)", l.shape.count()),
    };
    egui::ComboBox::from_label(tr("Anordnung", "Layout"))
        .selected_text(match chosen {
            Arrangement::Auto => auto_text.as_str(),
            Arrangement::Single => tr("Eine Linse", "One lens"),
            Arrangement::Rect => tr("Rechteckig", "Rectangular"),
            Arrangement::Circular => tr("Ringe", "Rings"),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen, Arrangement::Auto, auto_text.as_str());
            ui.selectable_value(&mut chosen, Arrangement::Single, tr("Eine Linse", "One lens"));
            ui.selectable_value(&mut chosen, Arrangement::Rect, tr("Rechteckig", "Rectangular"));
            ui.selectable_value(&mut chosen, Arrangement::Circular, tr("Ringe", "Rings"));
        });
    if chosen != current {
        let pitch = spec.leds.as_ref().map_or(0.04, |l| l.pitch);
        o.leds = match chosen {
            Arrangement::Auto => None,
            Arrangement::Single => Some(LedLayout { shape: LedShape::Rect { rows: 0, cols: 0 }, pitch, led_cells: vec![] }),
            Arrangement::Rect => Some(LedLayout::new(LedShape::Rect { rows: 1, cols: cells.len().max(1) as u32 }, pitch, &cells)),
            Arrangement::Circular => Some(match &spec.leds {
                Some(l) if matches!(l.shape, LedShape::Circular { .. }) => l.clone(),
                _ => LedLayout::new(LedShape::Circular { rings: vec![1, 6] }, pitch, &cells),
            }),
        };
    }

    // Shape parameters; editing an automatic layout turns it into a custom one.
    if let Some(l) = o.leds.as_mut().filter(|l| l.shape.count() > 0) {
        let before = l.shape.clone();
        match &mut l.shape {
            LedShape::Rect { rows, cols } => {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(rows).range(1..=64).prefix(tr("Zeilen ", "Rows ")));
                    ui.add(egui::DragValue::new(cols).range(1..=64).prefix(tr("Spalten ", "Columns ")));
                });
            }
            LedShape::Circular { rings } => {
                let mut remove = None;
                for (k, n) in rings.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(if k == 0 { tr("Innen", "Inside").to_string() } else { trf!("Ring {}", "Ring {}", k + 1) });
                        ui.add(egui::DragValue::new(n).range(1..=96).suffix(" LEDs"));
                        if ui.small_button("✖").on_hover_text(tr("Ring entfernen", "Remove ring")).clicked() {
                            remove = Some(k);
                        }
                    });
                }
                if let Some(k) = remove
                    && rings.len() > 1
                {
                    rings.remove(k);
                }
                if ui.small_button(tr("+ Ring außen", "+ Outer ring")).clicked() {
                    let next = rings.last().map_or(6, |&n| if n == 1 { 6 } else { n + 6 });
                    rings.push(next);
                }
            }
        }
        if l.shape != before {
            l.led_cells = distribute(l.shape.count(), &cells);
        }
        let mut mm = l.pitch * 1000.0;
        if ui.add(egui::DragValue::new(&mut mm).range(3.0..=300.0).suffix(" mm").prefix(tr("LED-Abstand ", "LED spacing "))).changed() {
            l.pitch = mm / 1000.0;
        }
    }

    if !cells.is_empty() {
        let mut mode = spec.cell_mode;
        ui.label(trf!("{} Zellen in BlinderKitten", "{} cells in BlinderKitten", cells.len()));
        ui.radio_value(&mut mode, CellMode::Physical, tr("Jede Zelle ist eine eigene Lichtquelle", "Every cell is a light of its own"));
        ui.radio_value(&mut mode, CellMode::Mixed, tr("Ein Strahl, Zellen gemischt", "One beam, cells mixed"));
        if mode != spec.cell_mode {
            o.cell_mode = Some(mode);
        }
    }

    let Some(layout) = spec.leds.as_ref() else { return };
    ui.add_space(4.0);
    ui.label(tr("Zelle zuweisen: Zelle wählen, dann LEDs anklicken oder überstreichen.", "Assign cells: choose a cell, then click or brush over LEDs."));
    ui.horizontal_wrapped(|ui| {
        for id in std::iter::once(0).chain(cells.iter().copied()) {
            let text = if id == 0 { tr("Körper", "Body").to_string() } else { id.to_string() };
            let label = egui::RichText::new(text).color(cell_color(id, &cells));
            if ui.selectable_label(painter.brush == id, label).clicked() {
                painter.brush = id;
            }
        }
    });
    if let Some(painted) = paint(ui, layout, &cells, painter.brush) {
        let l = o.leds.get_or_insert_with(|| layout.clone());
        l.led_cells = painted;
    }
    ui.horizontal(|ui| {
        if ui.button(tr("Der Reihe nach verteilen", "Spread in order")).clicked() {
            let l = o.leds.get_or_insert_with(|| layout.clone());
            l.led_cells = distribute(l.shape.count(), &cells);
        }
        if ui.button(tr("Alle zum Körper", "All to the body")).clicked() {
            let l = o.leds.get_or_insert_with(|| layout.clone());
            l.led_cells = vec![0; l.shape.count()];
        }
    });
    let unused: Vec<String> = cells.iter().filter(|c| !layout.led_cells.contains(c)).map(u32::to_string).collect();
    if !unused.is_empty() {
        ui.colored_label(egui::Color32::LIGHT_RED, trf!("Zellen ohne LED (unsichtbar): {}", "Cells without an LED (invisible): {}", unused.join(", ")));
    }
}

fn cell_color(id: u32, cells: &[u32]) -> egui::Color32 {
    if id == 0 {
        return egui::Color32::from_gray(150);
    }
    let i = cells.iter().position(|&c| c == id).unwrap_or(0);
    // Golden-angle hues keep neighbouring cells apart.
    egui::ecolor::Hsva::new((i as f32 * 0.618_034).fract(), 0.65, 0.95, 1.0).into()
}

/// Draws the LED field seen from the front; returns new assignments while painting.
fn paint(ui: &mut egui::Ui, layout: &LedLayout, cells: &[u32], brush: u32) -> Option<Vec<u32>> {
    let positions = layout.shape.positions(layout.pitch);
    // Farthest LED centers plus one LED radius, so the outer LEDs stay inside.
    let extent = positions.iter().fold(egui::Vec2::ZERO, |m, p| m.max(egui::vec2(p[0].abs(), p[1].abs()))) + egui::Vec2::splat(layout.pitch * 0.5);
    const MAX_HEIGHT: f32 = 280.0;
    let width = (ui.available_width() - 8.0).clamp(120.0, 320.0);
    let scale = ((width / 2.0 - 6.0) / extent.x.max(1e-3)).min((MAX_HEIGHT / 2.0 - 6.0) / extent.y.max(1e-3));
    let height = (extent.y * scale * 2.0 + 12.0).max(40.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(25));
    let radius = (layout.pitch * scale * 0.45).max(2.0);
    let center = rect.center();
    let screen = |p: [f32; 2]| center + egui::vec2(p[0], -p[1]) * scale;

    let mut cells_now = layout.led_cells.clone();
    let mut changed = false;
    if let Some(pos) = response.interact_pointer_pos().filter(|_| response.is_pointer_button_down_on() || response.clicked()) {
        for (i, p) in positions.iter().enumerate() {
            if screen(*p).distance(pos) <= radius.max(5.0) && cells_now[i] != brush {
                cells_now[i] = brush;
                changed = true;
            }
        }
    }
    for (i, p) in positions.iter().enumerate() {
        let id = cells_now[i];
        painter.circle_filled(screen(*p), radius, cell_color(id, cells));
        if radius >= 7.0 {
            let text = if id == 0 { "K".to_string() } else { id.to_string() };
            painter.text(screen(*p), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(radius.min(12.0)), egui::Color32::BLACK);
        }
    }
    changed.then_some(cells_now)
}

fn color_presets() -> [(&'static str, [f32; 3]); 13] {
    [
    (tr("Weiß", "White"), [1.0, 1.0, 1.0]),
    (tr("Rot", "Red"), [1.0, 0.02, 0.02]),
    ("Orange", [1.0, 0.35, 0.02]),
    (tr("Gelb", "Yellow"), [1.0, 0.85, 0.05]),
    (tr("Grün", "Green"), [0.05, 1.0, 0.1]),
    ("Cyan", [0.05, 0.9, 1.0]),
    (tr("Blau", "Blue"), [0.03, 0.12, 1.0]),
    (tr("Lavendel", "Lavender"), [0.55, 0.45, 1.0]),
    ("Magenta", [1.0, 0.05, 0.8]),
    ("Pink", [1.0, 0.35, 0.6]),
    ("UV", [0.25, 0.0, 0.7]),
    ("CTO", [1.0, 0.68, 0.38]),
    ("CTB", [0.75, 0.85, 1.0]),
    ]
}

fn color_wheel(ui: &mut egui::Ui, o: &mut FixtureOverride) {
    let slots = o.color_wheel.get_or_insert_with(Vec::new);
    if slots.is_empty() {
        ui.colored_label(GRAY, tr("Noch keine Farben – ohne Angaben bleibt das Licht weiß.", "No colours yet – without them the light stays white."));
    }
    let mut remove = None;
    for (i, s) in slots.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut s.start).prefix(tr("ab DMX ", "from DMX ")));
            ui.color_edit_button_rgb(&mut s.color);
            ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(80.0));
            if ui.small_button("✖").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        slots.remove(i);
    }
    ui.horizontal(|ui| {
        ui.menu_button(tr("+ Farbe", "+ Colour"), |ui| {
            for (name, color) in color_presets() {
                if ui.button(name).clicked() {
                    let start = next_start(slots.iter().map(|s| s.start));
                    slots.push(ColorSlot { start, name: name.into(), color });
                    ui.close();
                }
            }
        });
        if ui.button(tr("Gleichmäßig verteilen", "Spread evenly")).on_hover_text(tr("Teilt den DMX-Bereich gleich auf alle Farben auf", "Splits the DMX range evenly among all colours")).clicked() {
            spread(slots.iter_mut().map(|s| &mut s.start));
        }
    });
    if slots.is_empty() {
        o.color_wheel = None;
    }
}

fn gobo_wheel(ui: &mut egui::Ui, o: &mut FixtureOverride, ev: &mut Events) {
    let slots = o.gobo_wheel.get_or_insert_with(Vec::new);
    if slots.is_empty() {
        ui.colored_label(GRAY, tr("Noch keine Gobos – der Strahl bleibt offen.", "No gobos yet – the beam stays open."));
    }
    let mut remove = None;
    for (i, s) in slots.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut s.start).prefix(tr("ab DMX ", "from DMX ")));
            let text = match &s.gobo {
                Gobo::Open => tr("Offen", "Open").to_string(),
                Gobo::Builtin(n) => super::gobo::label(n).to_string(),
                Gobo::Image(p) => p.rsplit('/').next().unwrap_or(p).to_string(),
            };
            egui::ComboBox::from_id_salt(("gobo", i)).selected_text(text).width(110.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.gobo, Gobo::Open, tr("Offen", "Open"));
                for name in BUILTIN {
                    ui.selectable_value(&mut s.gobo, Gobo::Builtin(name.into()), super::gobo::label(name));
                }
            });
            if ui.small_button(tr("Bild…", "Picture…")).on_hover_text(tr("Eigenes Gobo-Bild (PNG/JPG): hell = Licht, dunkel = blockiert", "Own gobo picture (PNG/JPG): bright = light, dark = blocked")).clicked() {
                ev.import_gobo = Some(i);
            }
            if ui.small_button("✖").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        slots.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button(tr("+ Gobo", "+ Gobo")).clicked() {
            let start = next_start(slots.iter().map(|s| s.start));
            let gobo = if slots.is_empty() { Gobo::Open } else { Gobo::Builtin(BUILTIN[(slots.len() - 1) % BUILTIN.len()].into()) };
            slots.push(GoboSlot { start, gobo });
        }
        if ui.button(tr("Gleichmäßig verteilen", "Spread evenly")).clicked() {
            spread(slots.iter_mut().map(|s| &mut s.start));
        }
    });
    if slots.is_empty() {
        o.gobo_wheel = None;
    }
}

/// Stands for the fixed gobo in [`Events::import_gobo`].
pub const FIXED_GOBO: usize = usize::MAX;

/// A gobo that is always in the beam, for fixtures without a gobo channel
/// (a profile with a gobo holder, a projector).
fn fixed_gobo(ui: &mut egui::Ui, o: &mut FixtureOverride, ev: &mut Events) {
    let text = match &o.fixed_gobo {
        None | Some(Gobo::Open) => tr("Keins", "None").to_string(),
        Some(Gobo::Builtin(n)) => super::gobo::label(n).to_string(),
        Some(Gobo::Image(p)) => p.rsplit('/').next().unwrap_or(p).to_string(),
    };
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("fixed_gobo").selected_text(text).width(140.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut o.fixed_gobo, None, tr("Keins", "None"));
            for name in BUILTIN {
                ui.selectable_value(&mut o.fixed_gobo, Some(Gobo::Builtin(name.into())), super::gobo::label(name));
            }
        });
        if ui.small_button(tr("Bild…", "Picture…")).on_hover_text(tr("Eigenes Gobo-Bild (PNG/JPG): hell = Licht, dunkel = blockiert", "Own gobo picture (PNG/JPG): bright = light, dark = blocked")).clicked() {
            ev.import_gobo = Some(FIXED_GOBO);
        }
    });
    ui.colored_label(GRAY, tr("Ohne Gobo-Kanal: das Gobo steckt fest im Gerät.", "No gobo channel: the gobo sits fixed in the fixture."));
}

fn next_start(starts: impl Iterator<Item = u8>) -> u8 {
    starts.max().map_or(0, |m| m.saturating_add(16))
}

fn spread<'a>(starts: impl ExactSizeIterator<Item = &'a mut u8>) {
    let n = starts.len().max(1);
    for (i, s) in starts.enumerate() {
        *s = (i * 256 / n) as u8;
    }
}
