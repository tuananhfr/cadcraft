//! Solid fills as boundary paths, for plots ([`crate::DisplayList::fill_paths`]).
//!
//! A hatch's triangles fill its loops with odd parity. A writer that fills paths (PDF) can fill
//! the loops themselves: far fewer bytes for a dense boundary, and no seams between triangles.
//! The loops are oriented by nesting depth (even depth counter-clockwise, odd clockwise) so the
//! nonzero rule leaves the same islands empty. Even-odd would match the triangles' parity
//! exactly but punches holes where a traced signature's stroke crosses itself; AutoCAD and the
//! nonzero rule keep those filled.

use cadcraft_geom::{Bounds2, Vec2, point_in_polygon};

use crate::clip;

/// The boundary of the filled primitive [`FillPath::prim`] as closed loops (last point not
/// repeated), oriented for the nonzero fill rule.
#[derive(Clone, Debug, PartialEq)]
pub struct FillPath {
    pub prim: u32,
    pub loops: Vec<Vec<Vec2>>,
}

/// Points of a loop tested against the others to tell nesting from crossing.
const PROBES: usize = 8;

/// Upper bound on the nesting work (bounding-box comparisons plus probe tests × vertices); past
/// it the fill keeps only its triangles.
const MAX_NESTING_WORK: usize = 20_000_000;

/// `loops` oriented for the nonzero rule by nesting depth; `None` when they are too many to
/// compare. A loop counts as inside another only when its box and all its probes are, so loops
/// that merely cross keep the same orientation and their overlap stays filled.
pub(crate) fn oriented(loops: &[Vec<Vec2>]) -> Option<Vec<Vec<Vec2>>> {
    let loops: Vec<&Vec<Vec2>> = loops.iter().filter(|l| l.len() >= 3 && l.iter().all(|p| p.is_finite())).collect();
    let n = loops.len();
    if n == 0 || n.saturating_mul(n) > MAX_NESTING_WORK {
        return None;
    }
    let shapes: Vec<(&Vec<Vec2>, Bounds2, Vec<Vec2>)> = loops.iter().map(|l| (*l, Bounds2::from_points(l.iter().copied()), probes(l))).collect();
    let mut work = n.saturating_mul(n);
    let mut out = Vec::with_capacity(n);
    for (i, (l, bounds, probes)) in shapes.iter().enumerate() {
        let mut depth = 0usize;
        for (j, (other, other_bounds, _)) in shapes.iter().enumerate() {
            if i == j || probes.is_empty() || !encloses(other_bounds, bounds) {
                continue;
            }
            work = work.saturating_add(probes.len().saturating_mul(other.len()));
            if work > MAX_NESTING_WORK {
                return None;
            }
            if probes.iter().all(|p| point_in_polygon(other, *p)) {
                depth += 1;
            }
        }
        let ccw = signed_area(l) >= 0.0;
        let mut l = (*l).clone();
        if ccw != depth.is_multiple_of(2) {
            l.reverse();
        }
        out.push(l);
    }
    Some(out)
}

/// `loops` clipped to `rect`, each on its own (Sutherland–Hodgman). Every clip step swaps the
/// part of a loop beyond an edge for a run along that edge, which keeps the winding number of
/// every point inside, so the nonzero fill inside `rect` is unchanged; loops left with no area
/// are dropped.
pub(crate) fn clipped(loops: &[Vec<Vec2>], rect: &Bounds2) -> Vec<Vec<Vec2>> {
    loops
        .iter()
        .map(|l| if l.iter().all(|p| rect.contains(*p)) { l.clone() } else { clip::clip_polygon(l, rect) })
        .filter(|l| l.len() >= 3 && signed_area(l) != 0.0)
        .collect()
}

fn encloses(outer: &Bounds2, inner: &Bounds2) -> bool {
    outer.min.x <= inner.min.x && outer.min.y <= inner.min.y && outer.max.x >= inner.max.x && outer.max.y >= inner.max.y
}

/// Edge midpoints spread along the loop (vertices may touch an enclosing loop; midpoints rarely
/// do).
fn probes(l: &[Vec2]) -> Vec<Vec2> {
    let n = l.len();
    let step = n.div_ceil(PROBES).max(1);
    (0..n)
        .step_by(step)
        .filter_map(|i| {
            let (a, b) = (*l.get(i)?, *l.get((i + 1) % n)?);
            (a.dist(b) > 1e-12).then(|| (a + b) * 0.5).filter(|m| m.is_finite())
        })
        .collect()
}

/// Twice the signed area (positive when counter-clockwise).
fn signed_area(l: &[Vec2]) -> f64 {
    l.iter().zip(l.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(lo: f64, hi: f64) -> Vec<Vec2> {
        vec![Vec2::new(lo, lo), Vec2::new(hi, lo), Vec2::new(hi, hi), Vec2::new(lo, hi)]
    }

    #[test]
    fn nesting_alternates_orientation() {
        let mut island = square(2.0, 8.0);
        island.reverse();
        // Whichever way a loop comes in, its depth decides: 0, 1, 2 and 0.
        for island in [square(2.0, 8.0), island] {
            let out = oriented(&[square(0.0, 10.0), island, square(4.0, 6.0), square(20.0, 30.0)]).unwrap();
            let signs: Vec<bool> = out.iter().map(|l| signed_area(l) > 0.0).collect();
            assert_eq!(signs, [true, false, true, true]);
        }
    }

    #[test]
    fn crossing_loops_keep_one_orientation() {
        let a = square(0.0, 10.0);
        let b = square(5.0, 15.0);
        let out = oriented(&[a, b]).unwrap();
        assert!(out.iter().all(|l| signed_area(l) > 0.0));
    }

    #[test]
    fn clipping_keeps_islands_and_drops_outside_loops() {
        let loops = oriented(&[square(0.0, 10.0), square(2.0, 8.0), square(20.0, 30.0)]).unwrap();
        let out = clipped(&loops, &Bounds2::new(Vec2::new(5.0, -5.0), Vec2::new(15.0, 15.0)));
        assert_eq!(out.len(), 2);
        assert!(signed_area(&out[0]) > 0.0 && signed_area(&out[1]) < 0.0);
        assert!(out.iter().flatten().all(|p| p.x >= 5.0 - 1e-9));
    }

    #[test]
    fn hostile_loops() {
        assert!(oriented(&[]).is_none());
        assert!(oriented(&[vec![Vec2::ZERO, Vec2::X]]).is_none());
        assert!(oriented(&[vec![Vec2::new(f64::NAN, 0.0), Vec2::X, Vec2::Y]]).is_none());
        let many: Vec<Vec<Vec2>> = (0..5000).map(|i| square(f64::from(i), f64::from(i) + 0.5)).collect();
        assert!(oriented(&many).is_none(), "too many loops to compare");
    }
}
