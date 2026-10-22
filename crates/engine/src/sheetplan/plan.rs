use std::collections::{BTreeMap, BTreeSet};

use cadcraft_doc::{Drawing, Handle};
use cadcraft_geom::Vec2;
use cadcraft_render::paper::{insunits_mm, paper_size};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codes::{self, issue};
use super::ids::{job_id, sheet_key};
use super::outline::{normalize_outline, rect_of};
use super::paper::{PaperChoice, matches_size, resolve_paper, sheet_margins};
use super::preflight::{PreflightContext, preflight};
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
    let ctx = PreflightContext {
        registry: reg,
        device: None,
        device_missing: false,
        revisions: &BTreeMap::new(),
        fonts: &[],
        web: cfg!(target_arch = "wasm32"),
    };
    build_plan_with_context(d, req, &ctx, source_hash)
}

pub(crate) fn build_plan_with_context(d: &Drawing, req: &ScanRequest, ctx: &PreflightContext<'_>, source_hash: &str) -> Result<SheetPlan, String> {
    let reg = ctx.registry;
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
    let (unit_mm, assumed) = drawing_unit_mm(d);
    if let Some(scale) = &req.scale {
        parse_scale(scale, unit_mm)?;
    }
    if let Some(paper) = &req.paper {
        PaperChoice::parse(paper)?;
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
                plan.sheets.push(block_sheet(candidate, order(index)?, reg, unit_mm));
            }
        }
        ScanMode::Window => {
            for (index, value) in req.windows.iter().enumerate() {
                let (outline, rotation_deg) = window_outline(value)?;
                let [a, b, c, _] = outline.map(|[x, y]| Vec2::new(x, y));
                let mut sheet = empty_sheet(order(index)?, SheetSource::Window { outline, rotation_deg });
                sheet.landscape = (b - a).len() > (c - b).len();
                sheet.plot_scale = match &req.scale {
                    Some(scale) => parse_scale(scale, unit_mm)?,
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
                sheet.paper_name = Some(page.paper.clone());
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
                        let detected = block_sheet(candidate, sheet.order, reg, unit_mm);
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
    plan.issues.retain(|entry| entry.code != codes::UNITS_ASSUMED);
    if assumed {
        plan.issues.push(issue(
            Severity::Info,
            codes::UNITS_ASSUMED,
            "INSUNITS is unitless or unknown; drawing units are assumed to be millimetres.",
            None,
        ));
    }
    let excluded: BTreeSet<_> = req.exclude.iter().map(String::as_str).collect();
    let mut sheets = Vec::with_capacity(plan.sheets.len());
    for mut sheet in plan.sheets {
        let key = sheet_key(&sheet);
        if let Some(scale) = &req.scale {
            sheet.plot_scale = parse_scale(scale, unit_mm)?;
            sheet.scale_label = Some(scale.clone());
        }
        if let Some(settings) = req.overrides.get(&key) {
            apply_override(&mut sheet, settings, unit_mm)?;
        }
        if excluded.contains(key.as_str()) {
            plan.issues.push(issue(Severity::Info, codes::SHEET_EXCLUDED, format!("Sheet {key} was excluded."), Some(&sheet.sheet_id)));
            continue;
        }
        let paper = req.overrides.get(&key).and_then(|settings| settings.paper.as_deref()).or(req.paper.as_deref());
        resolve_sheet_paper(d, &mut sheet, paper, unit_mm)?;
        for entry in &mut sheet.issues {
            entry.sheet_id = Some(sheet.sheet_id.clone());
        }
        sheets.push(sheet);
    }
    plan.sheets = sheets;
    preflight(d, &mut plan, ctx);
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

pub(crate) fn drawing_unit_mm(d: &Drawing) -> (f64, bool) {
    match insunits_mm(d.header.i64("INSUNITS", 0)) {
        Some(unit_mm) => (unit_mm, false),
        None => (1.0, true),
    }
}

fn parse_scale(text: &str, unit_mm: f64) -> Result<PlotScale, String> {
    PlotScale::from_real_ratio(text, unit_mm).ok_or_else(|| "scale must be fit or a finite positive ratio".into())
}

fn resolve_sheet_paper(d: &Drawing, sheet: &mut Sheet, selected: Option<&str>, unit_mm: f64) -> Result<(), String> {
    let [existing_w, existing_h] = sheet.paper_mm;
    let existing = || {
        sheet
            .paper_name
            .as_deref()
            .and_then(|name| PaperChoice::parse(name).ok())
            .filter(|choice| match choice {
                PaperChoice::Named(paper) => {
                    (paper.width_mm - existing_w.min(existing_h)).abs() < 0.01 && (paper.height_mm - existing_w.max(existing_h)).abs() < 0.01
                }
                PaperChoice::Roll { .. } => true,
                _ => false,
            })
            .unwrap_or(PaperChoice::Custom { w: existing_w, h: existing_h })
    };
    let mut choice = match selected {
        Some(text) => PaperChoice::parse(text)?,
        None => existing(),
    };
    // Block media is chosen from the unscaled frame; a scale override must not enlarge it to hide a mismatch.
    if choice == PaperChoice::Auto && !matches!(&sheet.source, SheetSource::Window { .. }) {
        choice = existing();
    }
    let outline_size = match &sheet.source {
        SheetSource::Block { outline, .. } | SheetSource::Window { outline, .. } => {
            let [a, b, c, _] = outline.map(|[x, y]| Vec2::new(x, y));
            Some([(b - a).len(), (c - b).len()])
        }
        SheetSource::Layout { .. } => None,
    };
    let landscape = outline_size.map_or(sheet.landscape, |[w, h]| w > h);
    match &choice {
        PaperChoice::Named(paper) => {
            sheet.paper_mm = [paper.width_mm, paper.height_mm];
            sheet.paper_name = Some(paper.name.into());
        }
        PaperChoice::Custom { w, h } => {
            sheet.paper_mm = [w.min(*h), w.max(*h)];
            sheet.paper_name = Some(format!("{w}x{h}"));
        }
        PaperChoice::Roll { width, margin } => sheet.paper_name = Some(format!("roll:{width}:{margin}")),
        PaperChoice::Auto => {}
    }
    sheet.issues.retain(|entry| {
        !matches!(entry.code.as_str(), codes::PAPER_CUSTOM | codes::PAPER_TOO_SMALL | codes::PAPER_TOO_LARGE | codes::SCALE_PAPER_MISMATCH)
    });
    if outline_size.is_some_and(|size| !size.into_iter().all(positive)) {
        return Ok(());
    }
    let [a, b] = sheet.paper_mm;
    let oriented = if landscape { [a.max(b), a.min(b)] } else { [a.min(b), a.max(b)] };
    let margins = if sheet.plot_scale == PlotScale::Fit && !matches!(choice, PaperChoice::Auto | PaperChoice::Roll { .. }) {
        [0.0; 4]
    } else {
        sheet_margins(d, sheet)
    };
    let frame = match (&sheet.plot_scale, outline_size) {
        (PlotScale::Ratio { paper_mm, drawing_units }, Some(size)) => size.map(|side| side * (paper_mm / drawing_units)),
        (PlotScale::Fit, Some(size)) if matches!(choice, PaperChoice::Auto | PaperChoice::Roll { .. }) => size.map(|side| side * unit_mm),
        (_, None) => {
            let [w, h] = oriented;
            let [l, b, r, t] = margins;
            [w - l - r, h - b - t]
        }
        _ => oriented,
    };
    match resolve_paper(&choice, frame, margins) {
        Ok(resolved) => {
            sheet.paper_mm = resolved.paper_mm;
            sheet.landscape = resolved.landscape;
            sheet.paper_name = Some(resolved.name.clone());
            if matches!(choice, PaperChoice::Custom { .. }) || (choice == PaperChoice::Auto && paper_size(&resolved.name).is_none()) {
                sheet.issues.push(issue(Severity::Info, codes::PAPER_CUSTOM, "Frame uses a custom paper size.", Some(&sheet.sheet_id)));
            }
        }
        Err(mut error) => {
            let label = sheet.scale_label.clone().unwrap_or_else(|| match sheet.plot_scale {
                PlotScale::Fit => "fit".into(),
                PlotScale::Ratio { paper_mm, drawing_units } => format!("{paper_mm}:{drawing_units}"),
            });
            error.message = error.message.replacen("at selected scale;", &format!("at {label};"), 1);
            sheet.issues.push(error);
        }
    }
    Ok(())
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
        scale_label: None,
        paper_name: paper_size("A3").map(|paper| paper.name.into()),
        status: SheetStatus::NeedsPreview,
        issues: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

fn block_sheet(candidate: FrameCandidate, order: u32, reg: &FrameRegistry, unit_mm: f64) -> Sheet {
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
    if let PlotScale::Ratio { paper_mm, drawing_units } = &sheet.plot_scale {
        let ratio = drawing_units / paper_mm * unit_mm;
        if positive(ratio) {
            // Six significant digits keep detected INSERT scales readable without changing the stored ratio.
            sheet.scale_label = format!("{ratio:.5e}").parse::<f64>().ok().filter(|value| positive(*value)).map(|value| format!("1:{value}"));
        }
    }
    let [w, h] = candidate.local_mm;
    if [w, h].into_iter().all(positive) {
        sheet.landscape = w > h;
        let portrait = [w.min(h), w.max(h)];
        let media = template
            .into_iter()
            .flat_map(|t| t.media_mm.iter().take(MAX_SCAN_FRAMES))
            .map(|[w, h]| [w.min(*h), w.max(*h)])
            .find(|size| matches_size(*size, portrait));
        let choice = media.map_or(PaperChoice::Auto, |[w, h]| PaperChoice::Custom { w, h });
        match resolve_paper(&choice, [w, h], [0.0; 4]) {
            Ok(resolved) => {
                sheet.paper_mm = resolved.paper_mm;
                sheet.paper_name = Some(resolved.name);
            }
            Err(error) => {
                sheet.paper_mm = portrait;
                sheet.paper_name = None;
                sheet.issues.push(error);
            }
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

fn apply_override(sheet: &mut Sheet, settings: &SheetOverride, unit_mm: f64) -> Result<(), String> {
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
    if let Some(scale) = &settings.scale {
        sheet.plot_scale = parse_scale(scale, unit_mm)?;
        sheet.scale_label = Some(scale.clone());
        sheet.issues.retain(|i| i.code != codes::SCALE_NOT_SET);
    }
    if let Some(id) = &settings.sheet_id {
        sheet.sheet_id = id.clone();
    }
    Ok(())
}
