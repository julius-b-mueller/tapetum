//! Adds Tapetum's mirror interfaces to a show file.
//!
//! For every DMX interface `X` that fixtures are patched to, an Art-Net
//! interface `Tapetum X` sending to the visualizer on localhost is added, and
//! `X` gets a pass-through entry to it (`DMXInterface::thruManager`). The
//! patch stays untouched, so the real hardware keeps receiving its data.
//!
//! Edits work on the raw JSON so everything the visualizer doesn't model
//! survives unchanged.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::model::{Interface, InterfaceKind, Show};
use crate::short_name::to_short_name;

pub const PREFIX: &str = "Tapetum ";
/// Mirrors from before the rename; replaced by new ones when mirrors are added.
const LEGACY_PREFIX: &str = "bkviz ";

/// Short name of BK's `Pass-through` container.
const THRU: &str = "pass_through";

pub fn mirror_name(original: &str) -> String {
    format!("{PREFIX}{original}")
}

pub fn is_mirror_name(name: &str) -> bool {
    name.starts_with(PREFIX) || name.starts_with(LEGACY_PREFIX)
}

impl Show {
    /// The mirror of interface `index`, if the show has one (also an old `bkviz X`).
    pub fn mirror_of(&self, index: usize) -> Option<&Interface> {
        let original = &self.interfaces.get(index)?.name;
        let (name, legacy) = (mirror_name(original), format!("{LEGACY_PREFIX}{original}"));
        self.interfaces.iter().find(|i| i.name == name).or_else(|| self.interfaces.iter().find(|i| i.name == legacy))
    }
}

