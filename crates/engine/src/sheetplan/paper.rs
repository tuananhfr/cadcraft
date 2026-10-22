use cadcraft_doc::{Drawing, Handle, MAX_PAPER_MM, PlotOutput, Space};
use cadcraft_render::PaperSize;
use cadcraft_render::paper::{ELONGATED_SIZES, PAPER_SIZES, paper_size};

use super::codes::{self, issue};
use super::{PreflightIssue, Severity, Sheet, SheetSource};

#[derive(Clone, Debug, PartialEq)]
pub enum PaperChoice {
    Auto,
    Named(&'static PaperSize),
    Custom { w: f64, h: f64 },
    Roll { width: f64, margin: f64 },
}

impl PaperChoice {
    pub fn parse(text: &str) -> Result<PaperChoice, String> {
        if text.len() > 256 {
            return Err("paper choice exceeds 256 bytes".into());
        }
        let text = text.trim();
        if text.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        if let Some(paper) = paper_size(text) {
            return Ok(Self::Named(paper));
        }
        let invalid = || format!("invalid paper `{text}`: use auto, a paper name, WIDTHxHEIGHT or roll:WIDTH[:MARGIN]");
        let lower = text.to_ascii_lowercase();
        if let Some(roll) = lower.strip_prefix("roll:") {
            let (width, margin) = match roll.split_once(':') {
                Some((width, margin)) => (width, margin.trim().parse::<f64>().map_err(|_| invalid())?),
                None => (roll, 5.0),
            };
            let width = width.trim().parse::<f64>().map_err(|_| invalid())?;
            if !valid_side(width) || !valid_margin(margin) || 2.0 * margin >= width {
                return Err(invalid());
            }
            return Ok(Self::Roll { width, margin });
        }
        let (w, h) = lower.split_once('x').ok_or_else(invalid)?;
        let w = w.trim().parse::<f64>().map_err(|_| invalid())?;
        let h = h.trim().parse::<f64>().map_err(|_| invalid())?;
        if !valid_side(w) || !valid_side(h) {
            return Err(invalid());
        }
        Ok(Self::Custom { w, h })
    }
}

pub struct ResolvedPaper {
    pub paper_mm: [f64; 2],
    pub landscape: bool,
    pub name: String,
}

fn valid_side(value: f64) -> bool {
    value.is_finite() && (1.0..=MAX_PAPER_MM).contains(&value)
}

fn valid_margin(value: f64) -> bool {
    value.is_finite() && (0.0..=1000.0).contains(&value)
}

pub(super) fn matches_size(expected: [f64; 2], actual: [f64; 2]) -> bool {
    expected.into_iter().zip(actual).all(|(expected, actual)| (expected - actual).abs() <= (expected * 0.005).max(1.0))
}

pub fn resolve_paper(choice: &PaperChoice, frame_on_paper_mm: [f64; 2], margins_mm: [f64; 4]) -> Result<ResolvedPaper, PreflightIssue> {
    let block = |code, message| issue(Severity::Block, code, message, None);
    let [w, h] = frame_on_paper_mm;
    if !frame_on_paper_mm.into_iter().all(|side| side.is_finite() && side > 0.0) {
        return Err(block(codes::PAPER_TOO_LARGE, "Frame paper dimensions must be finite and positive.".into()));
    }
    if !margins_mm.into_iter().all(valid_margin) {
        return Err(block(codes::PAPER_TOO_SMALL, "Paper margins must be finite and within 0..=1000 mm.".into()));
    }
    let portrait = [w.min(h), w.max(h)];
    let landscape = w > h;
    let (paper_mm, name) = match choice {
        PaperChoice::Auto => {
            if let Some(paper) = PAPER_SIZES.iter().chain(ELONGATED_SIZES).find(|p| matches_size([p.width_mm, p.height_mm], portrait)) {
                ([paper.width_mm, paper.height_mm], paper.name.to_string())
            } else {
                let [w, h] = portrait.map(|side| (side * 10.0).round() / 10.0);
                ([w, h], format!("{w}x{h}"))
            }
        }
        PaperChoice::Named(paper) => ([paper.width_mm, paper.height_mm], paper.name.to_string()),
        PaperChoice::Custom { w, h } => {
            if ![*w, *h].into_iter().all(valid_side) {
                return Err(block(codes::PAPER_TOO_LARGE, format!("Paper dimensions must be within 1..={MAX_PAPER_MM} mm.")));
            }
            ([w.min(*h), w.max(*h)], format!("{w}x{h}"))
        }
        PaperChoice::Roll { width, margin } => {
            if !valid_side(*width) || !valid_margin(*margin) {
                return Err(block(codes::PAPER_TOO_LARGE, format!("Roll width must be within 1..={MAX_PAPER_MM} mm and margin within 0..=1000 mm.")));
            }
            let [short, long] = portrait;
            if short + 2.0 * margin > *width {
                return Err(block(
                    codes::PAPER_TOO_SMALL,
                    format!(
                        "frame needs {} mm across roll including margins; roll {width} mm is too narrow; choose a wider roll or paper",
                        short + 2.0 * margin
                    ),
                ));
            }
            let length = (long + 2.0 * margin).ceil();
            if !valid_side(length) {
                return Err(block(codes::PAPER_TOO_LARGE, format!("Roll length {length} mm exceeds {MAX_PAPER_MM} mm.")));
            }
            return Ok(ResolvedPaper { paper_mm: [*width, length], landscape, name: format!("roll:{width}:{margin}") });
        }
    };
    if !paper_mm.into_iter().all(valid_side) {
        return Err(block(codes::PAPER_TOO_LARGE, format!("Paper dimensions must be within 1..={MAX_PAPER_MM} mm.")));
    }
    let [a, b] = paper_mm;
    let [left, bottom, right, top] = margins_mm;
    let [pw, ph] = if landscape { [a.max(b) - left - right, a.min(b) - bottom - top] } else { [a.min(b) - left - right, a.max(b) - bottom - top] };
    if pw <= 0.0 || ph <= 0.0 || w > pw + (pw * 0.005).max(1.0) || h > ph + (ph * 0.005).max(1.0) {
        return Err(block(codes::SCALE_PAPER_MISMATCH, format!("frame needs {w}×{h} mm at selected scale; paper {name} prints {pw}×{ph} mm")));
    }
    Ok(ResolvedPaper { paper_mm, landscape, name })
}

pub(crate) fn sheet_margins(d: &Drawing, sheet: &Sheet) -> [f64; 4] {
    if let Some(PaperChoice::Roll { margin, .. }) = sheet.paper_name.as_deref().and_then(|name| PaperChoice::parse(name).ok()) {
        return [margin; 4];
    }
    let layout = match &sheet.source {
        SheetSource::Layout { name } => d.layout(name),
        SheetSource::Block { block_handle, .. } => Handle::parse_hex(block_handle).and_then(|handle| match d.space_of(handle)? {
            Space::Paper(name) => d.layout(&name),
            Space::Model => None,
        }),
        SheetSource::Window { .. } => None,
    };
    let [a, b] = sheet.paper_mm;
    layout
        .and_then(|layout| {
            let device = crate::cmd::plotters::find_device(&layout.page.device)?;
            if device.output == PlotOutput::Pdf {
                return None;
            }
            let matches_size = |paper: &&cadcraft_doc::DevicePaper| {
                (paper.width.min(paper.height) - a.min(b)).abs() < 0.01 && (paper.width.max(paper.height) - a.max(b)).abs() < 0.01
            };
            device.paper(&layout.page.paper).filter(matches_size).or_else(|| device.papers.iter().find(matches_size)).map(|paper| paper.margins)
        })
        .unwrap_or([0.0; 4])
}
