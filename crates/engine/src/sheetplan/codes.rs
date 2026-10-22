use super::types::{PreflightIssue, Severity};

pub const XREF_MISSING: &str = "XREF_MISSING";
pub const XREF_UNLOADED: &str = "XREF_UNLOADED";
pub const XREF_MISSING_OUTSIDE: &str = "XREF_MISSING_OUTSIDE";
pub const XREF_SCAN_LIMIT: &str = "XREF_SCAN_LIMIT";
pub const FRAME_AMBIGUOUS: &str = "FRAME_AMBIGUOUS";
pub const FRAME_NOT_FOUND: &str = "FRAME_NOT_FOUND";
pub const FRAME_MINSERT: &str = "FRAME_MINSERT";
pub const PAPER_TOO_SMALL: &str = "PAPER_TOO_SMALL";
pub const SCALE_PAPER_MISMATCH: &str = "SCALE_PAPER_MISMATCH";
pub const PAPER_TOO_LARGE: &str = "PAPER_TOO_LARGE";
pub const RENDER_FAILED: &str = "RENDER_FAILED";
pub const FRAME_DUPLICATE: &str = "FRAME_DUPLICATE";
pub const FRAME_OVERLAP: &str = "FRAME_OVERLAP";
pub const FRAME_NESTED: &str = "FRAME_NESTED";
pub const FRAME_MIRRORED: &str = "FRAME_MIRRORED";
pub const FRAME_NONUNIFORM: &str = "FRAME_NONUNIFORM";
pub const PAPER_CUSTOM: &str = "PAPER_CUSTOM";
pub const UNITS_ASSUMED: &str = "UNITS_ASSUMED";
pub const SCALE_NOT_SET: &str = "SCALE_NOT_SET";
pub const HANDLE_DUPLICATE: &str = "HANDLE_DUPLICATE";
pub const SHEET_EXCLUDED: &str = "SHEET_EXCLUDED";
pub const SOURCE_HASH_WEAK: &str = "SOURCE_HASH_WEAK";
pub const OUTPUT_IS_SOURCE: &str = "OUTPUT_IS_SOURCE";
pub const PLAN_STALE: &str = "PLAN_STALE";
pub const PLAN_INVALID: &str = "PLAN_INVALID";

pub fn issue(severity: Severity, code: &str, message: impl Into<String>, sheet_id: Option<&str>) -> PreflightIssue {
    PreflightIssue { severity, code: code.into(), message: message.into(), sheet_id: sheet_id.map(str::to_string), approved_by_user: false }
}
