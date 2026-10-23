//! Domain model of a BlinderKitten show, as far as the visualizer needs it.
//!
//! Defaults mirror the `add*Parameter(...)` calls in the BlinderKitten
//! sources, because `.olga` files only store values that differ from them.

use serde_json::Value;

use crate::attribute::Attribute;
use crate::organic::Node;
use crate::short_name;

#[derive(Clone, Debug, Default)]
pub struct Show {
    pub version: Option<String>,
    pub interfaces: Vec<Interface>,
    pub fixture_types: Vec<FixtureType>,
    pub fixtures: Vec<Fixture>,
    pub cuelists: Vec<Cuelist>,
    /// Fixture groups, sorted by ID.
    pub groups: Vec<Group>,
    /// Cuelist ID of the main conductor (project settings).
    pub conductor_id: i64,
    /// Non-fatal problems found while reading (unresolved references, ...).
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Interface {
    pub name: String,
    pub short_name: String,
    pub kind: InterfaceKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterfaceKind {
    ArtNet(ArtNetOutput),
    Sacn(SacnOutput),
    /// USB/serial DMX devices.
    OtherDmx(String),
    /// OSC, MIDI, ... interfaces carry no DMX.
    NonDmx(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArtNetOutput {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub net: u8,
    pub subnet: u8,
    pub universe: u8,
    /// BK binds UDP 6454 for this interface (Art-Net input or "Force src port"),
    /// which blocks a visualizer listening on 6454 on the same machine.
    pub binds_6454: bool,
}

impl ArtNetOutput {
    /// 15-bit Art-Net port-address, as written to bytes 14/15 of ArtDmx.
    pub fn port_address(&self) -> u16 {
        (u16::from(self.net & 0x7f) << 8) | (u16::from(self.subnet & 0x0f) << 4) | u16::from(self.universe & 0x0f)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SacnOutput {
    pub universe: u16,
    pub multicast: bool,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct FixtureType {
    pub name: String,
    pub short_name: String,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Bits8,
    Bits16,
    /// LSB of another channel of the same type and subfixture.
    FineOnly,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    /// Short name of the channel item, for its BK control address.
    pub short_name: String,
    /// Channel type reference path, e.g. `/color/channelTypes/red`. Empty for unused channels.
    pub channel_type: String,
    pub attribute: Attribute,
    pub resolution: Resolution,
    /// 1-based offset inside the fixture footprint.
    pub offset: u16,
    /// Offset of the LSB, if this channel is 16 bit or has a separate fine channel.
    pub fine_offset: Option<u16>,
    /// 0 = fixture body, >= 1 = cell (subfixture).
    pub sub_fixture: u32,
    pub default_value: f32,
    pub invert: bool,
    /// Physical output range (degrees for pan/tilt/zoom), if set.
    pub physical_range: Option<[f32; 2]>,
}

#[derive(Clone, Debug)]
pub struct Fixture {
    pub id: i64,
    pub name: String,
    /// Index into `Show::fixture_types`.
    pub fixture_type: Option<usize>,
    /// Meters.
    pub position: [f32; 3],
    /// Degrees.
    pub rotation: [f32; 3],
    /// False if position and rotation were never set in BK.
    pub has_placement: bool,
    pub patches: Vec<Patch>,
    /// Control address of the fixture item, e.g. `/fixture/11_MH1` (used by remote control).
    pub short_name: String,
}

#[derive(Clone, Debug)]
pub struct Patch {
    pub enabled: bool,
    /// Index into `Show::interfaces`.
    pub interface: Option<usize>,
    /// 1-512, 0 = unpatched.
    pub address: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    /// IDs of the fixtures in the group, in selection order.
    pub fixtures: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct Cuelist {
    pub id: i64,
    pub name: String,
    /// Control address of the cuelist item, e.g. `1_Main` (`/cuelist/1_Main`).
    pub short_name: String,
    pub cues: Vec<Cue>,
}

#[derive(Clone, Debug)]
pub struct Cue {
    pub id: f64,
    pub name: String,
    pub chapter: String,
}

impl Show {
    pub fn from_json(root: &Value) -> Self {
        let root = Node(root);
        let mut show = Show {
            version: root.0.pointer("/metaData/version").and_then(Value::as_str).map(str::to_owned),
            conductor_id: root
                .container_path(&["projectSettings", "conductorInfosSettings"])
                .and_then(|c| c.i64_param("conductorID"))
                .unwrap_or(1),
            ..Default::default()
        };

        if let Some(m) = root.0.get("interfaces") {
            show.interfaces = Node(m).items().map(parse_interface).collect();
        }
        if let Some(m) = root.0.get("fixtureType") {
            show.fixture_types = Node(m).items().map(parse_fixture_type).collect();
        }
        if let Some(m) = root.0.get("fixture") {
            let mut warnings = Vec::new();
            show.fixtures = Node(m).items().map(|n| parse_fixture(n, &show, &mut warnings)).collect();
            show.warnings.extend(warnings);
        }
        if let Some(m) = root.0.get("cuelist") {
            show.cuelists = Node(m).items().map(parse_cuelist).collect();
        }
        if let Some(m) = root.0.get("group") {
            show.groups = parse_groups(Node(m), &show.fixtures);
        }
        show
    }

    pub fn fixture_type(&self, f: &Fixture) -> Option<&FixtureType> {
        f.fixture_type.map(|i| &self.fixture_types[i])
    }

    pub fn interface(&self, p: &Patch) -> Option<&Interface> {
        p.interface.map(|i| &self.interfaces[i])
    }

    /// BK control address of a fixture type channel's parameter, e.g.
    /// `/fixtureType/movingHeadWash/channels/channel1_2_Pan/physicalRange`.
    pub fn channel_address(ty: &FixtureType, ch: &Channel, param: &str) -> String {
        format!("/fixtureType/{}/channels/{}/{param}", ty.short_name, ch.short_name)
    }

    pub fn conductor(&self) -> Option<&Cuelist> {
        self.cuelists.iter().find(|c| c.id == self.conductor_id)
    }
}

fn nice_name_or(n: Node, fallback: &str) -> String {
    n.nice_name().unwrap_or(fallback).to_string()
}

fn find_by_reference<T>(items: &[T], reference: &str, short: impl Fn(&T) -> &str) -> Option<usize> {
    let target = reference.strip_prefix('/').unwrap_or(reference);
    if target.is_empty() {
        return None;
    }
    items
        .iter()
        .position(|t| short(t) == target)
        .or_else(|| items.iter().position(|t| short(t).eq_ignore_ascii_case(target)))
}

fn parse_interface(n: Node) -> Interface {
    let name = nice_name_or(n, "Interface");
    let short_name = short_name::to_short_name(&name, false);
    let kind = match n.item_type() {
        Some("DMX") => {
            // EnumParameter defaults to its first option, "Open DMX".
            let dmx_type = n.str_param("dmxType").unwrap_or("Open DMX");
            match dmx_type {
                "Art-Net" => {
                    let out = n.container_path(&["artNet", "output"]);
                    let p = |name: &str| out.and_then(|o| o.i64_param(name));
                    InterfaceKind::ArtNet(ArtNetOutput {
                        enabled: out.and_then(|o| o.bool_param("enabled")).unwrap_or(true),
                        host: out.and_then(|o| o.str_param("remoteHost")).unwrap_or("127.0.0.1").to_string(),
                        port: p("remotePort").unwrap_or(6454) as u16,
                        net: p("net").unwrap_or(0) as u8,
                        subnet: p("subnet").unwrap_or(0) as u8,
                        universe: p("universe").unwrap_or(0) as u8,
                        binds_6454: n.container_path(&["artNet", "input"]).and_then(|i| i.bool_param("enabled")).unwrap_or(false)
                            || out.and_then(|o| o.bool_param("forceSrcPort")).unwrap_or(false),
                    })
                }
                "sACN" => {
                    let dev = n.container("sACN");
                    let out = dev.and_then(|d| d.container("output"));
                    let get = |name: &str| dev.and_then(|d| d.param(name)).or_else(|| out.and_then(|o| o.param(name)));
                    InterfaceKind::Sacn(SacnOutput {
                        universe: get("universe").and_then(Value::as_i64).unwrap_or(1) as u16,
                        multicast: get("multicast").and_then(Value::as_bool).unwrap_or(true),
                        host: get("remoteHost").and_then(Value::as_str).unwrap_or("127.0.0.1").to_string(),
                        port: get("remotePort").and_then(Value::as_i64).unwrap_or(5568) as u16,
                    })
                }
                other => InterfaceKind::OtherDmx(other.to_string()),
            }
        }
        other => InterfaceKind::NonDmx(other.unwrap_or("").to_string()),
    };
    Interface { name, short_name, kind }
}

fn parse_fixture_type(n: Node) -> FixtureType {
    let name = nice_name_or(n, "FixtureType");
    let short_name = short_name::to_short_name(&name, false);

    // Offsets are recomputed the way FixtureTypeChannelManager::calcDmxChannels
    // does it; the stored `dmxChannel` is only a cache of that.
    let mut current = 1u16;
    let mut channels: Vec<Channel> = n
        .container("channels")
        .into_iter()
        .flat_map(|c| c.items())
        .map(|c| {
            let channel_type = c.str_param("channelType").unwrap_or("").to_string();
            let attribute = Attribute::from_channel_type(channel_type.rsplit('/').next().unwrap_or(""));
            let resolution = match c.str_param("resolution") {
                Some("16bits") => Resolution::Bits16,
                Some("fine") => Resolution::FineOnly,
                _ => Resolution::Bits8,
            };
            let offset = current;
            current += if resolution == Resolution::Bits16 { 2 } else { 1 };
            let physical_range = c
                .f32_array_param::<2>("physicalRange")
                .filter(|r| r[0] != 0.0 || r[1] != 0.0);
            let name = nice_name_or(c, "Channel");
            Channel {
                short_name: short_name::to_short_name(&name, false),
                name,
                channel_type,
                attribute,
                resolution,
                offset,
                fine_offset: (resolution == Resolution::Bits16).then_some(offset + 1),
                sub_fixture: c.i64_param("subFixtureID").unwrap_or(0).max(0) as u32,
                default_value: c.f64_param("defaultValue").unwrap_or(0.0) as f32,
                invert: c.bool_param("invertOutput").unwrap_or(false),
                physical_range,
            }
        })
        .collect();

    // Pair "fine channel only" entries with their 8-bit coarse channel.
    for i in 0..channels.len() {
        if channels[i].resolution != Resolution::FineOnly {
            continue;
        }
        let (fine_offset, ty, sub) = (channels[i].offset, channels[i].channel_type.clone(), channels[i].sub_fixture);
        if let Some(coarse) = channels.iter_mut().find(|c| {
            c.resolution == Resolution::Bits8 && c.fine_offset.is_none() && c.channel_type == ty && c.sub_fixture == sub
        }) {
            coarse.fine_offset = Some(fine_offset);
        }
    }

    FixtureType { name, short_name, channels }
}

fn parse_fixture(n: Node, show: &Show, warnings: &mut Vec<String>) -> Fixture {
    let id = n.i64_param("id").unwrap_or(1);
    let name = n.str_param("name").unwrap_or("New Fixture").to_string();
    let position = n.f32_array_param::<3>("position");
    let rotation = n.f32_array_param::<3>("rotation");

    let type_ref = n.str_param("fixtureType").unwrap_or("");
    let fixture_type = find_by_reference(&show.fixture_types, type_ref, |t| &t.short_name);
    if fixture_type.is_none() {
        warnings.push(format!("Fixture {id} ({name}): fixture type {type_ref:?} not found"));
    }

    let mut patches = Vec::new();
    for p in n.container("patch").into_iter().flat_map(|c| c.items()) {
        let address = p.i64_param("address").unwrap_or(0).clamp(0, 512) as u16;
        let iface_ref = p.str_param("interface").unwrap_or("");
        let interface = find_by_reference(&show.interfaces, iface_ref, |i| &i.short_name);
        if interface.is_none() && address > 0 {
            warnings.push(format!("Fixture {id} ({name}): interface {iface_ref:?} not found"));
        }
        patches.push(Patch { enabled: p.bool_param("enabled").unwrap_or(true), interface, address });
    }

    let nice = nice_name_or(n, "Fixture");
    Fixture {
        id,
        name,
        fixture_type,
        position: position.unwrap_or_default(),
        rotation: rotation.unwrap_or_default(),
        has_placement: position.is_some() || rotation.is_some(),
        patches,
        short_name: short_name::to_short_name(&nice, false),
    }
}

/// One "+ fixture 11 thru 14" style rule of a group (`CommandSelection`).
struct SelectionRule {
    add: bool,
    group: bool,
    from: i64,
    to: i64,
}

struct RawGroup {
    id: i64,
    name: String,
    rules: Vec<SelectionRule>,
}

impl SelectionRule {
    fn new(action: Option<&str>, target: Option<&str>, from: i64, thru: bool, last: Option<i64>) -> Self {
        SelectionRule { add: action != Some("-"), group: target == Some("Group"), from, to: if thru { last.unwrap_or(from) } else { from } }
    }
}

fn parse_groups(n: Node, fixtures: &[Fixture]) -> Vec<Group> {
    let raw = n
        .items()
        .enumerate()
        .map(|(i, g)| {
            let nice = nice_name_or(g, "Group");
            // `ID` keeps its default (1) out of the file, so fall back to the "N - Name" prefix.
            let id = g
                .i64_param("id")
                .or_else(|| nice.split(" - ").next().and_then(|p| p.trim().parse().ok()))
                .unwrap_or(i as i64 + 1);
            let name = g.str_param("name").map(str::to_string).unwrap_or_else(|| nice.clone());
            let rules = g
                .container("selections")
                .into_iter()
                .flat_map(|c| c.items())
                .filter(|r| r.bool_param("enabled").unwrap_or(true))
                .map(|r| {
                    SelectionRule::new(
                        r.str_param("action"),
                        r.str_param("targetType"),
                        r.i64_param("id").unwrap_or(0),
                        r.bool_param("thru").unwrap_or(false),
                        r.i64_param("lastID"),
                    )
                })
                .collect();
            RawGroup { id, name, rules }
        })
        .collect();
    resolve_groups(raw, fixtures)
}

/// Groups from BK's remote control (`GET /group`, OSCQuery JSON), which also
/// has groups that aren't saved yet.
pub fn groups_from_remote(v: &Value, fixtures: &[Fixture]) -> Vec<Group> {
    fn contents(v: &Value) -> impl Iterator<Item = &Value> {
        v.get("CONTENTS").and_then(Value::as_object).into_iter().flat_map(|m| m.values())
    }
    fn value<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        v.get("CONTENTS")?.get(key)?.get("VALUE")?.get(0)
    }
    let raw = contents(v)
        .filter(|g| g.get("TYPE").and_then(Value::as_str) == Some("Group"))
        .map(|g| {
            let rules = g
                .get("CONTENTS")
                .and_then(|c| c.get("selections"))
                .into_iter()
                .flat_map(contents)
                .filter(|r| r.get("TYPE").and_then(Value::as_str) == Some("BaseItem"))
                .filter(|r| value(r, "enabled").and_then(Value::as_bool).unwrap_or(true))
                .map(|r| {
                    SelectionRule::new(
                        value(r, "action").and_then(Value::as_str),
                        value(r, "targetType").and_then(Value::as_str),
                        value(r, "id").and_then(Value::as_i64).unwrap_or(0),
                        value(r, "thru").and_then(Value::as_bool).unwrap_or(false),
                        value(r, "lastID").and_then(Value::as_i64),
                    )
                })
                .collect();
            RawGroup {
                id: value(g, "id").and_then(Value::as_i64).unwrap_or(0),
                name: value(g, "name").and_then(Value::as_str).unwrap_or("").to_string(),
                rules,
            }
        })
        .collect();
    resolve_groups(raw, fixtures)
}

fn resolve_groups(raw: Vec<RawGroup>, fixtures: &[Fixture]) -> Vec<Group> {
    fn resolve(id: i64, raw: &[RawGroup], fixtures: &[Fixture], depth: u8) -> Vec<i64> {
        let mut out: Vec<i64> = Vec::new();
        let Some(g) = raw.iter().find(|g| g.id == id) else { return out };
        // Groups may contain groups; stop silly cycles.
        if depth > 8 {
            return out;
        }
        for r in &g.rules {
            let (lo, hi) = (r.from.min(r.to), r.from.max(r.to));
            let mut ids: Vec<i64> = if r.group {
                (lo..=hi).flat_map(|g| resolve(g, raw, fixtures, depth + 1)).collect()
            } else {
                // Only fixtures that exist, in the order of the rule.
                let mut v: Vec<i64> = fixtures.iter().map(|f| f.id).filter(|f| (lo..=hi).contains(f)).collect();
                v.sort_unstable();
                if r.from > r.to {
                    v.reverse();
                }
                v
            };
            if r.add {
                ids.retain(|i| !out.contains(i));
                out.extend(ids);
            } else {
                out.retain(|i| !ids.contains(i));
            }
        }
        out
    }

    let mut groups: Vec<Group> = raw.iter().map(|g| Group { id: g.id, name: g.name.clone(), fixtures: resolve(g.id, &raw, fixtures, 0) }).collect();
    groups.sort_by_key(|g| g.id);
    groups
}

fn parse_cuelist(n: Node) -> Cuelist {
    Cuelist {
        short_name: short_name::to_short_name(&nice_name_or(n, "Cuelist"), false),
        id: n.i64_param("id").unwrap_or(1),
        name: n.str_param("name").unwrap_or("New cuelist").to_string(),
        cues: n
            .container("cues")
            .into_iter()
            .flat_map(|c| c.items())
            .map(|c| Cue {
                id: c.f64_param("id").unwrap_or(1.0),
                name: nice_name_or(c, "Cue"),
                chapter: c.str_param("chapterName").unwrap_or("").to_string(),
            })
            .collect(),
    }
}
