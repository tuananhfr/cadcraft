use std::collections::{BTreeMap, BTreeSet};

use cadcraft_doc::{Drawing, Handle};
use cadcraft_geom::Vec2;
use cadcraft_render::paper::paper_size;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codes::{self, issue};
use super::ids::{job_id, sheet_key};
use super::outline::{normalize_outline, rect_of};
use super::{
    FrameCandidate, FrameRegistry, MAX_SCAN_FRAMES, PlotScale, SHEETPLAN_SCHEMA, Severity, Sheet, SheetMode, SheetPlan, SheetSource, SheetStatus,
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    #[default]
    Block,
    Window,
    Layout,
    Last,
    Plan,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SheetOverride {
    pub paper: Option<String>,
    pub scale: Option<String>,
    pub window: Option<[[f64; 2]; 2]>,
    pub sheet_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub mode: ScanMode,
    #[serde(default)]
    pub handles: Vec<String>,
    #[serde(default)]
    pub windows: Vec<Value>,
    #[serde(default)]
    pub layouts: Vec<String>,
    #[serde(default)]
    pub paper: Option<String>,
    #[serde(default)]
    pub scale: Option<String>,
    #[serde(default)]
    pub overrides: BTreeMap<String, SheetOverride>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub plan: Option<Value>,
}

pub(crate) fn validate_request_value(value: &Value) -> Result<(), String> {
    super::types::validate_input_size(value)?;
    for name in ["handles", "windows", "layouts", "exclude", "overrides"] {
        let count = match value.get(name) {
            Some(Value::Array(items)) => items.len(),
            Some(Value::Object(items)) => items.len(),
            _ => 0,
        };
        if count > MAX_SCAN_FRAMES {
            return Err(format!("{name} exceeds {MAX_SCAN_FRAMES} entries"));
        }
    }
    Ok(())
}

pub fn build_plan(d: &Drawing, req: &ScanRequest, reg: &FrameRegistry, source_hash: &str) -> Result<SheetPlan, String> {
    if [req.handles.len(), req.windows.len(), req.layouts.len(), req.overrides.len(), req.exclude.len()].into_iter().any(|n| n > MAX_SCAN_FRAMES) {
        return Err(format!("scan exceeds {MAX_SCAN_FRAMES} entries"));
    }
    // Validate opaque JSON before serde can recursively clone it into the canonical request.
    for value in &req.windows {
        super::types::validate_input_size(value)?;
    }
    if let Some(value) = &req.plan {
        super::types::validate_input_size(value)?;
    }
    let request = serde_json::to_value(req).map_err(|error| error.to_string())?;
    validate_request_value(&request)?;
    if let Some(scale) = &req.scale {
        parse_scale(scale)?;
    }
    if let Some(paper) = &req.paper {
        paper_dimensions(paper)?;
    }
    let mut plan = SheetPlan {
        schema_version: SHEETPLAN_SCHEMA.into(),
        job_id: job_id(source_hash, &request),
        source_hash: source_hash.into(),
        sheets: Vec::new(),
        issues: Vec::new(),
    };
    match req.mode {
        ScanMode::Block => {
            let mut seen = BTreeSet::new();
            let mut handles = Vec::new();
            for text in &req.handles {
                let handle = parse_handle(text)?;
                if seen.insert(handle) {
                    handles.push(handle);
                } else {
                    plan.issues.push(issue(
                        Severity::Info,
                        codes::HANDLE_DUPLICATE,
                        format!("Duplicate handle {} was selected once.", handle.hex()),
                        None,
                    ));
                }
            }
            for (index, candidate) in super::detect_frames(d, &handles, reg).into_iter().enumerate() {
                plan.sheets.push(block_sheet(candidate, order(index)?, reg));
            }
        }
        ScanMode::Window => {
            for (index, value) in req.windows.iter().enumerate() {
                let (outline, rotation_deg) = window_outline(value)?;
                let [a, b, c, _] = outline.map(|[x, y]| Vec2::new(x, y));
                let mut sheet = empty_sheet(order(index)?, SheetSource::Window { outline, rotation_deg });
                sheet.paper_mm = paper_dimensions(req.paper.as_deref().unwrap_or("A3"))?;
                sheet.landscape = (b - a).len() > (c - b).len();
                sheet.plot_scale = match &req.scale {
                    Some(scale) => parse_scale(scale)?,
                    None => {
                        sheet.issues.push(issue(
                            Severity::Info,
                            codes::SCALE_NOT_SET,
                            "Window scale was not set; Fit is used.",
                            Some(&sheet.sheet_id),
                        ));
                        PlotScale::Fit
                    }
                };
                plan.sheets.push(sheet);
            }
        }
        ScanMode::Layout => {
            if d.layouts.len() > MAX_SCAN_FRAMES {
                return Err(format!("drawing exceeds {MAX_SCAN_FRAMES} layouts"));
            }
            let layouts = if req.layouts.iter().any(|name| name == "*") {
                let mut layouts: Vec<_> = d.layouts.iter().collect();
                layouts.sort_by_key(|layout| layout.tab_order);
                layouts
            } else {
                req.layouts.iter().map(|name| d.layout(name).ok_or_else(|| format!("unknown layout `{name}`"))).collect::<Result<Vec<_>, _>>()?
            };
            for (index, layout) in layouts.into_iter().enumerate() {
                let page = &layout.page;
                if ![page.width_mm, page.height_mm].into_iter().all(positive) {
                    return Err("layout paper dimensions must be finite and positive".into());
                }
                let mut sheet = empty_sheet(order(index)?, SheetSource::Layout { name: layout.name.clone() });
                sheet.paper_mm = [page.width_mm, page.height_mm];
                sheet.landscape = page.landscape;
                sheet.plot_scale = if page.scale_to_fit {
                    PlotScale::Fit
                } else if positive(page.scale) && positive(1.0 / page.scale) {
                    PlotScale::Ratio { paper_mm: 1.0, drawing_units: 1.0 / page.scale }
                } else {
                    return Err("layout plot scale must be finite and positive".into());
                };
                plan.sheets.push(sheet);
            }
        }
        ScanMode::Plan => {
            let input = req.plan.as_ref().ok_or_else(|| "PLAN_INVALID: plan is required".to_string())?;
            let mut supplied = super::parse_plan(input)?;
            let handles = supplied
                .sheets
                .iter()
                .filter_map(|sheet| match &sheet.source {
                    SheetSource::Block { block_handle, .. } => Some(parse_handle(block_handle)),
                    _ => None,
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut candidates = super::detect_frames(d, &handles, reg).into_iter();
            for sheet in &mut supplied.sheets {
                match &sheet.source {
                    SheetSource::Block { .. } => {
                        let candidate = candidates.next().ok_or_else(|| "FRAME_NOT_FOUND: missing frame candidate".to_string())?;
                        let detected = block_sheet(candidate, sheet.order, reg);
                        sheet.source = detected.source;
                        sheet.frame_template_id = detected.frame_template_id;
                        sheet.attributes = detected.attributes;
                        sheet.issues.retain(|i| !frame_issue(&i.code));
                        sheet.issues.extend(detected.issues.into_iter().filter(|i| i.code != codes::PAPER_CUSTOM));
                    }
                    SheetSource::Window { outline, .. } => {
                        if rect_of(&outline.map(|[x, y]| Vec2::new(x, y))).is_none() {
                            return Err("PLAN_INVALID: window outline must be a rectangle".into());
                        }
                    }
                    SheetSource::Layout { name } => {
                        if d.layout(name).is_none() {
                            return Err(format!("unknown layout `{name}`"));
                        }
                    }
                }
            }
            plan.sheets = supplied.sheets;
            plan.issues = supplied.issues;
        }
        ScanMode::Last => return Err("last mode requires session state".into()),
    }
    let excluded: BTreeSet<_> = req.exclude.iter().map(String::as_str).collect();
    let mut sheets = Vec::with_capacity(plan.sheets.len());
    for mut sheet in plan.sheets {
        let key = sheet_key(&sheet);
        if req.mode != ScanMode::Window {
            if let Some(paper) = &req.paper {
                sheet.paper_mm = paper_dimensions(paper)?;
                sheet.issues.retain(|i| i.code != codes::PAPER_CUSTOM);
            }
            if let Some(scale) = &req.scale {
                sheet.plot_scale = parse_scale(scale)?;
            }
        }
        if let Some(settings) = req.overrides.get(&key) {
            apply_override(&mut sheet, settings)?;
        }
        if excluded.contains(key.as_str()) {
            plan.issues.push(issue(Severity::Info, codes::SHEET_EXCLUDED, format!("Sheet {key} was excluded."), Some(&sheet.sheet_id)));
            continue;
        }
        for entry in &mut sheet.issues {
            entry.sheet_id = Some(sheet.sheet_id.clone());
        }
        sheet.issues.retain(|i| i.code != codes::PAPER_TOO_LARGE);
        if sheet.paper_mm.into_iter().any(|dimension| dimension > 5080.0) {
            sheet.issues.push(issue(Severity::Block, codes::PAPER_TOO_LARGE, "Paper dimension exceeds 5080 mm.", Some(&sheet.sheet_id)));
        }
        sheet.status = if sheet.issues.iter().chain(&plan.issues).any(|i| i.severity == Severity::Block) {
            SheetStatus::Blocked
        } else {
            SheetStatus::NeedsPreview
        };
        sheets.push(sheet);
    }
    plan.sheets = sheets;
    Ok(plan)
}

fn order(index: usize) -> Result<u32, String> {
    u32::try_from(index).ok().and_then(|n| n.checked_add(1)).ok_or_else(|| "sheet order exceeds the input limit".into())
}

fn parse_handle(text: &str) -> Result<Handle, String> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("frame handle must be hexadecimal".into());
    }
    u64::from_str_radix(text, 16).map(Handle).map_err(|_| "frame handle must be hexadecimal".into())
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn parse_scale(text: &str) -> Result<PlotScale, String> {
    PlotScale::parse(text).ok_or_else(|| "scale must be fit or a finite positive ratio".into())
}

fn paper_dimensions(name: &str) -> Result<[f64; 2], String> {
    paper_size(name).map(|paper| [paper.width_mm, paper.height_mm]).ok_or_else(|| format!("unknown paper size `{name}`"))
}

pub(crate) fn window_outline(value: &Value) -> Result<([[f64; 2]; 4], f64), String> {
    let points = value.as_array().ok_or_else(|| "window must contain two corners or four vertices".to_string())?;
    if !matches!(points.len(), 2 | 4) {
        return Err("window must contain two corners or four vertices".into());
    }
    let points = points
        .iter()
        .map(|point| {
            let [x, y]: [f64; 2] = serde_json::from_value(point.clone()).map_err(|_| "window coordinates must be finite pairs".to_string())?;
            Ok(Vec2::new(x, y))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let corners = match points.as_slice() {
        [a, b] => {
            let (lo, hi) = (a.min(*b), a.max(*b));
            [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)]
        }
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Err("window must contain two corners or four vertices".into()),
    };
    let rectangle = rect_of(&corners).ok_or_else(|| "window must be a finite nondegenerate rectangle".to_string())?;
    Ok(normalize_outline(rectangle, Vec2::Y))
}

fn empty_sheet(order: u32, source: SheetSource) -> Sheet {
    let mode = match &source {
        SheetSource::Block { .. } => SheetMode::Block,
        SheetSource::Window { .. } => SheetMode::Window,
        SheetSource::Layout { .. } => SheetMode::Layout,
    };
    Sheet {
        sheet_id: format!("S-{order:03}"),
        order,
        mode,
        source,
        frame_template_id: None,
        paper_mm: [297.0, 420.0],
        landscape: true,
        plot_scale: PlotScale::Fit,
        status: SheetStatus::NeedsPreview,
        issues: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

fn block_sheet(candidate: FrameCandidate, order: u32, reg: &FrameRegistry) -> Sheet {
    let template = reg.find(&candidate.effective_name);
    let mut sheet = empty_sheet(
        order,
        SheetSource::Block {
            block_handle: candidate.insert.hex(),
            effective_name: candidate.effective_name.clone(),
            outline: candidate.outline.unwrap_or([[0.0; 2]; 4]),
            rotation_deg: candidate.rotation_deg,
        },
    );
    if let Some(id) = template.and_then(|t| {
        t.attributes
            .sheet_id
            .iter()
            .take(64)
            .find_map(|tag| candidate.attributes.get(&tag.to_ascii_uppercase()).filter(|value| !value.trim().is_empty()))
    }) {
        sheet.sheet_id = id.clone();
    }
    sheet.frame_template_id = candidate.template_id;
    sheet.attributes = candidate.attributes;
    sheet.issues = candidate.issues;
    sheet.plot_scale = if !sheet.issues.iter().any(|i| i.code == codes::FRAME_NONUNIFORM) && positive(candidate.insert_scale) {
        PlotScale::Ratio { paper_mm: 1.0, drawing_units: candidate.insert_scale }
    } else {
        PlotScale::Fit
    };
    let [w, h] = candidate.local_mm;
    if [w, h].into_iter().all(positive) {
        sheet.landscape = w > h;
        let portrait = [w.min(h), w.max(h)];
        let standard = ["A0", "A1", "A2", "A3", "A4", "A5"].into_iter().filter_map(|name| paper_dimensions(name).ok());
        let media = template.into_iter().flat_map(|t| t.media_mm.iter().take(MAX_SCAN_FRAMES)).map(|[w, h]| [w.min(*h), w.max(*h)]);
        let found = media
            .chain(standard)
            .find(|size| size.iter().zip(portrait).all(|(expected, actual)| (expected - actual).abs() <= (expected * 0.005).max(1.0)));
        sheet.paper_mm = found.unwrap_or_else(|| portrait.map(|v| (v * 10.0).round() / 10.0));
        if found.is_none() {
            sheet.issues.push(issue(Severity::Info, codes::PAPER_CUSTOM, "Frame uses a custom paper size.", Some(&sheet.sheet_id)));
        }
        if !sheet.paper_mm.into_iter().all(positive) {
            sheet.paper_mm = [297.0, 420.0];
            sheet.issues.push(issue(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame paper dimensions are invalid.", Some(&sheet.sheet_id)));
        }
    }
    sheet
}

fn frame_issue(code: &str) -> bool {
    matches!(
        code,
        codes::FRAME_AMBIGUOUS
            | codes::FRAME_NOT_FOUND
            | codes::FRAME_MINSERT
            | codes::FRAME_DUPLICATE
            | codes::FRAME_OVERLAP
            | codes::FRAME_NESTED
            | codes::FRAME_MIRRORED
            | codes::FRAME_NONUNIFORM
    )
}

fn apply_override(sheet: &mut Sheet, settings: &SheetOverride) -> Result<(), String> {
    if let Some(window) = settings.window {
        if !window.into_iter().flatten().all(f64::is_finite) {
            return Err("window coordinates must be finite".into());
        }
        let (outline, rotation_deg) = window_outline(&serde_json::to_value(window).map_err(|error| error.to_string())?)?;
        sheet.source = SheetSource::Window { outline, rotation_deg };
        sheet.mode = SheetMode::Window;
        let [a, b, c, _] = outline.map(|[x, y]| Vec2::new(x, y));
        sheet.landscape = (b - a).len() > (c - b).len();
        sheet.frame_template_id = None;
        sheet.issues.retain(|i| !frame_issue(&i.code));
    }
    if let Some(paper) = &settings.paper {
        sheet.paper_mm = paper_dimensions(paper)?;
        sheet.issues.retain(|i| i.code != codes::PAPER_CUSTOM);
    }
    if let Some(scale) = &settings.scale {
        sheet.plot_scale = parse_scale(scale)?;
        sheet.issues.retain(|i| i.code != codes::SCALE_NOT_SET);
    }
    if let Some(id) = &settings.sheet_id {
        sheet.sheet_id = id.clone();
    }
    Ok(())
}
