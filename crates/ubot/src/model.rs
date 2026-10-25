use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Draft,
    Approved,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub id: String,
    pub sku: String,
    pub name: String,
    #[serde(default)]
    pub manufacturer: Option<String>,
    pub width_mm: f64,
    pub length_mm: f64,
    #[serde(default)]
    pub height_mm: Option<f64>,
    #[serde(default)]
    pub slab_thickness_mm: Option<f64>,
    pub version: String,
    pub approval_status: ApprovalStatus,
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// Insertion point of a block relative to the footprint centre, in the unrotated block frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Anchor {
    Center,
    TopCenter,
    Offset { x_mm: f64, y_mm: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockMapping {
    #[serde(default)]
    pub product_id: Option<String>,
    pub cad_engine: String,
    pub block_name: String,
    pub units_mm_per_block_unit: f64,
    pub anchor: Anchor,
    pub rotation_offset_deg: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulePack {
    pub id: String,
    pub version: String,
    pub approval_status: ApprovalStatus,
    pub edge_clearance_mm: f64,
    pub exclusion_clearance_mm: f64,
    pub rib_mm_default: f64,
    pub rib_mm_choices: Vec<f64>,
}
