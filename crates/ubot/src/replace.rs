//! Product swap planning (TUBAB): same centre and orientation, new product block and anchor.

use std::collections::HashSet;

use cadcraft_geom::Vec2;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::check::{Issue, IssueCode, Severity};
use crate::error::UbotError;
use crate::fill::ExistingInstance;
use crate::model::Product;
use crate::placement::{center_from_insert_xy, has_anchor_offset, insert_from_center_xy, relative_scale};

const MAX_TARGETS: usize = 100_000;
const SIZE_TOL_MM: f64 = 1e-6;
/// Relative scales closer than this are one uniform scale.
const SIZE_TOL_REL: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceRequest {
    /// Millimetres per drawing unit.
    pub unit_scale: f64,
    pub targets: Vec<ExistingInstance>,
    pub to_product_id: String,
    /// Allow a product whose footprint differs; the caller re-checks overlaps afterwards.
    #[serde(default)]
    pub allow_resize: bool,
}

/// New block, insert point (drawing units) and rotation for one target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceChange {
    pub handle: String,
    pub product_id: String,
    pub block_name: String,
    pub insert: [f64; 2],
    pub rotation_deg: f64,
    /// Uniform INSERT scale that draws the block at true size: mm per block unit over mm per drawing unit.
    pub scale: f64,
    /// The INSERT X and Y scales to write: `scale` with the target's mirror signs.
    #[serde(default)]
    pub scale_xy: [f64; 2],
    /// The drawn footprint changes size (another product, or a box that was not drawn at true size).
    #[serde(default)]
    pub resized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceResult {
    pub changes: Vec<ReplaceChange>,
    pub refused: Vec<Issue>,
    /// Targets replaced on the assumption that they are the size of the new product (legacy blocks).
    #[serde(default)]
    pub warnings: Vec<Issue>,
}

fn same_size(a: &Product, b: &Product) -> bool {
    (a.width_mm - b.width_mm).abs() <= SIZE_TOL_MM && (a.length_mm - b.length_mm).abs() <= SIZE_TOL_MM
}

pub fn replace(req: &ReplaceRequest, catalog: &Catalog) -> Result<ReplaceResult, UbotError> {
    if !(req.unit_scale.is_finite() && req.unit_scale > 0.0) {
        return Err(UbotError::BadInput("unitScale must be a finite number > 0".into()));
    }
    if req.targets.len() > MAX_TARGETS {
        return Err(UbotError::Limit(format!("{} targets, the limit is {MAX_TARGETS}", req.targets.len())));
    }
    let to = catalog.product(&req.to_product_id).ok_or_else(|| UbotError::UnknownProduct(req.to_product_id.clone()))?;
    let new_mapping = catalog.mapping_for_product(&to.id);
    let new_block = new_mapping.map_or_else(|| Catalog::default_block_name(to), |m| m.block_name.clone());
    let new_scale = new_mapping.map_or(1.0, |m| m.units_mm_per_block_unit) / req.unit_scale;
    if !new_scale.is_finite() {
        return Err(UbotError::BadInput("unitScale makes the block scale overflow".into()));
    }
    let mut seen = HashSet::new();
    let mut out = ReplaceResult { changes: vec![], refused: vec![], warnings: vec![] };
    for t in &req.targets {
        if !seen.insert(t.handle.as_str()) {
            return Err(UbotError::BadInput(format!("target {} is listed twice", t.handle)));
        }
        if !(t.insert.iter().all(|c| c.is_finite()) && t.rotation_deg.is_finite()) {
            return Err(UbotError::BadInput(format!("target {}: insert and rotation must be finite", t.handle)));
        }
        let scale_y = t.scale_y.unwrap_or(t.scale);
        if !(t.scale.is_finite() && t.scale != 0.0 && scale_y.is_finite() && scale_y != 0.0) {
            return Err(UbotError::BadInput(format!("target {}: scale must be finite and non-zero", t.handle)));
        }
        let issue = |code, severity, message: String| Issue { code, severity, handle: Some(t.handle.clone()), at: t.insert, message };
        let from = match t.product_id.as_deref() {
            Some(id) => match catalog.product(id) {
                Some(p) => p,
                None => {
                    out.refused.push(issue(IssueCode::UnknownProduct, Severity::Error, format!("product {id} is not in the catalog")));
                    continue;
                }
            },
            None => match catalog.mapping_for_block(&t.block_name).and_then(|m| m.product_id.as_deref()).and_then(|id| catalog.product(id)) {
                Some(p) => p,
                None => {
                    out.warnings.push(issue(
                        IssueCode::LegacyBlock,
                        Severity::Warning,
                        format!("block {} has no product, assumed to be the size of {}", t.block_name, to.id),
                    ));
                    to
                }
            },
        };
        if !req.allow_resize && !same_size(from, to) {
            out.refused.push(issue(IssueCode::Clearance, Severity::Error, format!("size differs: {} to {}", from.id, to.id)));
            continue;
        }
        let old_mapping = catalog.mapping_for_block(&t.block_name);
        let rel = [relative_scale(old_mapping, t.scale, req.unit_scale), relative_scale(old_mapping, scale_y, req.unit_scale)];
        // A stretched box has no single length along which its anchor offset lies.
        if (rel[0].abs() - rel[1].abs()).abs() > SIZE_TOL_REL * rel[0].abs().max(rel[1].abs())
            && (has_anchor_offset(old_mapping) || has_anchor_offset(new_mapping))
        {
            out.refused.push(issue(
                IssueCode::Clearance,
                Severity::Error,
                format!("non-uniform scale {} x {} with an anchor offset: the box centre is ambiguous", t.scale, scale_y),
            ));
            continue;
        }
        let insert_mm = Vec2::new(t.insert[0], t.insert[1]) * req.unit_scale;
        let center_mm = center_from_insert_xy(old_mapping, from.length_mm, insert_mm, t.rotation_deg, rel);
        let angle_deg = t.rotation_deg - old_mapping.map_or(0.0, |m| m.rotation_offset_deg);
        let rotation_deg = angle_deg + new_mapping.map_or(0.0, |m| m.rotation_offset_deg);
        // The new box is drawn at true size, as fill places it, keeping the target's mirror signs.
        let signs = [rel[0].signum(), rel[1].signum()];
        let insert = insert_from_center_xy(new_mapping, to.length_mm, center_mm, rotation_deg, signs) / req.unit_scale;
        let resized =
            (from.width_mm * rel[0].abs() - to.width_mm).abs() > SIZE_TOL_MM || (from.length_mm * rel[1].abs() - to.length_mm).abs() > SIZE_TOL_MM;
        if !(insert.is_finite() && rotation_deg.is_finite()) {
            return Err(UbotError::BadInput(format!("target {}: position overflows in mm", t.handle)));
        }
        out.changes.push(ReplaceChange {
            handle: t.handle.clone(),
            product_id: to.id.clone(),
            block_name: new_block.clone(),
            insert: [insert.x, insert.y],
            rotation_deg,
            scale: new_scale,
            scale_xy: [signs[0] * new_scale, signs[1] * new_scale],
            resized,
        });
    }
    Ok(out)
}
