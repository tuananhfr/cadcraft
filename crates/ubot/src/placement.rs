//! Block anchor maths shared by fill and replace: footprint centre ↔ INSERT point, both in mm.

use cadcraft_geom::Vec2;

use crate::model::{Anchor, BlockMapping};

/// Insertion point minus footprint centre in mm, in the unrotated block frame at true size.
fn anchor_offset_mm(mapping: Option<&BlockMapping>, length_mm: f64) -> Vec2 {
    match mapping.map(|m| m.anchor) {
        None | Some(Anchor::Center) => Vec2::ZERO,
        Some(Anchor::TopCenter) => Vec2::new(0.0, length_mm / 2.0),
        Some(Anchor::Offset { x_mm, y_mm }) => Vec2::new(x_mm, y_mm),
    }
}

/// Size of an INSERT relative to the true box size: `insert_scale` × mm per drawing unit ÷ mm per block unit.
pub fn relative_scale(mapping: Option<&BlockMapping>, insert_scale: f64, unit_scale: f64) -> f64 {
    let mm_per_block_unit = mapping.map_or(1.0, |m| m.units_mm_per_block_unit);
    insert_scale * unit_scale / mm_per_block_unit
}

/// INSERT point (mm) of a block whose footprint centre is `center_mm`; `rotation_deg` is the INSERT rotation and
/// `scale` the [`relative_scale`] (1 for a true-size box).
pub fn insert_from_center(mapping: Option<&BlockMapping>, length_mm: f64, center_mm: Vec2, rotation_deg: f64, scale: f64) -> Vec2 {
    insert_from_center_xy(mapping, length_mm, center_mm, rotation_deg, [scale, scale])
}

/// [`insert_from_center`] with signed per-axis relative scales `[x, y]`, so a mirrored box (one negative scale) flips
/// its anchor offset with it.
pub fn insert_from_center_xy(mapping: Option<&BlockMapping>, length_mm: f64, center_mm: Vec2, rotation_deg: f64, scale: [f64; 2]) -> Vec2 {
    center_mm + scaled_offset(mapping, length_mm, rotation_deg, scale)
}

/// Inverse of [`insert_from_center_xy`].
pub fn center_from_insert_xy(mapping: Option<&BlockMapping>, length_mm: f64, insert_mm: Vec2, rotation_deg: f64, scale: [f64; 2]) -> Vec2 {
    insert_mm - scaled_offset(mapping, length_mm, rotation_deg, scale)
}

/// The anchor offset in the drawing frame. Equal scales take the scalar form so its results stay bit for bit.
fn scaled_offset(mapping: Option<&BlockMapping>, length_mm: f64, rotation_deg: f64, [sx, sy]: [f64; 2]) -> Vec2 {
    let o = anchor_offset_mm(mapping, length_mm);
    if sx == sy {
        return o.rotate(rotation_deg.to_radians()) * sx;
    }
    Vec2::new(o.x * sx, o.y * sy).rotate(rotation_deg.to_radians())
}

/// Whether the block's insertion point is away from the footprint centre.
pub fn has_anchor_offset(mapping: Option<&BlockMapping>) -> bool {
    match mapping.map(|m| m.anchor) {
        None | Some(Anchor::Center) => false,
        Some(Anchor::TopCenter) => true,
        Some(Anchor::Offset { x_mm, y_mm }) => x_mm != 0.0 || y_mm != 0.0,
    }
}

/// Inverse of [`insert_from_center`].
pub fn center_from_insert(mapping: Option<&BlockMapping>, length_mm: f64, insert_mm: Vec2, rotation_deg: f64, scale: f64) -> Vec2 {
    center_from_insert_xy(mapping, length_mm, insert_mm, rotation_deg, [scale, scale])
}
