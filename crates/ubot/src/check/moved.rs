//! Detects boxes left off their grid cell, with the same tolerances as the fill diff.

use cadcraft_geom::Vec2;

use super::resolve::Placed;
use super::{Issue, IssueCode, LayoutInput, Severity, issue};
use crate::error::UbotError;
use crate::fill::{MOVE_TOL_MM, TURN_TOL_DEG, angle_gap_deg, parse_cell};

/// `Info(ManualMoved)` when the box is further than the fill tolerance from where its stored layout puts cell `<i>:<j>`.
pub(super) fn moved(p: &Placed<'_>, layout: &LayoutInput) -> Result<Option<Issue>, UbotError> {
    let (Some(rec), Some(_), Some(cell)) = (&layout.layout, &p.geom, p.inst.base.instance_id.as_deref().and_then(parse_cell)) else {
        return Ok(None);
    };
    let finite = [rec.origin[0], rec.origin[1], rec.angle_deg, rec.pitch_x_mm, rec.pitch_y_mm, rec.unit_scale];
    if !(finite.iter().all(|v| v.is_finite()) && rec.unit_scale > 0.0) {
        return Err(UbotError::BadInput(format!("layout {} record has a non-finite or non-positive value", layout.layout_id)));
    }
    let a = rec.angle_deg.to_radians();
    let (u, v) = (Vec2::new(a.cos(), a.sin()), Vec2::new(-a.sin(), a.cos()));
    // Row j sits j pitches below the origin, column i i pitches to the right, in the grid frame.
    let expected =
        Vec2::new(rec.origin[0], rec.origin[1]) * rec.unit_scale + u * (cell[0] as f64 * rec.pitch_x_mm) - v * (cell[1] as f64 * rec.pitch_y_mm);
    if !expected.is_finite() {
        return Err(UbotError::BadInput(format!("layout {} record places cell {}:{} out of range", layout.layout_id, cell[0], cell[1])));
    }
    let off = p.center_mm.dist(expected) > MOVE_TOL_MM || angle_gap_deg(p.angle_deg, rec.angle_deg) >= TURN_TOL_DEG;
    Ok(off.then(|| issue(IssueCode::ManualMoved, Severity::Info, p, "box was moved by hand from its grid cell".into())))
}
