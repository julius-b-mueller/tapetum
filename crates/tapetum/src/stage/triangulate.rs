//! Planar polygons (with holes) to triangles: holes are bridged into the
//! outer ring, then ears are clipped. Plenty for architectural faces, which
//! are mostly quads and a few dozen corners at most.

use bevy::math::{Vec2, Vec3};

/// Triangles as index triples into the concatenation of `outer` and all `holes`.
pub fn triangulate(outer: &[Vec3], holes: &[Vec<Vec3>]) -> Vec<[u32; 3]> {
    if outer.len() < 3 {
        return Vec::new();
    }
    let normal = newell(outer);
    if normal.length_squared() < 1e-20 {
        return Vec::new();
    }
    // Work in 2D in the polygon's plane, outer ring counterclockwise.
    let (u, v) = plane_axes(normal.normalize());
    let to_2d = |p: Vec3| Vec2::new(p.dot(u), p.dot(v));
    let mut points: Vec<Vec2> = outer.iter().map(|&p| to_2d(p)).collect();
    let mut ring: Vec<u32> = (0..outer.len() as u32).collect();
    if signed_area(&ring, &points) < 0.0 {
        ring.reverse();
    }
    let mut hole_rings: Vec<Vec<u32>> = Vec::new();
    for h in holes.iter().filter(|h| h.len() >= 3) {
        let start = points.len() as u32;
        points.extend(h.iter().map(|&p| to_2d(p)));
        let mut r: Vec<u32> = (start..points.len() as u32).collect();
        // Holes clockwise.
        if signed_area(&r, &points) > 0.0 {
            r.reverse();
        }
        hole_rings.push(r);
    }
    // Bridge the holes from the rightmost one inwards (Eberly's method).
    hole_rings.sort_by(|a, b| max_x(b, &points).total_cmp(&max_x(a, &points)));
    for hole in hole_rings {
        ring = bridge(ring, &hole, &points);
    }
    clip_ears(ring, &points)
}

/// Unit normal of a polygon (zero if degenerate).
pub fn newell_normal(ring: &[Vec3]) -> Vec3 {
    newell(ring).normalize_or_zero()
}

fn newell(ring: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (i, a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n
}

fn plane_axes(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let u = helper.cross(n).normalize();
    (u, n.cross(u))
}

fn signed_area(ring: &[u32], p: &[Vec2]) -> f32 {
    let mut a = 0.0;
    for (i, &k) in ring.iter().enumerate() {
        let (p0, p1) = (p[k as usize], p[ring[(i + 1) % ring.len()] as usize]);
        a += p0.x * p1.y - p1.x * p0.y;
    }
    a / 2.0
}

fn max_x(ring: &[u32], p: &[Vec2]) -> f32 {
    ring.iter().map(|&i| p[i as usize].x).fold(f32::MIN, f32::max)
}

fn cross(o: Vec2, a: Vec2, b: Vec2) -> f32 {
    (a - o).perp_dot(b - o)
}

fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

/// Joins `hole` into `ring` with a two-way cut from the hole's rightmost
/// point to a ring vertex it can see.
fn bridge(ring: Vec<u32>, hole: &[u32], p: &[Vec2]) -> Vec<u32> {
    let (hi, &h) = hole.iter().enumerate().max_by(|a, b| p[*a.1 as usize].x.total_cmp(&p[*b.1 as usize].x)).unwrap();
    let hp = p[h as usize];
    // Closest crossing of a ray to +X with the ring.
    let mut best: Option<(f32, usize)> = None;
    for i in 0..ring.len() {
        let (a, b) = (p[ring[i] as usize], p[ring[(i + 1) % ring.len()] as usize]);
        if (a.y > hp.y) == (b.y > hp.y) {
            continue;
        }
        let x = a.x + (hp.y - a.y) * (b.x - a.x) / (b.y - a.y);
        if x >= hp.x && best.is_none_or(|(bx, _)| x < bx) {
            // Of the edge, take the end further right as the candidate.
            best = Some((x, if a.x > b.x { i } else { (i + 1) % ring.len() }));
        }
    }
    let mut target = best.map_or(0, |(_, i)| i);
    if let Some((x, _)) = best {
        // A reflex vertex inside the triangle hole → crossing → candidate may
        // hide the candidate; then the one with the smallest angle wins.
        let m = Vec2::new(x, hp.y);
        let c = p[ring[target] as usize];
        let mut best_angle = f32::MAX;
        for (i, &k) in ring.iter().enumerate() {
            let q = p[k as usize];
            if i != target && q.x >= hp.x && (in_triangle(q, hp, m, c) || in_triangle(q, hp, c, m)) {
                let angle = (q.y - hp.y).abs().atan2(q.x - hp.x);
                if angle < best_angle {
                    best_angle = angle;
                    target = i;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(ring.len() + hole.len() + 2);
    out.extend_from_slice(&ring[..=target]);
    out.extend(hole[hi..].iter().chain(&hole[..=hi]));
    out.extend_from_slice(&ring[target..]);
    out
}

fn clip_ears(mut ring: Vec<u32>, p: &[Vec2]) -> Vec<[u32; 3]> {
    let mut tris = Vec::with_capacity(ring.len().saturating_sub(2));
    let mut guard = 0;
    while ring.len() > 3 && guard < ring.len() * ring.len() + 16 {
        guard += 1;
        let n = ring.len();
        let mut clipped = false;
        for i in 0..n {
            let (ia, ib, ic) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let (a, b, c) = (p[ia as usize], p[ib as usize], p[ic as usize]);
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            // No other corner may sit inside the ear (bridge duplicates excepted).
            let blocked = ring.iter().any(|&k| {
                let q = p[k as usize];
                k != ia && k != ib && k != ic && q != a && q != b && q != c && in_triangle(q, a, b, c)
            });
            if !blocked {
                tris.push([ia, ib, ic]);
                ring.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // Degenerate leftovers (collinear runs): fan them rather than loop.
            break;
        }
    }
    if ring.len() >= 3 {
        for i in 1..ring.len() - 1 {
            tris.push([ring[0], ring[i], ring[i + 1]]);
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(tris: &[[u32; 3]], pts: &[Vec3]) -> f32 {
        tris.iter().map(|t| (pts[t[1] as usize] - pts[t[0] as usize]).cross(pts[t[2] as usize] - pts[t[0] as usize]).length() / 2.0).sum()
    }

    #[test]
    fn square() {
        let sq = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        let t = triangulate(&sq, &[]);
        assert_eq!(t.len(), 2);
        assert!((area(&t, &sq) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn concave_l_shape() {
        let l = [
            Vec3::ZERO,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
        ];
        let t = triangulate(&l, &[]);
        assert_eq!(t.len(), 4);
        assert!((area(&t, &l) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn square_with_hole() {
        // A 4x4 wall (vertical, like a window opening) with a 2x2 hole.
        let outer = [Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 4.0), Vec3::new(0.0, 0.0, 4.0)];
        let hole = vec![Vec3::new(1.0, 0.0, 1.0), Vec3::new(3.0, 0.0, 1.0), Vec3::new(3.0, 0.0, 3.0), Vec3::new(1.0, 0.0, 3.0)];
        let t = triangulate(&outer, std::slice::from_ref(&hole));
        let mut all = outer.to_vec();
        all.extend(&hole);
        assert!((area(&t, &all) - 12.0).abs() < 1e-4, "{}", area(&t, &all));
    }
}
