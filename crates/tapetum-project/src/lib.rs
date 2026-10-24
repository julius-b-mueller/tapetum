//! Tapetum's own project file, stored next to the show as `<show>.tapetum`.
//! Everything BlinderKitten doesn't know about lives here. Files from before
//! the rename (`<show>.bkviz`) are read too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const EXTENSION: &str = "tapetum";
/// The extension before Tapetum got its name.
pub const LEGACY_EXTENSION: &str = "bkviz";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot access {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} is not a valid project file: {1}")]
    Json(PathBuf, serde_json::Error),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub render: RenderSettings,
    pub artnet: ArtNetSettings,
    /// Keyed by fixture type short name.
    pub fixture_types: BTreeMap<String, FixtureOverride>,
    /// Fixture types whose settings came from the fixture library, by type short name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub library_links: BTreeMap<String, LibraryLink>,
    /// When a fixture type's settings last changed (Unix seconds), by type short name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fixture_types_changed: BTreeMap<String, u64>,
    /// Keyed by fixture ID; wins over `fixture_types`.
    pub fixtures: BTreeMap<i64, FixtureOverride>,
    /// Placements edited while BlinderKitten was not connected, keyed by fixture ID.
    pub pending_placements: BTreeMap<i64, Placement>,
    pub objects: Vec<StageObject>,
    /// Scenes: object positions on top of the base scene (the files as they are).
    pub scenes: Vec<Scene>,
    pub cameras: Vec<CameraBookmark>,
    pub spacemouse: SpaceMouseSettings,
    pub view: ViewSettings,
}

/// The standard views the camera swings to (for plans that line up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StandardView {
    /// From the audience, slightly from above.
    #[default]
    Audience,
    /// Straight down, the audience at the bottom.
    Top,
    /// Straight from the front.
    Front,
    /// From the left and the right (as seen from the audience).
    Left,
    Right,
}

impl StandardView {
    pub const ALL: [StandardView; 5] = [StandardView::Audience, StandardView::Top, StandardView::Front, StandardView::Left, StandardView::Right];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    /// The standard view chosen last (until the camera is moved by hand).
    pub standard: Option<StandardView>,
    /// Orthographic projection: true to scale, for plans.
    pub orthographic: bool,
    /// Set-change drawings leave hidden lines out instead of dashing them.
    pub omit_hidden: bool,
}

/// 3Dconnexion SpaceMouse driving the camera.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpaceMouseSettings {
    pub enabled: bool,
    /// Speed factor, 1 = default.
    pub speed: f32,
    /// The cap moves and turns the stage (true) or the camera (false).
    pub move_stage: bool,
    /// Keeps the device while another program (BlinderKitten) is in front.
    pub keep_in_background: bool,
}

impl Default for SpaceMouseSettings {
    fn default() -> Self {
        SpaceMouseSettings { enabled: true, speed: 1.0, move_stage: true, keep_in_background: false }
    }
}