/// Address of an interface as a pass-through target (relative to the engine).
fn target_address(short_name: &str) -> String {
    format!("/interfaces/{short_name}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorEntry {
    pub original: String,
    pub mirror: String,
    pub port_address: u16,
    /// False if the mirror already existed and was only updated.
    pub created: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    #[error("the show has no interfaces section")]
    NoInterfaces,
    #[error("the show has no DMX interfaces")]
    NoDmxInterfaces,
}

/// Adds or updates the mirrors of all patched DMX interfaces.
/// They send Art-Net to `127.0.0.1:listen_port`.
pub fn add_mirrors(doc: &mut Value, listen_port: u16) -> Result<Vec<MirrorEntry>, MirrorError> {
    // Old `bkviz X` mirrors give way to `Tapetum X`.
    remove_where(doc, |n| n.starts_with(LEGACY_PREFIX));
    let show = Show::from_json(doc);
    let originals = originals(&show);
    if originals.is_empty() {
        return Err(MirrorError::NoDmxInterfaces);
    }
    let items = interface_items(doc).ok_or(MirrorError::NoInterfaces)?;

    // Mirrors of Art-Net interfaces keep their port-address when possible,
    // so universes read the same in BK and in the visualizer.
    let mut taken = BTreeSet::new();
    let mut wanted: Vec<(usize, Option<u16>)> = originals
        .iter()
        .map(|&i| match &show.interfaces[i].kind {
            InterfaceKind::ArtNet(a) => (i, Some(a.port_address())),
            InterfaceKind::Sacn(s) => (i, Some(s.universe.saturating_sub(1) & 0x7fff)),
            _ => (i, None),
        })
        .collect();
    for (_, pa) in &mut wanted {
        if let Some(p) = *pa
            && !taken.insert(p)
        {
            *pa = None;
        }
    }

    let mut entries = Vec::new();
    for (i, pa) in wanted {
        let pa = pa.unwrap_or_else(|| {
            let free = (0u16..0x8000).find(|p| !taken.contains(p)).unwrap_or(0);
            taken.insert(free);
            free
        });
        let original = &show.interfaces[i];
        let name = mirror_name(&original.name);
        let short = to_short_name(&name, false);

        let existing = items.iter().position(|it| it.get("niceName").and_then(Value::as_str) == Some(name.as_str()));
        let created = existing.is_none();
        let index = existing.unwrap_or_else(|| {
            items.push(json!({ "niceName": name, "type": "DMX" }));
            items.len() - 1
        });
        configure_mirror(&mut items[index], pa, listen_port);

        if let Some(orig) = items.iter_mut().find(|it| it.get("niceName").and_then(Value::as_str) == Some(original.name.as_str())) {
            ensure_thru(orig, &target_address(&short));
        }
        entries.push(MirrorEntry { original: original.name.clone(), mirror: name, port_address: pa, created });
    }
    Ok(entries)
}

/// Removes all mirror interfaces and the pass-through entries pointing to them.
/// Returns the names of the removed interfaces.
pub fn remove_mirrors(doc: &mut Value) -> Vec<String> {
    remove_where(doc, is_mirror_name)
}

fn remove_where(doc: &mut Value, is_mirror: impl Fn(&str) -> bool) -> Vec<String> {
    let Some(items) = interface_items(doc) else { return Vec::new() };
    let mut removed = Vec::new();
    items.retain(|it| match it.get("niceName").and_then(Value::as_str) {
        Some(n) if is_mirror(n) => {
            removed.push(n.to_string());
            false
        }
        _ => true,
    });
    let targets: Vec<String> = removed.iter().map(|n| target_address(&to_short_name(n, false))).collect();
    for it in items.iter_mut() {
        let Some(thru) = it.pointer_mut(&format!("/containers/{THRU}")).and_then(Value::as_object_mut) else { continue };
        if let Some(params) = thru.get_mut("parameters").and_then(Value::as_array_mut) {
            params.retain(|p| !p.get("value").and_then(Value::as_str).is_some_and(|v| targets.iter().any(|t| t == v)));
            // BK doesn't save an empty parameter list.
            if params.is_empty() {
                thru.remove("parameters");
            }
        }
    }
    removed
}

/// Indices of the non-mirror DMX interfaces that carry fixtures; all DMX
/// interfaces if nothing is patched yet.
fn originals(show: &Show) -> Vec<usize> {
    let is_dmx = |i: usize| {
        let iface = &show.interfaces[i];
        !is_mirror_name(&iface.name) && !matches!(iface.kind, InterfaceKind::NonDmx(_))
    };
    let used: BTreeSet<usize> =
        show.fixtures.iter().flat_map(|f| &f.patches).filter(|p| p.enabled).filter_map(|p| p.interface).filter(|&i| is_dmx(i)).collect();
    if used.is_empty() { (0..show.interfaces.len()).filter(|&i| is_dmx(i)).collect() } else { used.into_iter().collect() }
}

fn interface_items(doc: &mut Value) -> Option<&mut Vec<Value>> {
    let interfaces = doc.get_mut("interfaces")?.as_object_mut()?;
    interfaces.entry("items").or_insert_with(|| Value::Array(Vec::new())).as_array_mut()
}

fn configure_mirror(item: &mut Value, port_address: u16, listen_port: u16) {
    set_param(item, "dmxType", json!("Art-Net"));
    set_param(item, "enabled", json!(true));
    let art_net = child(child(item, "containers"), "artNet");
    // Input would bind the Art-Net port the visualizer listens on.
    let input = owned_container(child(art_net, "containers"), "input", "Input");
    set_param(input, "enabled", json!(false));
    let output = owned_container(child(art_net, "containers"), "output", "Output");
    set_param(output, "enabled", json!(true));
    set_param(output, "remoteHost", json!("127.0.0.1"));
    set_param(output, "remotePort", json!(listen_port));
    set_param(output, "net", json!(port_address >> 8));
    set_param(output, "subnet", json!((port_address >> 4) & 0xf));
    set_param(output, "universe", json!(port_address & 0xf));
}

/// Adds a pass-through target to `item` unless one already points to `target`.
fn ensure_thru(item: &mut Value, target: &str) {
    let thru = child(child(item, "containers"), THRU);
    let params = thru.as_object_mut().unwrap().entry("parameters").or_insert_with(|| Value::Array(Vec::new()));
    let Some(params) = params.as_array_mut() else { return };
    if let Some(p) = params.iter_mut().find(|p| p.get("value").and_then(Value::as_str) == Some(target)) {
        p["enabled"] = json!(true);
        return;
    }
    // Pass-through outputs must have unique names (allowSameChildrenNiceNames = false).
    let used: Vec<&str> = params.iter().filter_map(|p| p.get("niceName").and_then(Value::as_str)).collect();
    let nice = std::iter::once("Output Universe".to_string())
        .chain((2..).map(|n| format!("Output Universe {n}")))
        .find(|n| !used.contains(&n.as_str()))
        .unwrap();
    // The shape `DMXInterface::createThruControllable` saves (saveValueOnly = false).
    params.push(json!({
        "value": target,
        "controlAddress": format!("/{}", to_short_name(&nice, false)),
        "enabled": true,
        "type": "Target",
        "niceName": nice,
        "customizable": true,
        "removable": true,
        "description": "Target module to send the raw data to",
        "hideInEditor": false,
        "feedbackOnly": false
    }));
}

fn child<'a>(v: &'a mut Value, key: &str) -> &'a mut Value {
    if !v.is_object() {
        *v = Value::Object(Map::new());
    }
    let entry = v.as_object_mut().unwrap().entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry
}

