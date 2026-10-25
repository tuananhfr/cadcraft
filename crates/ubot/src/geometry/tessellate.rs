use cadcraft_geom::{Vec2, bulge_to_arc};

use super::{MAX_RING_VERTICES, POINT_TOL_MM};
use crate::UbotError;

/// Expands bulge segments into chords within `tol_mm` and removes consecutive duplicates.
/// Returns points plus per-edge flags (edge i starts at point i) marking arc-derived edges.
pub(crate) fn tessellate(vertices: &[[f64; 3]], unit_scale: f64, tol_mm: f64) -> Result<(Vec<Vec2>, Vec<bool>), UbotError> {
    let n = vertices.len();
    let mut points: Vec<Vec2> = Vec::with_capacity(n);
    let mut curved: Vec<bool> = Vec::with_capacity(n);
    let mut scratch: Vec<Vec2> = Vec::new();
    for (i, v) in vertices.iter().enumerate() {
        let a = Vec2::new(v[0] * unit_scale, v[1] * unit_scale);
        let next = vertices.get((i + 1) % n).map(|w| Vec2::new(w[0] * unit_scale, w[1] * unit_scale)).unwrap_or(a);
        push_point(&mut points, &mut curved, a, false);
        let arc = if v[2] == 0.0 { None } else { bulge_to_arc(a, next, v[2]) };
        let Some((arc, ccw)) = arc else { continue };
        scratch.clear();
        arc.tessellate(tol_mm, &mut scratch);
        if !ccw {
            scratch.reverse();
        }
        // Interior chord vertices only; the end vertex comes from the next input vertex.
        let inner = scratch.get(1..scratch.len().saturating_sub(1)).unwrap_or(&[]);
        if let Some(last) = curved.last_mut() {
            *last = true;
        }
        for p in inner {
            if !p.is_finite() {
                return Err(UbotError::BadInput("arc segment is not finite".into()));
            }
            push_point(&mut points, &mut curved, *p, true);
        }
        if points.len() > MAX_RING_VERTICES {
            return Err(UbotError::Limit(format!("ring exceeds {MAX_RING_VERTICES} vertices after arc subdivision")));
        }
    }
    close_ring(&mut points, &mut curved);
    Ok((points, curved))
}

/// Appends a point unless it repeats the previous one; the dropped point's outgoing edge flag wins.
fn push_point(points: &mut Vec<Vec2>, curved: &mut Vec<bool>, p: Vec2, flag: bool) {
    if let Some(prev) = points.last()
        && prev.dist(p) < POINT_TOL_MM
    {
        if let Some(last) = curved.last_mut() {
            *last = flag;
        }
        return;
    }
    points.push(p);
    curved.push(flag);
}

fn close_ring(points: &mut Vec<Vec2>, curved: &mut Vec<bool>) {
    while points.len() > 1 {
        let (Some(first), Some(last)) = (points.first(), points.last()) else { break };
        if first.dist(*last) >= POINT_TOL_MM {
            break;
        }
        points.pop();
        curved.pop();
    }
}
