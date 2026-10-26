//! Path marks for content streams: coordinates on a 0.01 pt grid, polylines that continue one
//! another joined into one subpath, and points within a tolerance of a straight run dropped.

use std::fmt::Write as _;

use cadcraft_geom::Vec2;

/// Grid steps per point (two decimals).
const GRID: f64 = 100.0;

/// Points dropped in a row at most, so simplifying stays linear on long straight runs.
const MAX_DROPPED_RUN: usize = 128;

/// A coordinate in grid steps, clamped like other content numbers.
fn grid(v: f64) -> i64 {
    let v = if v.is_finite() { v.clamp(-1.0e7, 1.0e7) } else { 0.0 };
    (v * GRID).round() as i64
}

fn grid_point(p: Vec2) -> (i64, i64) {
    (grid(p.x), grid(p.y))
}

fn write_grid(out: &mut String, g: i64) {
    let sign = if g < 0 { "-" } else { "" };
    let (int, frac) = (g.unsigned_abs() / 100, g.unsigned_abs() % 100);
    let _ = match frac {
        0 => write!(out, "{sign}{int}"),
        f if f % 10 == 0 => write!(out, "{sign}{int}.{}", f / 10),
        f => write!(out, "{sign}{int}.{f:02}"),
    };
}

/// A point (in points) with two decimals.
pub(super) fn point(out: &mut String, p: Vec2) {
    let (x, y) = grid_point(p);
    write_grid(out, x);
    out.push(' ');
    write_grid(out, y);
}

/// `pts` without the points that lie within `tol` of the straight run that replaces them; the
/// ends and every turn larger than `tol` stay.
fn simplify(pts: &[Vec2], tol: f64) -> Vec<Vec2> {
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return Vec::new() };
    if pts.len() < 3 || !(tol.is_finite() && tol > 0.0) {
        return pts.to_vec();
    }
    let mut out = vec![*first];
    let mut dropped_from = 1;
    for (i, w) in pts.windows(2).enumerate().skip(1) {
        let (Some(here), Some(next), Some(anchor)) = (w.first(), w.get(1), out.last().copied()) else { continue };
        let run = pts.get(dropped_from..=i).unwrap_or(&[]);
        let keep = i - dropped_from >= MAX_DROPPED_RUN || run.iter().any(|q| segment_distance(*q, anchor, *next) > tol);
        if keep {
            out.push(*here);
            dropped_from = i + 1;
        }
    }
    out.push(*last);
    out
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let d = b - a;
    let len2 = d.dot(d);
    let t = if len2 > 0.0 { ((p - a).dot(d) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.dist(a + d * t)
}

/// `pts` simplified within `tol` and snapped to the grid, without repeated grid points.
fn on_grid(pts: &[Vec2], tol: f64) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(pts.len());
    for g in simplify(pts, tol).into_iter().map(grid_point) {
        if out.last() != Some(&g) {
            out.push(g);
        }
    }
    out
}

fn subpath(out: &mut String, pts: &[(i64, i64)]) {
    for (i, (x, y)) in pts.iter().enumerate() {
        write_grid(out, *x);
        out.push(' ');
        write_grid(out, *y);
        out.push_str(if i == 0 { " m\n" } else { " l\n" });
    }
}

/// The stroked path being written: polylines added with the same stroke join into one subpath
/// where one starts at the grid point the last ended, which draws the same with round caps and
/// joins.
pub(super) struct Strokes {
    /// The subpath being collected (points, not yet written).
    sub: Vec<Vec2>,
    /// Subpaths were written since the last `S`.
    pending: bool,
    /// Simplification tolerance in points.
    tol: f64,
}

impl Strokes {
    pub(super) fn new(tol: f64) -> Self {
        Strokes { sub: Vec::new(), pending: false, tol }
    }

    /// The simplification tolerance (points).
    pub(super) fn tolerance(&self) -> f64 {
        self.tol
    }

    /// Add a polyline (two points or more) to the path.
    pub(super) fn add(&mut self, out: &mut String, pts: &[Vec2]) {
        let Some(first) = pts.first() else { return };
        match self.sub.last() {
            Some(last) if grid_point(*last) == grid_point(*first) => self.sub.extend_from_slice(pts.get(1..).unwrap_or(&[])),
            _ => {
                self.write_sub(out);
                self.sub.extend_from_slice(pts);
            }
        }
    }

