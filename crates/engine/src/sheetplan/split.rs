use cadcraft_doc::PlotterConfig;
use cadcraft_geom::Vec2;

use super::ids::{sha256_hex, sheet_key};
use super::outline::{points, rect_of};
use super::{AuditEntry, PlotScale, Sheet, SheetSource, SplitPlan};

pub const DEFAULT_OVERLAP_MM: f64 = 20.0;

pub(crate) fn device_limits(device: &PlotterConfig) -> Option<(f64, f64)> {
    if device.name.eq_ignore_ascii_case("CADCraft PDF") {
        return Some((cadcraft_doc::MAX_PAPER_MM, cadcraft_doc::MAX_PAPER_MM));
    }
    let mut long: f64 = 0.0;
    let mut short: f64 = 0.0;
    for paper in device.papers.iter().take(cadcraft_doc::MAX_DEVICE_PAPERS) {
        if ![paper.width, paper.height].into_iter().all(positive) {
            continue;
        }
        long = long.max(paper.width.max(paper.height));
        short = short.max(paper.width.min(paper.height));
    }
    (positive(long) && positive(short)).then_some((long, short))
}

pub(crate) fn frame(sheet: &Sheet) -> Option<([Vec2; 4], [f64; 2], f64)> {
    if !sheet.paper_mm.into_iter().all(positive) {
        return None;
    }
    let outline = match &sheet.source {
        SheetSource::Block { outline, .. } | SheetSource::Window { outline, .. } => outline,
        SheetSource::Layout { .. } => return None,
    };
    let corners = rect_of(&points(outline))?;
    let [a, b, c, _] = corners;
    let x = (b - a).len();
    let y = (c - b).len();
    let [p, q] = sheet.paper_mm;
    let (width, height) = if sheet.landscape { (p.max(q), p.min(q)) } else { (p.min(q), p.max(q)) };
    let scale = match sheet.plot_scale {
        PlotScale::Ratio { paper_mm, drawing_units } if positive(paper_mm) && positive(drawing_units) => paper_mm / drawing_units,
        PlotScale::Ratio { .. } => return None,
        PlotScale::Fit => (width / x).min(height / y),
    };
    let size = [x * scale, y * scale];
    (positive(scale) && size.into_iter().all(positive)).then_some((corners, size, scale))
}

pub fn propose_split(sheet: &Sheet, max_long_mm: f64, max_short_mm: f64, overlap_mm: f64) -> Option<SplitPlan> {
    if ![max_long_mm, max_short_mm].into_iter().all(positive) || !overlap_mm.is_finite() || !(0.0..=200.0).contains(&overlap_mm) {
        return None;
    }
    let (corners, [width, height], scale) = frame(sheet)?;
    let long = width.max(height);
    let [paper_a, paper_b] = sheet.paper_mm;
    if paper_a.min(paper_b) > max_short_mm || width.min(height) > max_short_mm || long <= max_long_mm || max_long_mm <= overlap_mm {
        return None;
    }
    let count = ((long - overlap_mm) / (max_long_mm - overlap_mm)).ceil();
    if !count.is_finite() || !(2.0..=cadcraft_doc::MAX_PLOT_PAGES as f64).contains(&count) {
        return None;
    }
    let piece = (long + (count - 1.0) * overlap_mm) / count;
    let [a, b, _, d] = corners;
    let direction = if width >= height { (b - a) / (b - a).len() } else { (d - a) / (d - a).len() };
    let mut pieces = Vec::with_capacity(count as usize);
    for index in 0..count as u32 {
        let from = direction * (f64::from(index) * (piece - overlap_mm) / scale);
        let end = direction * ((f64::from(index) * (piece - overlap_mm) + piece) / scale);
        let corners = if width >= height { [a + from, a + end, d + end, d + from] } else { [a + from, b + from, b + end, a + end] };
        let outline = corners.map(|p| [p.x, p.y]);
        if !outline.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        pieces.push(outline);
    }
    Some(SplitPlan { pieces, overlap_mm, approved: false })
}

fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.0
}

pub(crate) fn split_fingerprint(sheet: &Sheet, device: Option<&PlotterConfig>) -> String {
    let split = sheet.split.as_ref().map(|split| (&split.pieces, split.overlap_mm));
    let value = serde_json::json!({
        "source": sheet.source, "paper": sheet.paper_mm, "landscape": sheet.landscape,
        "scale": sheet.plot_scale, "split": split, "device": device.map(|d| &d.name),
        "limits": device.and_then(device_limits), "sheetKey": sheet_key(sheet)
    });
    sha256_hex(value.to_string().as_bytes()).chars().take(12).collect()
}

pub(crate) fn split_approved(sheet: &Sheet, audit: &[AuditEntry], device: Option<&PlotterConfig>) -> bool {
    let Some(split) = sheet.split.as_ref().filter(|split| split.approved) else { return false };
    let Some((long, short)) = device.and_then(device_limits) else { return false };
    if !propose_split(sheet, long, short, split.overlap_mm).is_some_and(|expected| expected.pieces == split.pieces) {
        return false;
    }
    let key = sheet_key(sheet);
    let fingerprint = split_fingerprint(sheet, device);
    audit.iter().any(|record| {
        record.action == "split" && record.sheet_key == key && record.code == super::codes::DEVICE_LIMIT_EXCEEDED && record.fingerprint == fingerprint
    })
}