/// Ordered from the lightest to the finest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Quality {
    /// For very weak computers and the battery: beams as plain cones, no lights.
    Draft,
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    pub quality: Quality,
    pub haze: Haze,
    /// Work light level 0..1.
    pub ambient: f32,
    /// Colour of the house light (linear RGB); `None` is warm white (3000 K).
    pub house_color: Option<[f32; 3]>,
    pub bloom: bool,
    /// Camera exposure (EV100); lower is brighter.
    pub exposure_ev: f32,
    /// 3D resolution relative to the screen's physical pixels; `None` renders
    /// at the logical resolution (half on Retina displays). The UI stays sharp.
    pub resolution: Option<f32>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings { quality: Quality::default(), haze: Haze::default(), ambient: 0.05, house_color: None, bloom: true, exposure_ev: 6.0, resolution: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Haze {
    pub enabled: bool,
    pub density: f32,
    /// Forward scattering, -1..1 (Henyey-Greenstein g).
    pub anisotropy: f32,
    pub color: [f32; 3],
    /// Size of the hazed volume in meters (width, height, depth), centered above the origin.
    pub extent: [f32; 3],
}

impl Default for Haze {
    fn default() -> Self {
        Haze { enabled: true, density: 0.08, anisotropy: 0.6, color: [1.0, 1.0, 1.0], extent: [30.0, 12.0, 30.0] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtNetSettings {
    pub listen_port: u16,
    /// Port-address received by the visualizer -> short name of the BK interface it mirrors.
    /// Without an entry, a universe maps to the Art-Net interface with the same port-address.
    pub routes: BTreeMap<u16, String>,
    /// UDP port of BlinderKitten's OSC Remote Control (Preferences).
    pub bk_remote_port: u16,
}

impl Default for ArtNetSettings {
    fn default() -> Self {
        ArtNetSettings { listen_port: 6454, routes: BTreeMap::new(), bk_remote_port: 42000 }
    }
}

/// Where a fixture type's settings came from in the fixture library.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryLink {
    /// The library file.
    pub file: String,
    /// Fingerprint of the settings as they were taken over: while the project
    /// still has exactly these, they weren't changed here.
    pub applied: String,
    /// Fingerprint of the library file's settings then: if the file now has
    /// others, it was updated since.
    pub source: String,
    /// The library file's time stamp last taken over or declined: newer ones are offered.
    pub seen: u64,
}

/// Now in Unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Settings of the generic fixture model; `None` fields keep the automatic choice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FixtureOverride {
    pub class: Option<String>,
    pub beam: Option<BeamKind>,
    pub cell_mode: Option<CellMode>,
    /// Arrangement of the LEDs on the front and the cell each one belongs to.
    pub leds: Option<LedLayout>,
    /// Path to a `.gdtf`, relative to the project file.
    pub gdtf: Option<String>,
    /// GDTF beam geometry name per BK cell id, if not in order.
    pub gdtf_beam_map: BTreeMap<u32, String>,
    /// Full beam angle in degrees for fixtures without a zoom channel.
    pub beam_angle: Option<f32>,
    /// Degrees at DMX 0 and full; also written to BK's "Physical range".
    pub zoom_range: Option<[f32; 2]>,
    pub pan_range: Option<[f32; 2]>,
    pub tilt_range: Option<[f32; 2]>,
    /// Top speed in degrees per second; 0 = follow DMX without inertia.
    pub pan_speed: Option<f32>,
    pub tilt_speed: Option<f32>,
    /// Pan/tilt acceleration in degrees per second squared.
    pub pt_acceleration: Option<f32>,
    /// Top speed of the zoom, beam angle in degrees per second; 0 = follow DMX at once.
    pub zoom_speed: Option<f32>,
    pub lumens: Option<f32>,
    /// Color temperature of the white LEDs / lamp in Kelvin.
    pub white_kelvin: Option<f32>,
    /// Strobe frequency at the lowest and highest strobe value.
    pub strobe_hz: Option<[f32; 2]>,
    pub color_wheel: Option<Vec<ColorSlot>>,
    pub gobo_wheel: Option<Vec<GoboSlot>>,
    /// A gobo always in the beam, for fixtures without a gobo channel.
    pub fixed_gobo: Option<Gobo>,
    /// Scale of the generic body (1 = default size).
    pub body_scale: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeamKind {
    /// Hard-edged beam (profile, spot, beam moving head).
    Spot,
    /// Soft-edged beam (PAR, wash, fresnel).
    Wash,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellMode {
    /// Every cell is its own light source.
    Physical,
    /// One beam per fixture whose color and intensity mix all cells.
    #[default]
    Mixed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedLayout {
    pub shape: LedShape,
    /// Distance between neighbouring LEDs in meters.
    pub pitch: f32,
    /// Cell (BK subfixture ID) of every LED; 0 = the fixture body.
    pub led_cells: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LedShape {
    /// Rows x columns, LEDs numbered row by row from the top left.
    Rect { rows: u32, cols: u32 },
    /// Concentric rings from the inside out with their LED counts; a ring of
    /// one LED is the center.
    Circular { rings: Vec<u32> },
}

impl LedShape {
    pub fn count(&self) -> usize {
        match self {
            LedShape::Rect { rows, cols } => (*rows * *cols) as usize,
            LedShape::Circular { rings } => rings.iter().map(|&n| n as usize).sum(),
        }
    }

    /// LED positions on the front (meters, y up), in LED order.
    pub fn positions(&self, pitch: f32) -> Vec<[f32; 2]> {
        match self {
            LedShape::Rect { rows, cols } => {
                let (rows, cols) = (*rows as usize, *cols as usize);
                (0..rows * cols)
                    .map(|i| {
                        let (r, c) = (i / cols, i % cols);
                        [(c as f32 - (cols as f32 - 1.0) / 2.0) * pitch, ((rows as f32 - 1.0) / 2.0 - r as f32) * pitch]
                    })
                    .collect()
            }
            LedShape::Circular { rings } => {
                let centered = rings.first() == Some(&1);
                let mut out = Vec::new();
                for (k, &n) in rings.iter().enumerate() {
                    let radius = if centered { k as f32 } else { k as f32 + 0.5 } * pitch;
                    for i in 0..n {
                        // First LED at the top, clockwise as seen from the front.
                        let a = std::f32::consts::FRAC_PI_2 - i as f32 * std::f32::consts::TAU / n as f32;
                        out.push(if n == 1 && radius == 0.0 { [0.0, 0.0] } else { [a.cos() * radius, a.sin() * radius] });
                    }
                }
                out
            }
        }
    }
}

impl LedLayout {
    /// A layout with `cells` distributed over the LEDs in order (blocks of equal size).
    pub fn new(shape: LedShape, pitch: f32, cells: &[u32]) -> Self {
        let led_cells = distribute(shape.count(), cells);
        LedLayout { shape, pitch, led_cells }
    }

    /// Keeps the assignment valid after the shape changed.
    pub fn fit(&mut self, cells: &[u32]) {
        if self.led_cells.len() != self.shape.count() {
            self.led_cells = distribute(self.shape.count(), cells);
        }
    }
}

/// LED i gets cell `cells[i * n / leds]`; all body if there are no cells.
pub fn distribute(leds: usize, cells: &[u32]) -> Vec<u32> {
    if cells.is_empty() {
        return vec![0; leds];
    }
    (0..leds).map(|i| cells[i * cells.len() / leds.max(1)]).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorSlot {
    /// First DMX value (0-255) of this slot.
    pub start: u8,
    pub name: String,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoboSlot {
    pub start: u8,
    pub gobo: Gobo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Gobo {
    Open,
    /// One of the built-in patterns, by name.
    Builtin(String),
    /// Image file, relative to the project file; dark = blocked.
    Image(String),
}

/// The slot active at a (0..1) wheel value.
pub fn wheel_slot<T>(slots: &[T], start: impl Fn(&T) -> u8, value: f32) -> Option<&T> {
    let dmx = (value * 255.0).round() as u8;
    slots.iter().filter(|s| start(s) <= dmx).max_by_key(|s| start(s))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StageObject {
    pub id: u64,
    pub name: String,
    /// glTF/GLB, OBJ, STL or IngeTrazo `.igz`; relative to the project file
    /// when it lies below it. Changes to the file are picked up live.
    pub model: String,
    /// Meters per model unit.
    pub unit_scale: f32,
    pub up: UpAxis,
    /// Placement of the whole file (m, degrees in BK's convention, scale).
    /// IngeTrazo files keep theirs: they are placed in IngeTrazo.
    pub transform: Transform,
    pub visible: bool,
    /// Overrides for the objects inside the file (IngeTrazo groups and
    /// components, glTF nodes, OBJ objects), by object key. The file itself
    /// is never changed.
    pub parts: BTreeMap<String, PartOverride>,
}

/// Settings that replace what a model file says about one of its objects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PartOverride {
    /// sRGB color instead of the file's colors and textures.
    pub color: Option<[f32; 3]>,
    /// Share of light the surface reflects (albedo), 0..1.
    pub reflectance: Option<f32>,
    pub surface: Option<Surface>,
    pub visible: Option<bool>,
    /// Locked: not picked in the 3D view (an auditorium that stays put).
    pub locked: Option<bool>,
}

impl Default for StageObject {
    fn default() -> Self {
        StageObject {
            id: 0,
            name: String::new(),
            model: String::new(),
            unit_scale: 1.0,
            up: UpAxis::Z,
            transform: Transform::default(),
            visible: true,
            parts: BTreeMap::new(),
        }
    }
}

/// Which model axis points up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpAxis {
    /// CAD, IngeTrazo, BlinderKitten.
    #[default]
    Z,
    /// glTF and most OBJ exports.
    Y,
}

/// How light gets through an object's surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Surface {
    #[default]
    Opaque,
    /// Gauze, opera foil, frosted panels.
    Translucent {
        /// Share of the light passing through, 0..1.
        transmission: f32,
        /// 0 = clear (see-through), 1 = fully diffusing (glows, nothing visible behind).
        diffusion: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Euler angles in degrees (XYZ).
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Transform { translation: [0.0; 3], rotation: [0.0; 3], scale: [1.0; 3] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    /// Active from this cue of the main conductor cuelist on, until the next
    /// scene's cue.
    pub cue_id: Option<f64>,
    /// Seconds the objects take to move into this scene.
    pub transition: f32,
    /// Objects that differ from the base scene, by object key.
    pub objects: BTreeMap<String, SceneObjectState>,
}

impl Default for Scene {
    fn default() -> Self {
        Scene { id: 0, name: String::new(), cue_id: None, transition: 3.0, objects: BTreeMap::new() }
    }
}

/// An object's place in a scene, relative to where the base scene has it:
/// moved by `offset` and turned by `rotation` around its foot point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneObjectState {
    pub offset: [f32; 3],
    /// Degrees, BK's convention.
    pub rotation: [f32; 3],
    pub visible: bool,
}

impl Default for SceneObjectState {
    fn default() -> Self {
        SceneObjectState { offset: [0.0; 3], rotation: [0.0; 3], visible: true }
    }
}

impl Project {
    /// The scene a cue of the main cuelist shows: the one with the highest
    /// cue ID not above it (`None` = base scene).
    pub fn scene_for_cue(&self, cue: f64) -> Option<u64> {
        self.scenes
            .iter()
            .filter_map(|s| s.cue_id.filter(|&c| c <= cue + 1e-9).map(|c| (c, s.id)))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraBookmark {
    pub name: String,
    pub position: [f32; 3],
    pub target: [f32; 3],
}

impl Project {
    pub fn new() -> Self {
        Project { version: FORMAT_VERSION, ..Default::default() }
    }

    /// `show.olga` -> `show.tapetum`.
    pub fn path_for_show(show: &Path) -> PathBuf {
        show.with_extension(EXTENSION)
    }

    /// Loads the project, or returns a fresh one if the file doesn't exist yet.
    /// Without one, a project from before the rename is taken over (it is
    /// saved under the new name the next time something changes).
    pub fn load_or_default(path: &Path) -> Result<Self, Error> {
        let legacy = path.with_extension(LEGACY_EXTENSION);
        if !path.exists() && legacy.exists() && path.extension().is_some_and(|e| e == EXTENSION) {
            return Self::load_or_default(&legacy);
        }
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| Error::Json(path.into(), e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Project::new()),
            Err(e) => Err(Error::Io(path.into(), e)),
        }
    }

    /// Writes atomically so a crash never leaves a half-written project.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let json = serde_json::to_vec_pretty(self).expect("project serializes");
        let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
        std::fs::write(&tmp, json).map_err(|e| Error::Io(tmp.clone(), e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::Io(path.into(), e))
    }

    /// Per-fixture settings merged over the fixture type's.
    pub fn fixture_override(&self, fixture_id: i64, type_short_name: &str) -> FixtureOverride {
        let base = self.fixture_types.get(type_short_name).cloned().unwrap_or_default();
        let Some(o) = self.fixtures.get(&fixture_id).cloned() else { return base };
        FixtureOverride {
            class: o.class.or(base.class),
            beam: o.beam.or(base.beam),
            cell_mode: o.cell_mode.or(base.cell_mode),
            leds: o.leds.or(base.leds),
            gdtf: o.gdtf.or(base.gdtf),
            gdtf_beam_map: if o.gdtf_beam_map.is_empty() { base.gdtf_beam_map } else { o.gdtf_beam_map },
            beam_angle: o.beam_angle.or(base.beam_angle),
            zoom_range: o.zoom_range.or(base.zoom_range),
            pan_range: o.pan_range.or(base.pan_range),
            tilt_range: o.tilt_range.or(base.tilt_range),
            pan_speed: o.pan_speed.or(base.pan_speed),
            tilt_speed: o.tilt_speed.or(base.tilt_speed),
            pt_acceleration: o.pt_acceleration.or(base.pt_acceleration),
            zoom_speed: o.zoom_speed.or(base.zoom_speed),
            lumens: o.lumens.or(base.lumens),
            white_kelvin: o.white_kelvin.or(base.white_kelvin),
            strobe_hz: o.strobe_hz.or(base.strobe_hz),
            color_wheel: o.color_wheel.or(base.color_wheel),
            gobo_wheel: o.gobo_wheel.or(base.gobo_wheel),
            fixed_gobo: o.fixed_gobo.or(base.fixed_gobo),
            body_scale: o.body_scale.or(base.body_scale),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_follows_the_last_cue_reached() {
        let mut p = Project::default();
        for (id, cue) in [(1, Some(10.0)), (2, Some(20.0)), (3, None)] {
            p.scenes.push(Scene { id, cue_id: cue, ..Default::default() });
        }
        assert_eq!(p.scene_for_cue(5.0), None);
        assert_eq!(p.scene_for_cue(10.0), Some(1));
        assert_eq!(p.scene_for_cue(19.5), Some(1));
        assert_eq!(p.scene_for_cue(27.0), Some(2));
    }

    #[test]
    fn led_layouts() {
        let ring = LedShape::Circular { rings: vec![1, 6] };
        assert_eq!(ring.count(), 7);
        let p = ring.positions(0.05);
        assert_eq!(p[0], [0.0, 0.0]);
        assert!((p[1][1] - 0.05).abs() < 1e-6, "first ring LED at the top: {:?}", p[1]);
        let rect = LedShape::Rect { rows: 2, cols: 3 };
        assert_eq!(rect.positions(1.0)[0], [-1.0, 0.5]);
        // 12 LEDs over 6 cells: two LEDs per cell, in order.
        assert_eq!(distribute(12, &[1, 2, 3, 4, 5, 6]), [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6]);
        assert_eq!(distribute(3, &[]), [0, 0, 0]);
    }

    #[test]
    fn wheel_slots() {
        let slots = [(0u8, "open"), (32, "red"), (64, "blue")];
        let pick = |v| wheel_slot(&slots, |s| s.0, v).map(|s| s.1);
        assert_eq!(pick(0.0), Some("open"));
        assert_eq!(pick(40.0 / 255.0), Some("red"));
        assert_eq!(pick(1.0), Some("blue"));
    }

    #[test]
    fn roundtrip_and_tolerates_missing_fields() {
        let dir = std::env::temp_dir().join(format!("tapetum-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.tapetum");

        let mut p = Project::new();
        p.fixtures.insert(11, FixtureOverride { cell_mode: Some(CellMode::Mixed), ..Default::default() });
        p.scenes.push(Scene { id: 1, name: "Strand".into(), cue_id: Some(12.0), ..Default::default() });
        p.save(&path).unwrap();
        assert_eq!(Project::load_or_default(&path).unwrap(), p);

        std::fs::write(&path, r#"{"version": 1, "render": {"ambient": 0.2}}"#).unwrap();
        let partial = Project::load_or_default(&path).unwrap();
        assert_eq!(partial.render.ambient, 0.2);
        assert_eq!(partial.render.haze, Haze::default());
        assert_eq!(partial.artnet.listen_port, 6454);

        assert_eq!(Project::load_or_default(&dir.join("missing.tapetum")).unwrap(), Project::new());

        // A project from before the rename is taken over.
        std::fs::write(dir.join("old.bkviz"), r#"{"version": 1, "render": {"ambient": 0.7}}"#).unwrap();
        assert_eq!(Project::load_or_default(&dir.join("old.tapetum")).unwrap().render.ambient, 0.7);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fixture_settings_override_type_settings() {
        let mut p = Project::new();
        p.fixture_types.insert("movingHeadWash".into(), FixtureOverride { cell_mode: Some(CellMode::Mixed), beam_angle: Some(20.0), ..Default::default() });
        p.fixtures.insert(12, FixtureOverride { cell_mode: Some(CellMode::Physical), ..Default::default() });
        let o = p.fixture_override(12, "movingHeadWash");
        assert_eq!((o.cell_mode, o.beam_angle), (Some(CellMode::Physical), Some(20.0)));
        assert_eq!(p.fixture_override(11, "movingHeadWash").cell_mode, Some(CellMode::Mixed));
    }
}
