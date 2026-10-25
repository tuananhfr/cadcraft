//! Turns each existing box into a centre, footprint angle and (when its size is known) footprint, in mm.

use std::collections::HashMap;

use cadcraft_geom::{Bounds2, Vec2};

use super::CheckInstance;
use crate::catalog::Catalog;
use crate::error::UbotError;
use crate::geometry::{Footprint, footprint};
use crate::model::{BlockMapping, Product};
use crate::placement::{center_from_insert, relative_scale};

pub(super) struct Geom {
    pub fp: Footprint,
    pub bounds: Bounds2,
}

pub(super) struct Placed<'a> {
    pub inst: &'a CheckInstance,
    pub center_mm: Vec2,
    /// INSERT rotation minus the block's rotation offset.
    pub angle_deg: f64,
    /// Footprint centre in drawing units (the INSERT point when the size is unknown).
    pub at: [f64; 2],
    pub unknown_product: bool,
    pub geom: Option<Geom>,
}

/// Catalog lookups cached per name; the catalog lists are scanned linearly.
pub(super) struct Lookup<'c> {
    catalog: &'c Catalog,
    blocks: HashMap<String, Option<&'c BlockMapping>>,
    products: HashMap<String, Option<&'c Product>>,
}

impl<'c> Lookup<'c> {
    pub fn new(catalog: &'c Catalog) -> Self {
        Lookup { catalog, blocks: HashMap::new(), products: HashMap::new() }
    }

    pub fn mapping(&mut self, block: &str) -> Option<&'c BlockMapping> {
        let catalog = self.catalog;
        *self.blocks.entry(block.to_ascii_lowercase()).or_insert_with(|| catalog.mapping_for_block(block))
    }

    pub fn product(&mut self, id: &str) -> Option<&'c Product> {
        let catalog = self.catalog;
        *self.products.entry(id.to_string()).or_insert_with(|| catalog.product(id))
    }
}

fn bad(inst: &CheckInstance, what: &str) -> UbotError {
    UbotError::BadInput(format!("instance {}: {what}", inst.base.handle))
}

/// `layout_product` sizes boxes that carry no product of their own (legacy blocks).
pub(super) fn resolve<'a>(
    inst: &'a CheckInstance,
    layout_product: Option<&Product>,
    lookup: &mut Lookup<'_>,
    unit_scale: f64,
) -> Result<Placed<'a>, UbotError> {
    let b = &inst.base;
    if !(b.insert.iter().all(|c| c.is_finite()) && b.rotation_deg.is_finite()) {
        return Err(bad(inst, "insert and rotation must be finite"));
    }
    if !(b.scale.is_finite() && b.scale != 0.0) {
        return Err(bad(inst, "scale must be finite and non-zero"));
    }
    let mapping = lookup.mapping(&b.block_name);
    let (size, unknown_product) = match b.product_id.as_deref() {
        Some(id) => match lookup.product(id) {
            Some(p) => (Some(Vec2::new(p.width_mm, p.length_mm)), false),
            None => (None, true),
        },
        None => {
            // A legacy block still has a size when the catalog maps it to a product; failing that, the layout's product.
            let mapped = mapping.and_then(|m| m.product_id.as_deref()).and_then(|id| lookup.product(id));
            (mapped.or(layout_product).map(|p| Vec2::new(p.width_mm, p.length_mm)), false)
        }
    };
    let insert_mm = Vec2::new(b.insert[0], b.insert[1]) * unit_scale;
    let center_mm = center_from_insert(mapping, size.map_or(0.0, |s| s.y), insert_mm, b.rotation_deg, relative_scale(mapping, b.scale, unit_scale));
    let angle_deg = b.rotation_deg - mapping.map_or(0.0, |m| m.rotation_offset_deg);
    if !(center_mm.is_finite() && angle_deg.is_finite()) {
        return Err(bad(inst, "position overflows in mm"));
    }
    let geom = match size {
        Some(s) => {
            let fp = footprint(center_mm, s.x, s.y, angle_deg);
            if !fp.corners.iter().all(|c| c.is_finite()) {
                return Err(bad(inst, "footprint overflows in mm"));
            }
            Some(Geom { bounds: Bounds2::from_points(fp.corners), fp })
        }
        None => None,
    };
    let at = if geom.is_some() { [center_mm.x / unit_scale, center_mm.y / unit_scale] } else { b.insert };
    Ok(Placed { inst, center_mm, angle_deg, at, unknown_product, geom })
}
