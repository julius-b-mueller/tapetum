//! Turns a fixture type into something renderable: a generic fixture class,
//! the list of cells, and DMX decoding into normalized attribute values.

use std::collections::BTreeMap;

use crate::attribute::Attribute;
use crate::model::FixtureType;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixtureClass {
    MovingHead,
    /// Profile/fresnel style fixture with optics channels (zoom, focus, iris, ...).
    Spot,
    /// Color fixture without optics channels.
    Wash,
    /// Several cells side by side without pan/tilt.
    Bar,
    /// Intensity only: a conventional tungsten fixture or a generic dimmer.
    Conventional,
    /// No channel with a visual effect.
    Unknown,
}

impl FixtureClass {
    pub fn label(self) -> &'static str {
        match self {
            FixtureClass::MovingHead => "Moving Head",
            FixtureClass::Spot => "Spot",
            FixtureClass::Wash => "Wash",
            FixtureClass::Bar => "Bar",
            FixtureClass::Conventional => "Conventional",
            FixtureClass::Unknown => "Unknown",
        }
    }
}

impl FixtureType {
    /// Sorted subfixture ids >= 1 that carry at least one visual channel.
    pub fn cells(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .channels
            .iter()
            .filter(|c| c.sub_fixture > 0 && c.attribute.is_visual())
            .map(|c| c.sub_fixture)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn has(&self, attr: &Attribute) -> bool {
        self.channels.iter().any(|c| &c.attribute == attr)
    }

    pub fn classify(&self) -> FixtureClass {
        let visual: Vec<&Attribute> = self.channels.iter().map(|c| &c.attribute).filter(|a| a.is_visual()).collect();
        if visual.is_empty() {
            return FixtureClass::Unknown;
        }
        if self.has(&Attribute::Pan) && self.has(&Attribute::Tilt) {
            return FixtureClass::MovingHead;
        }
        if self.cells().len() >= 2 {
            return FixtureClass::Bar;
        }
        if visual.iter().any(|a| a.is_optics()) {
            return FixtureClass::Spot;
        }
        if visual.iter().any(|a| a.is_color()) {
            return FixtureClass::Wash;
        }
        FixtureClass::Conventional
    }

    /// Number of DMX channels the fixture occupies.
    pub fn footprint(&self) -> u16 {
        self.channels
            .iter()
            .map(|c| c.fine_offset.unwrap_or(c.offset).max(c.offset))
            .max()
            .unwrap_or(0)
    }

    /// Decodes the fixture's channels from a 512-byte universe, starting at the
    /// 1-based patch `address`.
    pub fn decode(&self, address: u16, universe: &[u8]) -> FixtureState {
        let mut state = FixtureState::default();
        if address == 0 {
            return state;
        }
        let byte = |offset: u16| -> Option<u8> {
            let idx = usize::from(address) + usize::from(offset) - 2;
            universe.get(idx).copied()
        };
        for ch in &self.channels {
            if !ch.attribute.is_visual() || ch.resolution == crate::model::Resolution::FineOnly {
                continue;
            }
            let Some(msb) = byte(ch.offset) else { continue };
            let mut v = match ch.fine_offset.and_then(byte) {
                Some(lsb) => f32::from(u16::from(msb) << 8 | u16::from(lsb)) / 65535.0,
                None => f32::from(msb) / 255.0,
            };
            if ch.invert {
                v = 1.0 - v;
            }
            let target = if ch.sub_fixture == 0 { &mut state.body } else { state.cells.entry(ch.sub_fixture).or_default() };
            target.insert(
                ch.attribute.clone(),
                AttrValue { value: v, default: ch.default_value, physical_range: ch.physical_range },
            );
        }
        state
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttrValue {
    /// Normalized 0..1 (after BK's invert flag has been undone).
    pub value: f32,
    /// The channel's default value in BK (its "idle" position).
    pub default: f32,
    pub physical_range: Option<[f32; 2]>,
}

impl AttrValue {
    /// Value mapped into its physical range, or `fallback` range if none is set.
    pub fn physical(&self, fallback: [f32; 2]) -> f32 {
        let [a, b] = self.physical_range.unwrap_or(fallback);
        a + (b - a) * self.value
    }
}

pub type AttrMap = BTreeMap<Attribute, AttrValue>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FixtureState {
    pub body: AttrMap,
    pub cells: BTreeMap<u32, AttrMap>,
}

impl FixtureState {
    /// Attribute of a cell, falling back to the fixture body.
    pub fn get(&self, cell: Option<u32>, attr: &Attribute) -> Option<AttrValue> {
        cell.and_then(|c| self.cells.get(&c))
            .and_then(|m| m.get(attr))
            .or_else(|| self.body.get(attr))
            .copied()
    }
}
