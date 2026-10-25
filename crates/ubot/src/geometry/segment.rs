use cadcraft_geom::{Line, Vec2};

use super::POINT_TOL_MM;

/// Side of `p` relative to a→b: -1, 0 (within tolerance) or 1 (left).
fn side(a: Vec2, b: Vec2, p: Vec2) -> i8 {
    let len = a.dist(b);
    if len <= POINT_TOL_MM {
        return 0;
    }
    let d = (b - a).cross(p - a) / len;
    if d > POINT_TOL_MM {
        1
    } else if d < -POINT_TOL_MM {
        -1
    } else {
        0
    }
}

fn on_segment(a: Vec2, b: Vec2, p: Vec2) -> bool {
    let t = POINT_TOL_MM;
    p.x >= a.x.min(b.x) - t && p.x <= a.x.max(b.x) + t && p.y >= a.y.min(b.y) - t && p.y <= a.y.max(b.y) + t
}

/// True when the segments cross at a point interior to both.
pub(crate) fn cross_properly(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    side(a, b, c) * side(a, b, d) < 0 && side(c, d, a) * side(c, d, b) < 0
}

/// True on any contact: crossing, endpoint touch or collinear overlap.
pub(crate) fn touch(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    if cross_properly(a, b, c, d) {
        return true;
    }
    (side(a, b, c) == 0 && on_segment(a, b, c))
        || (side(a, b, d) == 0 && on_segment(a, b, d))
        || (side(c, d, a) == 0 && on_segment(c, d, a))
        || (side(c, d, b) == 0 && on_segment(c, d, b))
}

pub(crate) fn distance(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> f64 {
    if touch(a, b, c, d) {
        return 0.0;
    }
    let ab = Line::new(a, b);
    let cd = Line::new(c, d);
    ab.dist(c).min(ab.dist(d)).min(cd.dist(a)).min(cd.dist(b))
}

pub(crate) fn dist_to_segment(a: Vec2, b: Vec2, p: Vec2) -> f64 {
    Line::new(a, b).dist(p)
}
