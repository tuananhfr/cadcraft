use cadcraft_geom::{Vec2, point_in_polygon, shoelace};

use super::POINT_TOL_MM;
use super::ring::Ring;
use super::segment;

/// Rotated rectangle of a box, corners in CCW order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub corners: [Vec2; 4],
}

pub fn footprint(center: Vec2, w_mm: f64, l_mm: f64, angle_deg: f64) -> Footprint {
    let (hw, hl) = (w_mm / 2.0, l_mm / 2.0);
    let angle = angle_deg.to_radians();
    let local = [Vec2::new(-hw, -hl), Vec2::new(hw, -hl), Vec2::new(hw, hl), Vec2::new(-hw, hl)];
    Footprint { corners: local.map(|p| center + p.rotate(angle)) }
}

impl Footprint {
    fn is_finite(&self) -> bool {
        self.corners.iter().all(|c| c.is_finite())
    }

    fn edge(&self, i: usize) -> (Vec2, Vec2) {
        let [a, b, c, d] = self.corners;
        match i % 4 {
            0 => (a, b),
            1 => (b, c),
            2 => (c, d),
            _ => (d, a),
        }
    }

    fn center(&self) -> Vec2 {
        self.corners[0].mid(self.corners[2])
    }
}

/// Candidate edges as endpoints; indices out of range are ignored.
fn edges_of<'a>(ring: &'a Ring, edges: impl IntoIterator<Item = usize> + 'a) -> impl Iterator<Item = (usize, Vec2, Vec2)> + 'a {
    edges.into_iter().filter_map(move |i| ring.edge(i).map(|(a, b)| (i, a, b)))
}

/// True when segment c-d passes through the open interior of the footprint, shrunk inward by `POINT_TOL_MM`.
/// Liang-Barsky against the four half-planes; exact for the convex box, so edges lying on or touching the boundary are not "entering".
fn edge_enters_interior(fp: &Footprint, c: Vec2, d: Vec2) -> bool {
    if shoelace(&fp.corners) <= 0.0 {
        return false;
    }
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for k in 0..4 {
        let (a, b) = fp.edge(k);
        let len = a.dist(b);
        if len <= POINT_TOL_MM {
            return false;
        }
        let side = |p: Vec2| (b - a).cross(p - a) / len - POINT_TOL_MM;
        let (sc, sd) = (side(c), side(d));
        let den = sd - sc;
        if den.abs() <= 1e-12 {
            if sc <= 0.0 {
                return false;
            }
        } else if den > 0.0 {
            t0 = t0.max(-sc / den);
        } else {
            t1 = t1.min(-sc / den);
        }
        if t0 >= t1 {
            return false;
        }
    }
    true
}

/// True when any of `edges` passes through the footprint interior; lets callers that know the centre side skip the centre test.
pub(crate) fn any_edge_enters(fp: &Footprint, ring: &Ring, edges: impl IntoIterator<Item = usize>) -> bool {
    edges_of(ring, edges).any(|(_, c, d)| edge_enters_interior(fp, c, d))
}

/// Without a ring edge through the box interior the interior is wholly inside or outside the ring, so the centre decides.
fn center_inside(fp: &Footprint, ring: &Ring) -> bool {
    point_in_polygon(&ring.points, fp.center())
}

fn all_edges(ring: &Ring) -> std::ops::Range<usize> {
    0..ring.edge_count()
}

/// True when the footprint lies wholly inside the ring; touching the boundary is allowed.
pub fn ring_contains_footprint(ring: &Ring, fp: &Footprint) -> bool {
    ring_contains_footprint_with(ring, fp, all_edges(ring))
}

/// Like [`ring_contains_footprint`] but only `edges` (ring edge indices, e.g. from `EdgeIndex::query` on the footprint bounds
/// with margin >= 1e-6 mm) are tested. With no candidate edge the centre point alone decides.
pub fn ring_contains_footprint_with(ring: &Ring, fp: &Footprint, edges: impl IntoIterator<Item = usize>) -> bool {
    fp.is_finite() && ring.edge_count() >= 3 && !any_edge_enters(fp, ring, edges) && center_inside(fp, ring)
}

/// True when the footprint overlaps the ring region (including lying inside it) or a ring edge crosses it; mere touching is not overlap.
pub fn footprint_intersects_ring(fp: &Footprint, ring: &Ring) -> bool {
    footprint_intersects_ring_with(fp, ring, all_edges(ring))
}

/// Candidate-edge variant of [`footprint_intersects_ring`]; with no candidate edge the centre point alone decides.
pub fn footprint_intersects_ring_with(fp: &Footprint, ring: &Ring, edges: impl IntoIterator<Item = usize>) -> bool {
    fp.is_finite() && ring.edge_count() >= 3 && (any_edge_enters(fp, ring, edges) || center_inside(fp, ring))
}

/// Minimum distance between the footprint boundary and the ring's edges, and whether any nearest edge is an arc.
/// It is 0 when the boundaries touch or cross and positive when one lies wholly inside the other, so combine with
/// `footprint_intersects_ring` / `ring_contains_footprint` for containment.
pub fn distance_footprint_ring_edges(fp: &Footprint, ring: &Ring) -> (f64, bool) {
    distance_footprint_ring_edges_with(fp, ring, all_edges(ring))
}

/// Candidate-edge variant of [`distance_footprint_ring_edges`]; `(INFINITY, false)` when there is no candidate edge.
pub fn distance_footprint_ring_edges_with(fp: &Footprint, ring: &Ring, edges: impl IntoIterator<Item = usize>) -> (f64, bool) {
    if !fp.is_finite() {
        return (f64::INFINITY, false);
    }
    const TIE_MM: f64 = 1e-9;
    let mut best = (f64::INFINITY, false);
    for (i, c, d) in edges_of(ring, edges) {
        let dist = (0..4).map(|k| fp.edge(k)).map(|(a, b)| segment::distance(a, b, c, d)).fold(f64::INFINITY, f64::min);
        let curved = ring.curved.get(i).copied().unwrap_or(false);
        if dist < best.0 - TIE_MM {
            best = (dist, curved);
        } else if dist <= best.0 + TIE_MM {
            best = (best.0.min(dist), best.1 || curved);
        }
    }
    best
}