fn owned_container<'a>(containers: &'a mut Value, key: &str, nice: &str) -> &'a mut Value {
    let c = child(containers, key);
    let o = c.as_object_mut().unwrap();
    o.entry("owned").or_insert(json!(true));
    o.entry("niceName").or_insert(json!(nice));
    c
}

fn set_param(container: &mut Value, name: &str, value: Value) {
    let address = format!("/{name}");
    let obj = container.as_object_mut().expect("container is an object");
    let params = obj.entry("parameters").or_insert_with(|| Value::Array(Vec::new()));
    let Some(params) = params.as_array_mut() else { return };
    match params.iter_mut().find(|p| p.get("controlAddress").and_then(Value::as_str) == Some(address.as_str())) {
        Some(p) => p["value"] = value,
        None => params.push(json!({ "value": value, "controlAddress": address })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show() -> Value {
        serde_json::from_str(include_str!("../tests/show.olga")).unwrap()
    }

    #[test]
    fn adds_mirrors_with_pass_through() {
        let mut doc = show();
        let entries = add_mirrors(&mut doc, 6454).unwrap();
        let names: Vec<_> = entries.iter().map(|e| (e.mirror.as_str(), e.port_address, e.created)).collect();
        assert_eq!(names, [("Tapetum DMX1", 0, true), ("Tapetum DMX2", 0x100, true)]);

        let s = Show::from_json(&doc);
        let dmx1 = s.interfaces.iter().position(|i| i.name == "DMX1").unwrap();
        let m = s.mirror_of(dmx1).unwrap();
        let InterfaceKind::ArtNet(a) = &m.kind else { panic!("{:?}", m.kind) };
        assert_eq!((a.host.as_str(), a.port, a.port_address()), ("127.0.0.1", 6454, 0));
        assert!(!a.binds_6454, "a mirror must not take the visualizer's port");
        // The original still goes to the hardware.
        let InterfaceKind::ArtNet(o) = &s.interfaces[dmx1].kind else { panic!() };
        assert_eq!(o.host, "10.0.0.11");

        let thru = doc.pointer("/interfaces/items/0/containers/pass_through/parameters").unwrap();
        assert_eq!(thru[0]["value"], "/interfaces/tapetumDMX1");
        assert_eq!(thru[0]["controlAddress"], "/outputUniverse");
        assert_eq!(thru[0]["type"], "Target");
    }

    #[test]
    fn adding_twice_is_idempotent() {
        let mut doc = show();
        add_mirrors(&mut doc, 6454).unwrap();
        let once = doc.clone();
        let entries = add_mirrors(&mut doc, 6454).unwrap();
        assert!(entries.iter().all(|e| !e.created));
        assert_eq!(doc, once);
    }

    #[test]
    fn old_bkviz_mirrors_are_replaced() {
        let mut doc = show();
        add_mirrors(&mut doc, 6454).unwrap();
        // As a show set up before the rename has them.
        let old = serde_json::to_string(&doc).unwrap().replace("Tapetum DMX", "bkviz DMX").replace("tapetumDMX", "bkvizDMX");
        let mut doc: Value = serde_json::from_str(&old).unwrap();
        let s = Show::from_json(&doc);
        assert!(s.mirror_of(0).is_some_and(|m| m.name == "bkviz DMX1"), "still recognized");
        add_mirrors(&mut doc, 6454).unwrap();
        let s = Show::from_json(&doc);
        let names: Vec<_> = s.interfaces.iter().map(|i| i.name.as_str()).filter(|n| is_mirror_name(n)).collect();
        assert_eq!(names, ["Tapetum DMX1", "Tapetum DMX2"]);
        let thru = doc.pointer("/interfaces/items/0/containers/pass_through/parameters").unwrap().as_array().unwrap();
        assert_eq!(thru.len(), 1, "{thru:?}");
        assert_eq!(thru[0]["value"], "/interfaces/tapetumDMX1");
    }

    #[test]
    fn remove_restores_original() {
        let original = show();
        let mut doc = original.clone();
        add_mirrors(&mut doc, 6455).unwrap();
        assert_eq!(remove_mirrors(&mut doc), ["Tapetum DMX1", "Tapetum DMX2"]);
        let s = Show::from_json(&doc);
        assert_eq!(s.interfaces.len(), Show::from_json(&original).interfaces.len());
        assert_eq!(doc, original);
    }
}
