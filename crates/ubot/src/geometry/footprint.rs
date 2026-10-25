use cadcraft_geom::{Bounds2, Vec2, point_in_polygon};

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

    fn bounds(&self) -> Bounds2 {
        Bounds2::from_points(self.corners)
    }

    fn center(&self) -> Vec2 {
        self.corners[0].mid(self.corners[2])
    }
}

fn ring_edges(ring: &Ring) -> impl Iterator<Item = (usize, Vec2, Vec2)> + '_ {
    (0..ring.edge_count()).filter_map(|i| ring.edge(i).map(|(a, b)| (i, a, b)))
}

fn distance_to_boundary(poly: &[Vec2], p: Vec2) -> f64 {
    let n = poly.len();
    (0..n)
        .filter_map(|i| Some((*poly.get(i)?, *poly.get((i + 1) % n)?)))
        .map(|(a, b)| segment::dist_to_segment(a, b, p))
        .fold(f64::INFINITY, f64::min)
}

fn strictly_inside(poly: &[Vec2], p: Vec2) -> bool {
    point_in_polygon(poly, p) && distance_to_boundary(poly, p) > POINT_TOL_MM
}

fn inside_or_on(poly: &[Vec2], p: Vec2) -> bool {
    point_in_polygon(poly, p) || distance_to_boundary(poly, p) <= POINT_TOL_MM
}

fn crosses_ring(fp: &Footprint, ring: &Ring) -> bool {
    let fb = fp.bounds().expand(POINT_TOL_MM);
    ring_edges(ring).filter(|(_, c, d)| fb.intersects(&Bounds2::from_points([*c, *d]))).any(|(_, c, d)| {
        (0..4).any(|k| {
            let (a, b) = fp.edge(k);
            segment::cross_properly(a, b, c, d)
        })
    })
}

fn ring_vertex_strictly_inside_footprint(fp: &Footprint, ring: &Ring) -> bool {
    let fb = fp.bounds();
    ring.points.iter().any(|v| fb.contains(*v) && strictly_inside(&fp.corners, *v))
}

/// True when the footprint lies wholly inside the ring; touching the boundary is allowed.
pub fn ring_contains_footprint(ring: &Ring, fp: &Footprint) -> bool {
    if !fp.is_finite() || ring.edge_count() < 3 {
        return false;
    }
    let pts = &ring.points;
    let inside = (0..4).all(|k| {
        let (a, b) = fp.edge(k);
        inside_or_on(pts, a) && inside_or_on(pts, a.mid(b))
    });
    inside && !crosses_ring(fp, ring) && !ring_vertex_strictly_inside_footprint(fp, ring)
}

/// True when the footprint overlaps the ring's interior or crosses its boundary; mere touching is not overlap.
pub fn footprint_intersects_ring(fp: &Footprint, ring: &Ring) -> bool {
    if !fp.is_finite() || ring.edge_count() < 3 {
        return false;
    }
    let pts = &ring.points;
    crosses_ring(fp, ring)
        || fp.corners.iter().any(|c| strictly_inside(pts, *c))
        || strictly_inside(pts, fp.center())
        || ring_vertex_strictly_inside_footprint(fp, ring)
}

/// Minimum distance from the footprint boundary to the ring's edges (0 on contact) and whether the nearest edge is an arc.
pub fn distance_footprint_ring_edges(fp: &Footprint, ring: &Ring) -> (f64, bool) {
    if !fp.is_finite() {
        return (f64::INFINITY, false);
    }
    let mut best = (f64::INFINITY, false);
    for (i, c, d) in ring_edges(ring) {
        let dist = (0..4).map(|k| fp.edge(k)).map(|(a, b)| segment::distance(a, b, c, d)).fold(f64::INFINITY, f64::min);
        if dist < best.0 {
            best = (dist, ring.curved.get(i).copied().unwrap_or(false));
        }
    }
    best
}
