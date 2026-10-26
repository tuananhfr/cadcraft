pub const DEVICE_LIMIT_NEAR: &str = "DEVICE_LIMIT_NEAR";
pub const DEVICE_LIMIT_EXCEEDED: &str = "DEVICE_LIMIT_EXCEEDED";
pub const DEVICE_UNAVAILABLE: &str = "DEVICE_UNAVAILABLE";
pub const PRINTER_MISSING: &str = "PRINTER_MISSING";
pub const PRINTER_HANDLER_MISSING: &str = "PRINTER_HANDLER_MISSING";
use super::types::{PreflightIssue, Severity};

pub const XREF_MISSING: &str = "XREF_MISSING";
pub const XREF_UNLOADED: &str = "XREF_UNLOADED";
pub const XREF_MISSING_OUTSIDE: &str = "XREF_MISSING_OUTSIDE";
pub const XREF_SCAN_LIMIT: &str = "XREF_SCAN_LIMIT";
pub const FONT_FALLBACK: &str = "FONT_FALLBACK";
pub const FONT_MISSING: &str = "FONT_MISSING";
pub const FONT_MAPPED: &str = "FONT_MAPPED";
pub const SHEET_ID_DUPLICATE: &str = "SHEET_ID_DUPLICATE";
pub const REVISION_OLD: &str = "REVISION_OLD";
pub const REVISION_NEWER: &str = "REVISION_NEWER";
pub const TEMPLATE_SUPERSEDED: &str = "TEMPLATE_SUPERSEDED";
pub const PLOT_STYLE: &str = "PLOT_STYLE";
pub const FILE_RENAMED: &str = "FILE_RENAMED";
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
pub const PROFILE_LEVEL_SKIPPED: &str = "PROFILE_LEVEL_SKIPPED";
pub const BASELINE_CHANGED: &str = "BASELINE_CHANGED";
pub const AUDIT_LIMIT: &str = "AUDIT_LIMIT";
pub const PLAN_INVALID: &str = "PLAN_INVALID";
pub const KCS_LINE_DEGENERATE: &str = "KCS_LINE_DEGENERATE";
pub const KCS_NOT_MARKER: &str = "KCS_NOT_MARKER";

pub fn issue(severity: Severity, code: &str, message: impl Into<String>, sheet_id: Option<&str>) -> PreflightIssue {
    PreflightIssue { severity, code: code.into(), message: message.into(), sheet_id: sheet_id.map(str::to_string), approved_by_user: false }
}
