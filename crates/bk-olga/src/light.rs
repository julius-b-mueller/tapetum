//! Light output of one emitter (a cell, or the whole fixture) computed from
//! decoded attribute values. Colors are linear RGB normalized so that the
//! brightest component is 1; brightness lives in `intensity`.

use crate::attribute::Attribute;
use crate::semantics::{FixtureClass, FixtureState};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightOutput {
    /// 0..1 relative to the fixture's full output.
    pub intensity: f32,
    pub color: [f32; 3],
    /// Strobe frequency in Hz, if strobing.
    pub strobe_hz: Option<f32>,
}

impl LightOutput {
    pub const OFF: LightOutput = LightOutput { intensity: 0.0, color: [1.0, 1.0, 1.0], strobe_hz: None };
}

// Rough LED and filter tints in linear RGB.
const WHITE_LED: [f32; 3] = [1.0, 0.97, 0.92];
const AMBER_LED: [f32; 3] = [1.0, 0.45, 0.0];
const UV_LED: [f32; 3] = [0.18, 0.0, 0.55];
/// Tungsten at 3200 K.
pub const TUNGSTEN: [f32; 3] = [1.0, 0.68, 0.38];
const CTC_TINT: [f32; 3] = [0.78, 0.88, 1.0];

/// Color temperature shift of a dimmed tungsten lamp (1800 K at low levels).
pub fn tungsten_at(level: f32) -> [f32; 3] {
    const EMBER: [f32; 3] = [1.0, 0.42, 0.12];
    let t = level.clamp(0.0, 1.0).powf(0.5);
    lerp3(EMBER, TUNGSTEN, t)
}

/// Per fixture type properties the channels don't tell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Color of the white LEDs (or of a lamp without color mixing).
    pub white: Option<[f32; 3]>,
    /// Strobe frequency at the lowest and highest strobe value.
    pub strobe_hz: [f32; 2],
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { white: None, strobe_hz: [1.0, 25.0] }
    }
}

