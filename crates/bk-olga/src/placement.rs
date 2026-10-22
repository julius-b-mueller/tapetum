//! BlinderKitten's 3D conventions, derived from `Tracker::computeValues`:
//! Z is up, a fixture pans around its local Z axis and tilts around its local
//! X axis, and at pan = tilt = 0 the beam points straight down (-Z). At
//! tilt = -90° it points horizontally towards -Y.

/// Row-major 3x3 matrix.
pub type Mat3 = [[f32; 3]; 3];

/// `juce::Matrix3D::rotation`, as a flat 4x4 array in JUCE's memory order.
fn juce_rotation(e: [f32; 3]) -> [f32; 16] {
    let (sx, cx) = e[0].sin_cos();
    let (sy, cy) = e[1].sin_cos();
    let (sz, cz) = e[2].sin_cos();
    [
        cy * cz + sx * sy * sz, cx * sz, cy * sx * sz - cz * sy, 0.0,
        cz * sx * sy - cy * sz, cx * cz, cy * cz * sx + sy * sz, 0.0,
        cx * sy, -sx, cx * cy, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Maps world directions into the fixture's frame exactly like BK's tracker
/// (`rotation(-fixtureRotation)` applied with its row-vector convention).
pub fn local_from_world(rotation_deg: [f32; 3]) -> Mat3 {
    let m = juce_rotation(rotation_deg.map(|d| -d.to_radians()));
    let mut l = [[0.0; 3]; 3];
    for (i, row) in l.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = m[4 * j + i];
        }
    }
    l
}

/// Orientation of the fixture in the world (inverse of `local_from_world`).
pub fn world_from_local(rotation_deg: [f32; 3]) -> Mat3 {
    transpose(local_from_world(rotation_deg))
}

/// BK rotation (degrees) for an orientation, the inverse of `world_from_local`.
pub fn rotation_from_world(w: Mat3) -> [f32; 3] {
    // `world_from_local(r)` is `juce_rotation(-r)` read row by row.
    let sx = (-w[2][1]).clamp(-1.0, 1.0);
    let x = sx.asin();
    let (y, z) = if sx.abs() < 0.9999 {
        (w[2][0].atan2(w[2][2]), w[0][1].atan2(w[1][1]))
    } else {
        // Looking straight along X: only y - z (or y + z) is defined, keep z = 0.
        ((-w[0][2]).atan2(w[0][0]) * sx.signum(), 0.0)
    };
    [x, y, z].map(|e| -e.to_degrees())
}

/// Turns an orientation by `angle_deg` around world axis `axis` (0 = X, 1 = Y,
/// 2 = Z; counterclockwise looking down the axis) and returns the BK rotation.
pub fn rotate_around_world_axis(rotation_deg: [f32; 3], axis: usize, angle_deg: f32) -> [f32; 3] {
    let (s, c) = angle_deg.to_radians().sin_cos();
    let mut r = [[0.0; 3]; 3];
    let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
    r[axis][axis] = 1.0;
    r[i][i] = c;
    r[j][j] = c;
    r[i][j] = -s;
    r[j][i] = s;
    let w = world_from_local(rotation_deg);
    let turned = [0, 1, 2].map(|row| [0, 1, 2].map(|col| (0..3).map(|k| r[row][k] * w[k][col]).sum()));
    rotation_from_world(turned)
}

/// Beam direction in the fixture frame for physical pan/tilt angles in degrees.
///
/// Equivalent to rotating the yoke by `-pan` around Z and the head by `tilt`
/// around X, starting from a beam pointing down.
pub fn beam_local(pan_deg: f32, tilt_deg: f32) -> [f32; 3] {
    let (sp, cp) = pan_deg.to_radians().sin_cos();
    let (st, ct) = tilt_deg.to_radians().sin_cos();
    [st * sp, st * cp, -ct]
}

/// Pan/tilt BK's tracker would compute for a direction in the fixture frame
/// (tilt in -180..=0).
pub fn pan_tilt_for_local(dir: [f32; 3]) -> (f32, f32) {
    let dist = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    let pan = -(90.0 + dir[1].atan2(dir[0]).to_degrees());
    let tilt = -(90.0 + dir[2].atan2(dist).to_degrees());
    (pan, tilt)
}

pub fn mul(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

pub fn transpose(m: Mat3) -> Mat3 {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| m[c][r]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    fn close_m(a: Mat3, b: Mat3) -> bool {
        (0..3).all(|r| close(a[r], b[r]))
    }

    #[test]
    fn rotation_roundtrip() {
        for r in [[10.0, 20.0, 30.0], [-45.0, 5.0, 170.0], [0.0, 0.0, 0.0], [60.0, -80.0, 15.0]] {
            let w = world_from_local(r);
            assert!(close_m(world_from_local(rotation_from_world(w)), w), "{r:?}");
        }
    }

    #[test]
    fn world_axis_rotation() {
        // A fixture tilted around X, then turned 90° around world Z: its beam
        // direction turns around Z like any other vector.
        let r = [30.0, 0.0, 0.0];
        let down = beam_local(0.0, 0.0);
        let before = mul(&world_from_local(r), down);
        let after = mul(&world_from_local(rotate_around_world_axis(r, 2, 90.0)), down);
        assert!(close(after, [-before[1], before[0], before[2]]), "{before:?} -> {after:?}");
    }

    #[test]
    fn home_position_points_down() {
        assert!(close(beam_local(0.0, 0.0), [0.0, 0.0, -1.0]));
        assert!(close(beam_local(0.0, -90.0), [0.0, -1.0, 0.0]));
        assert!(close(beam_local(90.0, -90.0), [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn inverse_of_tracker_formula() {
        for pan in [-270.0f32, -120.0, -10.0, 0.0, 45.0, 200.0] {
            for tilt in [-170.0f32, -100.0, -45.0, -5.0] {
                let dir = beam_local(pan, tilt);
                let (p2, t2) = pan_tilt_for_local(dir);
                assert!(close(beam_local(p2, t2), dir), "pan {pan} tilt {tilt} -> {p2} {t2}");
                assert!((t2 - tilt).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn rotations_are_orthonormal_inverses() {
        let rot = [30.0, -60.0, 110.0];
        let v = [0.3, -0.5, 0.8];
        let back = mul(&world_from_local(rot), mul(&local_from_world(rot), v));
        assert!(close(back, v));
        assert!(close(mul(&world_from_local([0.0; 3]), v), v));
    }

    #[test]
    fn rotation_about_x_tips_a_hanging_fixture() {
        // A down-pointing beam rotated by 45° around X faces diagonally down.
        let dir = mul(&world_from_local([45.0, 0.0, 0.0]), [0.0, 0.0, -1.0]);
        assert!((dir[2] + std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!(dir[0].abs() < 1e-4);
    }
}
