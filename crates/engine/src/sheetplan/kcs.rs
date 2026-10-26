//! KCS Plotter plot areas: each Model-space LINE on layer `KCS_Plotter` marks one sheet with its diagonal.

use std::collections::BTreeSet;

use cadcraft_doc::{Drawing, EntityKind, Handle};
use cadcraft_geom::Vec2;

use super::codes::{self, issue};
use super::outline::{normalize_outline, points, rect_of};
use super::{MAX_SCAN_FRAMES, PreflightIssue, Severity, Sheet, SheetSource};

pub const KCS_LAYER: &str = "KCS_Plotter";
pub const NO_PLOT_AREAS: &str = "No KCS plot areas found: draw LINEs on layer KCS_Plotter (KL)";
// KCS rows windows whose top edges differ by at most the first window's height / 12.5.
const ROW_TOLERANCE_DIVISOR: f64 = 12.5;
const SNAP_TOLERANCE: f64 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KcsWindow {
    pub handle: Handle,
    pub min: Vec2,
    pub max: Vec2,
    pub outline: [[f64; 2]; 4],
    pub rotation_deg: f64,
}

#[derive(Debug, Default)]
pub struct KcsScan {
    pub windows: Vec<KcsWindow>,
    pub issues: Vec<PreflightIssue>,
}

pub fn is_marker_layer(layer: &str) -> bool {
    layer.eq_ignore_ascii_case(KCS_LAYER)
}

/// Marker windows in KCS print order; `only` restricts the scan to selected handles.
pub fn scan(d: &Drawing, only: Option<&BTreeSet<Handle>>) -> Result<KcsScan, String> {
    let mut result = KcsScan::default();
    let mut found = BTreeSet::new();
    for entity in d.model.iter() {
        let EntityKind::Line(line) = &entity.kind else { continue };
        if !is_marker_layer(&entity.common.layer) || only.is_some_and(|only| !only.contains(&entity.handle)) {
            continue;
        }
        found.insert(entity.handle);
        let (a, b) = (line.a.xy(), line.b.xy());
        let (min, max) = (a.min(b), a.max(b));
        let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        let Some(rectangle) = rect_of(&corners).filter(|_| max.x > min.x && max.y > min.y) else {
            result.issues.push(issue(
                Severity::Warn,
                codes::KCS_LINE_DEGENERATE,
                format!("KCS plot area line {} has no width or height and was skipped.", entity.handle.hex()),
                None,
            ));
            continue;
        };
        if result.windows.len() >= MAX_SCAN_FRAMES {
            return Err(format!("KCS scan exceeds {MAX_SCAN_FRAMES} plot areas"));
        }
        let (outline, rotation_deg) = normalize_outline(rectangle, Vec2::Y);
        result.windows.push(KcsWindow { handle: entity.handle, min, max, outline, rotation_deg });
    }
    for handle in only.into_iter().flatten().filter(|handle| !found.contains(handle)) {
        result.issues.push(issue(
            Severity::Info,
            codes::KCS_NOT_MARKER,
            format!("Handle {} is not a LINE on layer KCS_Plotter and was ignored.", handle.hex()),
            None,
        ));
    }
    if result.windows.is_empty() {
        return Err(NO_PLOT_AREAS.into());
    }
    kcs_order(&mut result.windows);
    Ok(result)
}

/// KCS `Sap_Xep_Window_List`: an exchange sort, top edge descending, then left edge ascending within a row.
/// Replayed step for step because its row test is not transitive, so the result depends on input order.
pub fn kcs_order(windows: &mut [KcsWindow]) {
    let Some(first) = windows.first() else { return };
    let tolerance = (first.max.y - first.min.y) / ROW_TOLERANCE_DIVISOR;
    let count = windows.len();
    for i in 0..count {
        for j in i.saturating_add(1)..count {
            let (Some(a), Some(b)) = (windows.get(i), windows.get(j)) else { continue };
            if a.max.y < b.max.y {
                windows.swap(i, j);
            }
            let (Some(a), Some(b)) = (windows.get(i), windows.get(j)) else { continue };
            // As in KCS, an incomparable (NaN) difference counts as the same row.
            let same_row = (a.max.y - b.max.y).abs().partial_cmp(&tolerance) != Some(std::cmp::Ordering::Greater);
            if same_row && a.min.x > b.min.x {
                windows.swap(i, j);
            }
        }
    }
}

/// The fit-to-paper scale as `1:n`, snapped to a whole `n` within 0.5 %.
pub fn fit_label(sheet: &Sheet, unit_mm: f64, margins: [f64; 4]) -> Option<String> {
    let SheetSource::Window { outline, .. } = &sheet.source else { return None };
    let [a, b, c, _] = points(outline);
    let (width, height) = ((b - a).len() * unit_mm, (c - b).len() * unit_mm);
    let [p, q] = sheet.paper_mm;
    let (paper_w, paper_h) = if sheet.landscape { (p.max(q), p.min(q)) } else { (p.min(q), p.max(q)) };
    let [left, bottom, right, top] = margins;
    let scale = ((paper_w - left - right) / width).min((paper_h - bottom - top) / height);
    let n = 1.0 / scale;
    if !(n.is_finite() && n > 0.0) {
        return None;
    }
    let whole = n.round();
    if whole >= 1.0 && (n - whole).abs() <= n * SNAP_TOLERANCE {
        Some(format!("1:{whole}"))
    } else {
        Some(format!("1:{}", (n * 100.0).round() / 100.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(x: f64, top: f64, width: f64, height: f64) -> KcsWindow {
        let (min, max) = (Vec2::new(x, top - height), Vec2::new(x + width, top));
        KcsWindow { handle: Handle(0), min, max, outline: [[0.0; 2]; 4], rotation_deg: 0.0 }
    }

    #[test]
    fn order_is_rows_top_down_then_left_to_right() {
        let mut windows =
            vec![window(20.0, 0.0, 10.0, 7.0), window(0.0, 0.0, 10.0, 7.0), window(10.0, 10.0, 10.0, 7.0), window(0.0, 10.3, 10.0, 7.0)];
        kcs_order(&mut windows);
        let order: Vec<_> = windows.iter().map(|w| (w.min.x, w.max.y)).collect();
        assert_eq!(order, vec![(0.0, 10.3), (10.0, 10.0), (0.0, 0.0), (20.0, 0.0)]);
    }

    #[test]
    fn row_tolerance_comes_from_the_first_window() {
        // 12.5 / 12.5 = 1.0: a 0.9 step stays in the row, a 1.1 step starts a new one.
        let mut same = vec![window(10.0, 0.0, 10.0, 12.5), window(0.0, -0.9, 10.0, 12.5)];
        kcs_order(&mut same);
        assert_eq!(same.first().map(|w| w.min.x), Some(0.0));
        let mut next = vec![window(10.0, 0.0, 10.0, 12.5), window(0.0, -1.1, 10.0, 12.5)];
        kcs_order(&mut next);
        assert_eq!(next.first().map(|w| w.min.x), Some(10.0));
    }

    #[test]
    fn empty_and_single_lists_are_unchanged() {
        let mut empty: Vec<KcsWindow> = Vec::new();
        kcs_order(&mut empty);
        let mut one = vec![window(0.0, 0.0, f64::NAN, f64::NAN)];
        kcs_order(&mut one);
        assert_eq!(one.len(), 1);
    }
}