/// Approximate linear RGB of a black body at `kelvin`, normalized to a peak of 1.
pub fn kelvin_to_rgb(kelvin: f32) -> [f32; 3] {
    // Tanner Helland's fit, in sRGB; converted to linear below.
    let t = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.699 * (t - 60.0).powf(-0.133_204_76) };
    let g = if t <= 66.0 { 99.470_8 * t.ln() - 161.119_57 } else { 288.122_16 * (t - 60.0).powf(-0.075_514_85) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    let lin = |c: f32| {
        let c = (c / 255.0).clamp(0.0, 1.0);
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let rgb = [lin(r), lin(g), lin(b)];
    let peak = rgb.iter().copied().fold(f32::EPSILON, f32::max);
    rgb.map(|c| c / peak)
}

pub fn compute(state: &FixtureState, cell: Option<u32>, class: FixtureClass) -> LightOutput {
    compute_tuned(state, cell, class, &Tuning::default())
}

pub fn compute_tuned(state: &FixtureState, cell: Option<u32>, class: FixtureClass, tuning: &Tuning) -> LightOutput {
    use Attribute::*;
    let v = |a: Attribute| state.get(cell, &a).map(|x| x.value);

    let white = tuning.white.unwrap_or(WHITE_LED);
    let additive = [(Red, [1.0, 0.0, 0.0]), (Green, [0.0, 1.0, 0.0]), (Blue, [0.0, 0.0, 1.0]), (White, white), (Amber, AMBER_LED), (Uv, UV_LED)];
    let mut has_additive = false;
    let mut rgb = [0.0f32; 3];
    for (attr, tint) in additive {
        if let Some(x) = v(attr) {
            has_additive = true;
            for i in 0..3 {
                rgb[i] += tint[i] * x;
            }
        }
    }

    let base_white = tuning.white.unwrap_or(if class == FixtureClass::Conventional { TUNGSTEN } else { WHITE_LED });
    let mut color = if has_additive {
        rgb
    } else if let (Some(h), s) = (v(Hue), v(Saturation)) {
        hsv_to_rgb(h, s.unwrap_or(1.0))
    } else {
        base_white
    };

    // Subtractive CMY filters on top.
    for (attr, idx) in [(Cyan, 0), (Magenta, 1), (Yellow, 2)] {
        if let Some(x) = v(attr) {
            color[idx] *= 1.0 - x;
        }
    }
    if let Some(x) = v(Cto) {
        color = mul3(color, lerp3([1.0; 3], TUNGSTEN, x));
    }
    if let Some(x) = v(Ctc) {
        color = mul3(color, lerp3([1.0; 3], CTC_TINT, x));
    }

    let peak = color.iter().copied().fold(0.0f32, f32::max);
    if peak <= f32::EPSILON {
        return LightOutput::OFF;
    }
    let color = color.map(|c| c / peak);
    // Without a dimmer the color channels carry the brightness.
    let level = if has_additive { peak.min(1.0) } else { 1.0 };
    let dimmer = v(Intensity).unwrap_or(1.0);
    // A cell's own dimmer and the body's master dimmer both apply.
    let master = match cell {
        Some(c) if state.cells.get(&c).is_some_and(|m| m.contains_key(&Intensity)) => {
            state.body.get(&Intensity).map_or(1.0, |x| x.value)
        }
        _ => 1.0,
    };
    // At its default value a strobe channel is idle; that is 0 for most
    // fixtures but e.g. 1.0 ("open") for the water effect light of the test show.
    let strobe_hz = state
        .get(cell, &Strobe)
        .filter(|s| (s.value - s.default).abs() > 0.02)
        .map(|s| tuning.strobe_hz[0] + s.value * (tuning.strobe_hz[1] - tuning.strobe_hz[0]));

    LightOutput { intensity: (level * dimmer * master).clamp(0.0, 1.0), color, strobe_hz }
}

/// Intensity-weighted mix of several emitters into one beam.
pub fn mix(outputs: &[LightOutput]) -> LightOutput {
    mix_weighted(&outputs.iter().map(|&o| (o, 1.0)).collect::<Vec<_>>())
}

/// Mix of emitters whose share of the light differs, e.g. cells with more LEDs.
/// Brightness is the weighted mean; color is weighted by weight times intensity.
pub fn mix_weighted(outputs: &[(LightOutput, f32)]) -> LightOutput {
    let weights: f32 = outputs.iter().map(|(_, w)| w).sum();
    let total: f32 = outputs.iter().map(|(o, w)| o.intensity * w).sum();
    if weights <= f32::EPSILON || total <= f32::EPSILON {
        return LightOutput::OFF;
    }
    let mut color = [0.0f32; 3];
    for (o, w) in outputs {
        for (c, oc) in color.iter_mut().zip(o.color) {
            *c += oc * o.intensity * w / total;
        }
    }
    let peak = color.iter().copied().fold(0.0f32, f32::max).max(f32::EPSILON);
    LightOutput {
        intensity: total / weights,
        color: color.map(|c| c / peak),
        strobe_hz: outputs.iter().find_map(|(o, _)| o.strobe_hz),
    }
}

fn hsv_to_rgb(h: f32, s: f32) -> [f32; 3] {
    let h = (h.fract() + 1.0).fract() * 6.0;
    let f = h.fract();
    let (p, q, t) = (1.0 - s, 1.0 - s * f, 1.0 - s * (1.0 - f));
    match h as u32 {
        0 => [1.0, t, p],
        1 => [q, 1.0, p],
        2 => [p, 1.0, t],
        3 => [p, q, 1.0],
        4 => [t, p, 1.0],
        _ => [1.0, p, q],
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_mix_follows_led_count() {
        let red = LightOutput { intensity: 1.0, color: [1.0, 0.0, 0.0], strobe_hz: None };
        let blue = LightOutput { intensity: 1.0, color: [0.0, 0.0, 1.0], strobe_hz: None };
        // One red LED, three blue ones.
        let m = mix_weighted(&[(red, 1.0), (blue, 3.0)]);
        assert!((m.color[0] - 1.0 / 3.0).abs() < 1e-5 && m.color[2] == 1.0, "{:?}", m.color);
        assert!((m.intensity - 1.0).abs() < 1e-6);
        // Only the single red LED on: a quarter of the light.
        let off = LightOutput { intensity: 0.0, ..blue };
        assert!((mix_weighted(&[(red, 1.0), (off, 3.0)]).intensity - 0.25).abs() < 1e-6);
    }

    #[test]
    fn kelvin() {
        let warm = kelvin_to_rgb(3200.0);
        let cold = kelvin_to_rgb(6500.0);
        assert_eq!(warm[0], 1.0);
        assert!(warm[2] < 0.4 && cold[2] > 0.8, "{warm:?} {cold:?}");
    }
}