    fn write_sub(&mut self, out: &mut String) {
        if self.sub.is_empty() {
            return;
        }
        let mut pts = on_grid(&self.sub, self.tol);
        // A polyline shorter than the grid still prints as a dot with round caps.
        if let [p] = pts[..] {
            pts.push(p);
        }
        subpath(out, &pts);
        self.sub.clear();
        self.pending = true;
    }

    /// Stroke what has been added (`S`).
    pub(super) fn flush(&mut self, out: &mut String) {
        self.write_sub(out);
        if self.pending {
            out.push_str("S\n");
            self.pending = false;
        }
    }
}

/// Closed subpaths (`m … l h`) of `loops` (in points), simplified within `tol`; loops left with
/// fewer than three points are skipped. Returns whether any was written.
pub(super) fn loops<'a>(out: &mut String, loops: impl IntoIterator<Item = &'a [Vec2]>, map: impl Fn(Vec2) -> Vec2, tol: f64) -> bool {
    let mut any = false;
    for l in loops {
        let pts: Vec<Vec2> = l.iter().map(|p| map(*p)).collect();
        let mut g = on_grid(&pts, tol);
        if g.len() > 1 && g.first() == g.last() {
            g.pop();
        }
        if g.len() < 3 {
            continue;
        }
        subpath(out, &g);
        out.push_str("h\n");
        any = true;
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(f: impl FnOnce(&mut String)) -> String {
        let mut s = String::new();
        f(&mut s);
        s
    }

    #[test]
    fn two_decimals_without_trailing_zeros() {
        assert_eq!(text(|s| point(s, Vec2::new(12.345_678, -0.004))), "12.35 0");
        assert_eq!(text(|s| point(s, Vec2::new(-3.1, 2.0))), "-3.1 2");
        assert_eq!(text(|s| point(s, Vec2::new(-0.05, 1e300))), "-0.05 10000000");
        assert_eq!(text(|s| point(s, Vec2::new(f64::NAN, 0.999))), "0 1");
    }

    #[test]
    fn simplify_keeps_corners_and_reversals() {
        let pts = [Vec2::ZERO, Vec2::new(1.0, 0.001), Vec2::new(2.0, 0.0), Vec2::new(2.0, 1.0)];
        assert_eq!(simplify(&pts, 0.01), [Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(2.0, 1.0)]);
        // A spike back along the line is collinear but not on the run: kept.
        let spike = [Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(1.0, 0.0)];
        assert_eq!(simplify(&spike, 0.01), spike);
    }

    #[test]
    fn long_straight_runs_stay_bounded() {
        let pts: Vec<Vec2> = (0..=10_000).map(|i| Vec2::new(f64::from(i), 0.0)).collect();
        let kept = simplify(&pts, 0.01);
        assert!(kept.len() <= 2 + 10_000 / MAX_DROPPED_RUN, "{}", kept.len());
    }

    #[test]
    fn continuing_polylines_share_a_subpath() {
        let mut s = Strokes::new(0.01);
        let out = text(|c| {
            s.add(c, &[Vec2::ZERO, Vec2::new(10.0, 0.0)]);
            s.add(c, &[Vec2::new(10.001, 0.0), Vec2::new(10.0, 10.0)]);
            s.add(c, &[Vec2::new(20.0, 0.0), Vec2::new(20.0, 0.001)]);
            s.flush(c);
        });
        assert_eq!(out, "0 0 m\n10 0 l\n10 10 l\n20 0 m\n20 0 l\nS\n");
        assert_eq!(text(|c| s.flush(c)), "", "nothing pending after a flush");
    }

    #[test]
    fn loops_are_closed_and_degenerate_ones_skipped() {
        let square = [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0), Vec2::ZERO];
        let sliver = [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0001)];
        let mut out = String::new();
        assert!(loops(&mut out, [&square[..], &sliver[..]], |p| p * 10.0, 0.01));
        assert_eq!(out, "0 0 m\n10 0 l\n10 10 l\n0 10 l\nh\n");
        assert!(!loops(&mut String::new(), [&sliver[..]], |p| p, 0.01));
    }
}
