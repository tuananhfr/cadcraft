use cadcraft_geom::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Disjoint,
    Duplicate,
    AInsideB,
    BInsideA,
    Overlap,
}

pub fn rect_of(points: &[Vec2]) -> Option<[Vec2; 4]> {
    let corners = match points {
        [a, b, c, d] => [*a, *b, *c, *d],
        [a, b, c, d, end] if a == end => [*a, *b, *c, *d],
        _ => return None,
    };
    let [a, b, c, d] = corners;
    if !corners.into_iter().all(Vec2::is_finite) {
        return None;
    }
    let edges = [b - a, c - b, d - c, a - d];
    if !edges.into_iter().all(|v| v.len().is_finite() && v.len() > 0.0) {
        return None;
    }
    let [ab, bc, cd, da] = edges.map(|v| v / v.len());
    if [ab.dot(bc), bc.dot(cd), cd.dot(da), da.dot(ab)].into_iter().any(|cos| cos.abs() > 1e-6)
        || ab.cross(bc) * cd.cross(da) <= 0.0
        || ab.cross(bc) * bc.cross(cd) <= 0.0
    {
        return None;
    }
    let area = (b - a).len() * (c - b).len();
    (area.is_finite() && area > 0.0).then_some(corners)
}

pub fn normalize_outline(corners: [Vec2; 4], up: Vec2) -> ([[f64; 2]; 4], f64) {
    let hint = if up.is_finite() && up.len().is_finite() && up.len() > 0.0 { up / up.len() } else { Vec2::Y };
    let [a, b, c, d] = corners;
    let y =
        [b - a, c - b, d - c, a - d].into_iter().map(|edge| edge / edge.len()).max_by(|a, b| a.dot(hint).total_cmp(&b.dot(hint))).unwrap_or(Vec2::Y);
    let x = Vec2::new(y.y, -y.x);
    let ccw = if ((b - a) / (b - a).len()).cross((c - b) / (c - b).len()) < 0.0 { [a, d, c, b] } else { corners };
    let origin = a;
    let start =
        ccw.iter().enumerate().min_by(|(_, p), (_, q)| ((**p - origin).dot(x + y)).total_cmp(&((**q - origin).dot(x + y)))).map_or(0, |(i, _)| i);
    let mut ordered = ccw;
    ordered.rotate_left(start);
    let mut rotation = (-y.x).atan2(y.y).to_degrees();
    let snapped = (rotation / 90.0).round() * 90.0;
    if (rotation - snapped).abs() < 1e-9 {
        rotation = snapped;
    }
    (ordered.map(|p| [p.x, p.y]), rotation)
}

pub(super) fn points(outline: &[[f64; 2]; 4]) -> [Vec2; 4] {
    outline.map(|[x, y]| Vec2::new(x, y))
}

pub(super) fn area(outline: &[[f64; 2]; 4]) -> f64 {
    let [a, b, c, _] = points(outline);
    (b - a).cross(c - b).abs()
}

pub fn relation(a: &[[f64; 2]; 4], b: &[[f64; 2]; 4]) -> Relation {
    let ap = points(a);
    let bp = points(b);
    if rect_of(&ap).is_none() || rect_of(&bp).is_none() {
        return Relation::Disjoint;
    }
    let [a0, a1, a2, a3] = ap;
    let [b0, b1, b2, b3] = bp;
    let longest = [(a1 - a0).len(), (a2 - a1).len(), (b1 - b0).len(), (b2 - b1).len()].into_iter().fold(0.0, f64::max);
    let tolerance = longest * 1e-6;
    if ap.iter().all(|p| bp.iter().any(|q| p.dist(*q) <= tolerance)) {
        return Relation::Duplicate;
    }
    let mut a_inside = true;
    let mut b_inside = true;
    for edge in [a1 - a0, a2 - a1, a3 - a2, a0 - a3, b1 - b0, b2 - b1, b3 - b2, b0 - b3] {
        let axis = edge.perp() / edge.len();
        let project =
            |ps: [Vec2; 4]| ps.into_iter().map(|p| (p - a0).dot(axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
        let (alo, ahi) = project(ap);
        let (blo, bhi) = project(bp);
        if ahi <= blo || bhi <= alo {
            return Relation::Disjoint;
        }
        a_inside &= alo >= blo - tolerance && ahi <= bhi + tolerance;
        b_inside &= blo >= alo - tolerance && bhi <= ahi + tolerance;
    }
    if a_inside {
        Relation::AInsideB
    } else if b_inside {
        Relation::BInsideA
    } else {
        Relation::Overlap
    }
}
