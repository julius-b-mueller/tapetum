use bk_olga::light;
use bk_olga::{Attribute, FixtureClass, InterfaceKind, Resolution};

fn show() -> bk_olga::Show {
    bk_olga::load(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/show.olga")).unwrap()
}

#[test]
fn reads_fixtures_and_patch() {
    let show = show();
    assert!(show.warnings.is_empty(), "{:?}", show.warnings);
    assert_eq!(show.fixtures.len(), 13);
    assert_eq!(show.fixture_types.len(), 5);

    let mh1 = show.fixtures.iter().find(|f| f.id == 11).unwrap();
    assert_eq!(mh1.name, "MH 1");
    assert_eq!(show.fixture_type(mh1).unwrap().name, "Moving Head Wash");
    assert!(!mh1.has_placement);
    assert_eq!(mh1.patches.len(), 1);
    assert_eq!(mh1.patches[0].address, 150);
    assert_eq!(show.interface(&mh1.patches[0]).unwrap().name, "DMX1");

    let lk = show.fixtures.iter().find(|f| f.id == 51).unwrap();
    assert_eq!(show.interface(&lk.patches[0]).unwrap().name, "DMX2");
    assert_eq!(lk.patches[0].address, 500);
}

#[test]
fn interfaces_with_defaults() {
    let show = show();
    let InterfaceKind::ArtNet(dmx1) = &show.interfaces[0].kind else { panic!() };
    assert_eq!((dmx1.host.as_str(), dmx1.port, dmx1.net, dmx1.universe), ("10.0.0.11", 6454, 0, 0));
    assert_eq!(dmx1.port_address(), 0);
    let InterfaceKind::ArtNet(dmx2) = &show.interfaces[1].kind else { panic!() };
    assert_eq!(dmx2.port_address(), 0x100);
    assert!(matches!(show.interfaces[2].kind, InterfaceKind::NonDmx(_)));
}

#[test]
fn channel_offsets_and_classes() {
    let show = show();
    let by_name = |n: &str| show.fixture_types.iter().find(|t| t.name == n).unwrap();

    let head = by_name("Moving Head Wash");
    assert_eq!(head.classify(), FixtureClass::MovingHead);
    assert_eq!(head.cells(), (1..=7).collect::<Vec<_>>());
    let pan = &head.channels[0];
    assert_eq!((pan.attribute.clone(), pan.resolution, pan.offset, pan.fine_offset), (Attribute::Pan, Resolution::Bits16, 1, Some(2)));
    assert_eq!(pan.physical_range, Some([-270.0, 270.0]));
    let zoom = head.channels.iter().find(|c| c.attribute == Attribute::Zoom).unwrap();
    assert_eq!((zoom.offset, zoom.physical_range), (6, Some([38.0, 1.0])));
    assert_eq!(head.footprint(), 48);

    let bar = by_name("Bar RGBW");
    assert_eq!(bar.classify(), FixtureClass::Bar);
    assert_eq!(bar.cells().len(), 6);
    assert_eq!(bar.footprint(), 48);

    assert_eq!(by_name("Theatre Spot RGBW").classify(), FixtureClass::Wash);
    assert_eq!(by_name("Water Effect").classify(), FixtureClass::Wash);
    assert_eq!(by_name("Channel").classify(), FixtureClass::Conventional);
}

#[test]
fn conductor_cues() {
    let show = show();
    let main = show.conductor().unwrap();
    assert_eq!(main.name, "Main");
    assert_eq!(main.cues[0].name, "Einlass (M)");
    assert_eq!(main.cues[0].id, 1.0);
    let strand = main.cues.iter().find(|c| c.name == "Strand (M)").unwrap();
    assert_eq!(strand.chapter, "1 - Kapitel 1");
}

#[test]
fn decode_moving_head() {
    let show = show();
    let head = show.fixture_types.iter().find(|t| t.name == "Moving Head Wash").unwrap();
    let mut dmx = [0u8; 512];
    // 0-based index of fixture channel `k` (1-based) when patched at 150.
    let ch = |k: usize| 150 + k - 2;
    dmx[ch(1)] = 0xff; // pan MSB
    dmx[ch(2)] = 0xff; // pan LSB
    dmx[ch(3)] = 0x80; // tilt MSB -> ~center
    dmx[ch(6)] = 0; // zoom 0 -> 38°
    dmx[ch(7)] = 255; // dimmer
    dmx[ch(9)] = 255; // cell 1 red
    dmx[ch(14)] = 255; // cell 2 green

    let state = head.decode(150, &dmx);
    let pan = state.get(None, &Attribute::Pan).unwrap();
    assert_eq!(pan.physical([0.0, 0.0]), 270.0);
    let tilt = state.get(None, &Attribute::Tilt).unwrap().physical([0.0, 0.0]);
    assert!(tilt.abs() < 1.0, "{tilt}");
    assert_eq!(state.get(Some(1), &Attribute::Zoom).unwrap().physical([0.0, 0.0]), 38.0);

    let c1 = light::compute(&state, Some(1), FixtureClass::MovingHead);
    assert_eq!((c1.intensity, c1.color), (1.0, [1.0, 0.0, 0.0]));
    let c2 = light::compute(&state, Some(2), FixtureClass::MovingHead);
    assert_eq!(c2.color, [0.0, 1.0, 0.0]);
    let c3 = light::compute(&state, Some(3), FixtureClass::MovingHead);
    assert_eq!(c3.intensity, 0.0);

    let mixed = light::mix(&[c1, c2, c3]);
    assert!((mixed.intensity - 2.0 / 3.0).abs() < 1e-6);
    assert_eq!(mixed.color, [1.0, 1.0, 0.0]);
}

#[test]
fn decode_conventional_dimmer() {
    let show = show();
    let ch = show.fixture_types.iter().find(|t| t.name == "Channel").unwrap();
    let mut dmx = [0u8; 512];
    dmx[499] = 128;
    let out = light::compute(&ch.decode(500, &dmx), None, ch.classify());
    assert!((out.intensity - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(out.color, [1.0, 0.68 / 1.0, 0.38]);
}

#[test]
fn strobe_idle_at_default_value() {
    let show = show();
    let wf = show.fixture_types.iter().find(|t| t.name == "Water Effect").unwrap();
    let mut dmx = [0u8; 512];
    let ch = |k: usize| 400 + k - 2;
    dmx[ch(1)] = 255; // red
    dmx[ch(5)] = 255; // strobe at its default (1.0) = open
    dmx[ch(6)] = 255; // dimmer
    assert_eq!(light::compute(&wf.decode(400, &dmx), None, wf.classify()).strobe_hz, None);
    dmx[ch(5)] = 128;
    assert!(light::compute(&wf.decode(400, &dmx), None, wf.classify()).strobe_hz.is_some());
}

#[test]
fn channel_addresses() {
    let show = show();
    let ty = show.fixture_types.iter().find(|t| t.name == "Moving Head Wash").unwrap();
    let pan = ty.channels.iter().find(|c| c.attribute == Attribute::Pan).unwrap();
    assert_eq!(pan.name, "Channel 1 + 2 - Pan");
    assert_eq!(bk_olga::Show::channel_address(ty, pan, "physicalRange"), "/fixtureType/movingHeadWash/channels/channel1_2_Pan/physicalRange");
}

#[test]
fn groups() {
    let show = show();
    let g: Vec<(i64, &str, Vec<i64>)> = show.groups.iter().map(|g| (g.id, g.name.as_str(), g.fixtures.clone())).collect();
    assert_eq!(g[0], (1, "MH", vec![11, 12, 13, 14]));
    assert_eq!(g[1], (2, "Bar", vec![21, 22, 23, 24]));
    assert_eq!(g[2].2, vec![31, 32]);
    assert_eq!(g.len(), 5);
}

#[test]
fn groups_from_remote_control() {
    let show = show();
    let json: serde_json::Value = serde_json::from_str(include_str!("data/remote_group.json")).unwrap();
    let groups = bk_olga::groups_from_remote(&json, &show.fixtures);
    let g = |id: i64| groups.iter().find(|g| g.id == id).unwrap();
    // "+ 11 thru 14", "- 12", and a disabled "+ 21".
    assert_eq!(g(1).fixtures, [11, 13, 14]);
    assert_eq!(g(11).name, "MHs Links");
    assert_eq!(g(11).fixtures, [11, 12]);
    assert_eq!(g(12).fixtures, [13, 14]);
}
