use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{SheetPlan, SheetSource, parse_plan};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanDiff {
    pub index: usize,
    pub field: String,
    pub a: String,
    pub b: String,
}

pub(crate) fn parse_ordered(value: &Value) -> Result<SheetPlan, String> {
    // Validate limits and geometry first; comparison must retain the input array's order.
    parse_plan(value)?;
    SheetPlan::deserialize(value).map_err(|error| format!("PLAN_INVALID: {error}"))
}

pub(crate) fn tolerance(value: Option<&Value>) -> Result<f64, String> {
    match value {
        None => Ok(1e-6),
        Some(value) => value
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1.0)
            .ok_or_else(|| "tol must be finite and within 0 < tol <= 1".into()),
    }
}

/// Compares sheets by index, with absolute per-coordinate tolerance only for outlines.
/// Hashes, schema, status, issues and other metadata are outside the portable LISP contract.
pub fn compare(a: &SheetPlan, b: &SheetPlan, tol: f64) -> Vec<PlanDiff> {
    let mut diffs = Vec::new();
    difference(&mut diffs, 0, "count", &a.sheets.len(), &b.sheets.len());
    for (index, (a, b)) in a.sheets.iter().zip(&b.sheets).enumerate() {
        difference(&mut diffs, index, "sheet_id", &a.sheet_id, &b.sheet_id);
        difference(&mut diffs, index, "mode", &a.mode, &b.mode);
        let (ah, an, ao) = source_fields(&a.source);
        let (bh, bn, bo) = source_fields(&b.source);
        difference(&mut diffs, index, "block_handle", &ah, &bh);
        difference(&mut diffs, index, "effective_name", &an, &bn);
        let same_outline = match (ao, bo) {
            (Some(a), Some(b)) => a.iter().flatten().zip(b.iter().flatten()).all(|(a, b)| (a - b).abs() <= tol),
            (None, None) => true,
            _ => false,
        };
        if !same_outline {
            diffs.push(PlanDiff { index, field: "outline".into(), a: format!("{ao:?}"), b: format!("{bo:?}") });
        }
        difference(&mut diffs, index, "paper_mm", &a.paper_mm, &b.paper_mm);
        difference(&mut diffs, index, "landscape", &a.landscape, &b.landscape);
        difference(&mut diffs, index, "frame_template_id", &a.frame_template_id, &b.frame_template_id);
    }
    diffs
}

type SourceFields<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a [[f64; 2]; 4]>);

fn source_fields(source: &SheetSource) -> SourceFields<'_> {
    match source {
        SheetSource::Block { block_handle, effective_name, outline, .. } => (Some(block_handle), Some(effective_name), Some(outline)),
        SheetSource::Window { outline, .. } => (None, None, Some(outline)),
        SheetSource::Layout { .. } => (None, None, None),
    }
}

fn difference<T: PartialEq + std::fmt::Debug>(diffs: &mut Vec<PlanDiff>, index: usize, field: &str, a: &T, b: &T) {
    if a != b {
        diffs.push(PlanDiff { index, field: field.into(), a: format!("{a:?}"), b: format!("{b:?}") });
    }
}
