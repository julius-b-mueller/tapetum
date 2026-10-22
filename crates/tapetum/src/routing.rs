//! Which Art-Net port-address the visualizer receives a BK interface on.

use bk_olga::{InterfaceKind, Patch, Show};
use tapetum_project::ArtNetSettings;

/// Explicit routes win, then a `Tapetum X` mirror interface; otherwise an
/// Art-Net interface is received on its own port-address.
pub fn port_address_for(show: &Show, settings: &ArtNetSettings, patch: &Patch) -> Option<u16> {
    let index = patch.interface?;
    let iface = &show.interfaces[index];
    if let Some((&pa, _)) = settings.routes.iter().find(|(_, name)| **name == iface.short_name) {
        return Some(pa);
    }
    let iface = show.mirror_of(index).filter(|m| matches!(m.kind, InterfaceKind::ArtNet(_))).unwrap_or(iface);
    match &iface.kind {
        InterfaceKind::ArtNet(a) => Some(a.port_address()),
        _ => None,
    }
}

/// First enabled, patched patch of a fixture together with its port-address.
pub fn primary_patch<'a>(show: &Show, settings: &ArtNetSettings, patches: &'a [Patch]) -> Option<(&'a Patch, u16)> {
    patches
        .iter()
        .filter(|p| p.enabled && p.address > 0)
        .find_map(|p| port_address_for(show, settings, p).map(|pa| (p, pa)))
}

pub fn format_port_address(pa: u16) -> String {
    format!("{}:{}:{}", pa >> 8, (pa >> 4) & 0xf, pa & 0xf)
}
