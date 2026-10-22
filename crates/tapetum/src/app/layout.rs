//! Where fixtures go when BlinderKitten has no position for them.

use bk_olga::placement::{beam_local, mul, world_from_local};
use bk_olga::{FixtureClass, Show};
use tapetum_project::{Placement, Project};

pub const TRUSS_HEIGHT: f32 = 6.0;

/// Placement of every fixture, in BK coordinates (meters / degrees, Z up).
/// Placements edited in the visualizer win until BK has saved them, then
/// BK's own, then an automatic rig: one truss row per ID decade (11-19, 21-29, ...).
pub fn placements(show: &Show, project: &Project) -> Vec<Placement> {
    let mut groups: Vec<i64> = show.fixtures.iter().map(|f| f.id / 10).collect();
    groups.sort_unstable();
    groups.dedup();

    show.fixtures
        .iter()
        .map(|f| {
            if let Some(p) = project.pending_placements.get(&f.id) {
                return *p;
            }
            if f.has_placement {
                return Placement { position: f.position, rotation: f.rotation };
            }
            let row = groups.iter().position(|&g| g == f.id / 10).unwrap_or(0);
            let members: Vec<_> = show.fixtures.iter().filter(|o| o.id / 10 == f.id / 10).collect();
            let idx = members.iter().position(|o| o.id == f.id).unwrap_or(0);
            let spacing = 1.6;
            let x = (idx as f32 - (members.len() as f32 - 1.0) / 2.0) * spacing;
            let y = row as f32 * 1.5 - 3.0;
            let position = [x, y, TRUSS_HEIGHT];

            let class = show.fixture_type(f).map_or(FixtureClass::Unknown, |t| t.classify());
            let rotation = if class == FixtureClass::MovingHead {
                [0.0; 3]
            } else {
                // Static fixtures are focused onto the stage (1 m platform) in front of them.
                let target = [x * 0.6, 0.0, 1.0];
                aim_rotation(position, target)
            };
            Placement { position, rotation }
        })
        .collect()
}

/// BK rotation (X and Z only) that points a hanging fixture's beam at `target`.
pub fn aim_rotation(from: [f32; 3], target: [f32; 3]) -> [f32; 3] {
    let d = [target[0] - from[0], target[1] - from[1], target[2] - from[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let want = d.map(|c| c / len);
    let down = beam_local(0.0, 0.0);
    let score = |rx: f32, rz: f32| {
        let v = mul(&world_from_local([rx, 0.0, rz]), down);
        v[0] * want[0] + v[1] * want[1] + v[2] * want[2]
    };
    // Coarse grid, then refine; robust against Euler-order surprises.
    let mut best = (0.0, 0.0, f32::MIN);
    for rx in (-90..=90).step_by(3) {
        for rz in (-180..180).step_by(3) {
            let s = score(rx as f32, rz as f32);
            if s > best.2 {
                best = (rx as f32, rz as f32, s);
            }
        }
    }
    let (mut rx, mut rz) = (best.0, best.1);
    let mut step = 1.5;
    while step > 0.05 {
        let mut improved = false;
        for (dx, dz) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
            if score(rx + dx, rz + dz) > score(rx, rz) {
                rx += dx;
                rz += dz;
                improved = true;
            }
        }
        if !improved {
            step /= 2.0;
        }
    }
    [rx, 0.0, rz]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aims_at_target() {
        let from = [2.0, -3.0, 6.0];
        let to = [0.0, 0.0, 0.0];
        let rot = aim_rotation(from, to);
        let v = mul(&world_from_local(rot), beam_local(0.0, 0.0));
        let d = [-2.0f32, 3.0, -6.0];
        let len = (4.0f32 + 9.0 + 36.0).sqrt();
        let dot = v[0] * d[0] / len + v[1] * d[1] / len + v[2] * d[2] / len;
        assert!(dot > 0.9999, "{dot} {rot:?}");
    }
}
