use cadcraft_geom::{Bounds2, Vec2, point_in_polygon, shoelace};
use serde::{Deserialize, Serialize};

use super::tessellate::tessellate;
use super::{MAX_RING_VERTICES, POINT_TOL_MM, segment};
use crate::UbotError;

const MIN_AREA_MM2: f64 = 1e-6;

/// Closed ring as `[x, y, bulge]` in drawing units; the last vertex joins the first implicitly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RingInput {
    pub vertices: Vec<[f64; 3]>,
}

/// Validated simple ring in mm, counter-clockwise. `curved[i]` marks edge i (points[i] to points[i+1]) as arc-derived.
#[derive(Debug, Clone, PartialEq)]
pub struct Ring {
    pub points: Vec<Vec2>,
    pub curved: Vec<bool>,
}

impl Ring {
    /// Endpoints of edge `i`, or `None` when out of range.
    pub fn edge(&self, i: usize) -> Option<(Vec2, Vec2)> {
        let a = *self.points.get(i)?;
        let b = *self.points.get((i + 1) % self.points.len())?;
        Some((a, b))
    }

    pub fn edge_count(&self) -> usize {
        self.points.len()
    }

    pub fn bounds(&self) -> Bounds2 {
        Bounds2::from_points(self.points.iter().copied())
    }
}

pub fn point_in_ring(p: Vec2, ring: &Ring) -> bool {
    point_in_polygon(&ring.points, p)
}

pub fn ring_from_input(r: &RingInput, unit_scale: f64, tol_mm: f64) -> Result<Ring, UbotError> {
    if !(unit_scale.is_finite() && unit_scale > 0.0) {
        return Err(UbotError::BadInput("unit scale must be a positive finite number".into()));
    }
    if !(tol_mm.is_finite() && tol_mm > 0.0) {
        return Err(UbotError::BadInput("tolerance must be a positive finite number".into()));
    }
    if r.vertices.len() > MAX_RING_VERTICES {
        return Err(UbotError::Limit(format!("ring has {} vertices, the limit is {MAX_RING_VERTICES}", r.vertices.len())));
    }
    for v in &r.vertices {
        if !v.iter().all(|c| c.is_finite()) || !(v[0] * unit_scale).is_finite() || !(v[1] * unit_scale).is_finite() {
            return Err(UbotError::BadInput("ring vertex is not a finite number".into()));
        }
    }
    let (mut points, mut curved) = tessellate(&r.vertices, unit_scale, tol_mm)?;
    if points.len() < 3 {
        return Err(UbotError::BadInput("ring needs at least 3 distinct vertices".into()));
    }
    if points.iter().any(|p| !p.is_finite()) {
        return Err(UbotError::BadInput("ring vertex is not a finite number".into()));
    }
    let area = shoelace(&points);
    if !area.is_finite() || area.abs() <= MIN_AREA_MM2 {
        return Err(UbotError::Geometry("ring has zero area".into()));
    }
    if area < 0.0 {
        reverse_ccw(&mut points, &mut curved);
    }
    let ring = Ring { points, curved };
    check_simple(&ring)?;
    Ok(ring)
}

/// Reverses vertex order; reversed edge k is original edge n-2-k, so flags are re-aligned.
fn reverse_ccw(points: &mut [Vec2], curved: &mut [bool]) {
    let n = points.len();
    let old = curved.to_vec();
    points.reverse();
    for (k, flag) in curved.iter_mut().enumerate() {
        *flag = old.get((2 * n - 2 - k) % n).copied().unwrap_or(false);
    }
}

struct SweepEdge {
    idx: usize,
    lo: f64,
    hi: f64,
    other_lo: f64,
    other_hi: f64,
}

/// Plane sweep over edge extents, along the axis where edges are shortest so the active set stays small.
fn check_simple(ring: &Ring) -> Result<(), UbotError> {
    let n = ring.points.len();
    let mut edges: Vec<SweepEdge> = Vec::with_capacity(n);
    let (mut load_x, mut load_y) = (0.0, 0.0);
    for i in 0..n {
        let Some((a, b)) = ring.edge(i) else { continue };
        load_x += (a.x - b.x).abs();
        load_y += (a.y - b.y).abs();
    }
    let along_x = load_x <= load_y;
    for i in 0..n {
        let Some((a, b)) = ring.edge(i) else { continue };
        let (pa, pb, qa, qb) = if along_x { (a.x, b.x, a.y, b.y) } else { (a.y, b.y, a.x, b.x) };
        edges.push(SweepEdge { idx: i, lo: pa.min(pb), hi: pa.max(pb), other_lo: qa.min(qb), other_hi: qa.max(qb) });
    }
    edges.sort_by(|x, y| x.lo.total_cmp(&y.lo).then(x.idx.cmp(&y.idx)));
    let mut active: Vec<usize> = Vec::new();
    for pos in 0..edges.len() {
        let Some(cur) = edges.get(pos) else { continue };
        active.retain(|&k| edges.get(k).is_some_and(|e| e.hi >= cur.lo - POINT_TOL_MM));
        for &k in &active {
            let Some(other) = edges.get(k) else { continue };
            if other.other_lo > cur.other_hi + POINT_TOL_MM || other.other_hi < cur.other_lo - POINT_TOL_MM {
                continue;
            }
            check_pair(ring, other.idx.min(cur.idx), other.idx.max(cur.idx))?;
        }
        active.push(pos);
    }
    Ok(())
}

fn check_pair(ring: &Ring, i: usize, j: usize) -> Result<(), UbotError> {
    let n = ring.points.len();
    let (Some((a, b)), Some((c, d))) = (ring.edge(i), ring.edge(j)) else { return Ok(()) };
    let next = j == i + 1;
    if next || (i == 0 && j == n - 1) {
        if adjacent_fold_back(a, b, c, d, next) {
            return Err(UbotError::Geometry(format!("ring edge {j} folds back onto edge {i}")));
        }
    } else if segment::touch(a, b, c, d) {
        return Err(UbotError::Geometry(format!("ring edges {i} and {j} intersect")));
    }
    Ok(())
}

/// Adjacent edges may only share their common vertex; a collinear reversal overlaps instead.
fn adjacent_fold_back(a: Vec2, b: Vec2, c: Vec2, d: Vec2, next: bool) -> bool {
    // next: edges a→b and b→d share b. Otherwise the closing edge c→d ends where a→b starts.
    let (shared, far1, far2) = if next { (b, a, d) } else { (a, b, c) };
    segment::dist_to_segment(shared, far1, far2) <= POINT_TOL_MM || segment::dist_to_segment(shared, far2, far1) <= POINT_TOL_MM
}
