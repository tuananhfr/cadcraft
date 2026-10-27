use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use cadcraft_doc::{Drawing, Entity, EntityKind, MAX_BLOCK_DEPTH, Space, XrefStatus, entity_bounds};
use cadcraft_geom::{Bounds2, Mat3};

use super::approval::{approved, refresh_status};
use super::codes::{self, issue};
use super::ids::sheet_key;
use super::outline::{Relation, relation};
use super::{AuditEntry, FrameRegistry, Severity, Sheet, SheetPlan, SheetSource, SheetStatus, TemplateStatus};

pub struct FontStatus {
    pub style: String,
    pub font: String,
    pub state: cadcraft_fonts::FontState,
    pub with: Option<String>,
    pub mapped: bool,
}

pub struct PreflightContext<'a> {
    pub registry: &'a FrameRegistry,
    pub device: Option<&'a cadcraft_doc::PlotterConfig>,
    pub device_missing: bool,
    pub revisions: &'a BTreeMap<String, String>,
    pub fonts: &'a [FontStatus],
    pub web: bool,
}

pub fn verdict(sheet: &Sheet) -> Severity {
    sheet.issues.iter().map(|entry| entry.severity).max().unwrap_or(Severity::Pass)
}

pub fn derive_status(sheet: &Sheet, audit: &[AuditEntry]) -> SheetStatus {
    if verdict(sheet) == Severity::Block {
        return SheetStatus::Blocked;
    }
    let key = sheet_key(sheet);
    if sheet.issues.iter().any(|entry| entry.severity == Severity::Warn && !approved(entry, &key, audit)) {
        SheetStatus::NeedsPreview
    } else {
        SheetStatus::Ready
    }
}

fn owned(code: &str) -> bool {
    matches!(
        code,
        codes::XREF_MISSING
            | codes::XREF_UNLOADED
            | codes::XREF_MISSING_OUTSIDE
            | codes::XREF_SCAN_LIMIT
            | codes::FONT_FALLBACK
            | codes::FONT_MISSING
            | codes::FONT_MAPPED
            | codes::SHEET_ID_DUPLICATE
            | codes::REVISION_OLD
            | codes::REVISION_NEWER
            | codes::TEMPLATE_SUPERSEDED
            | codes::PLOT_STYLE
            | codes::PLOT_PROFILE_MISSING
            | codes::FILE_RENAMED
            | codes::DEVICE_LIMIT_NEAR
            | codes::DEVICE_LIMIT_EXCEEDED
            | codes::DEVICE_UNAVAILABLE
    )
}

pub fn preflight(d: &Drawing, plan: &mut SheetPlan, ctx: &PreflightContext<'_>) {
    plan.issues.retain(|entry| !owned(&entry.code));
    for sheet in &mut plan.sheets {
        sheet.issues.retain(|entry| !owned(&entry.code));
    }
    if d.blocks.values().any(|block| block.xref_path.is_some()) {
        let mut walk = Xrefs::default();
        for (space, store) in std::iter::once((Space::Model, &d.model))
            .chain(d.layouts.iter().take(MAX_XREF_ENTITIES + 1).map(|layout| (Space::Paper(layout.name.clone()), &layout.entities)))
        {
            for entity in store.iter().take(MAX_XREF_ENTITIES + 1) {
                walk.visit(d, entity, &space, Mat3::IDENTITY, 0);
                if walk.incomplete {
                    break;
                }
            }
            if walk.incomplete {
                break;
            }
        }
        walk.incomplete |= d.layouts.len() > MAX_XREF_ENTITIES;
        for reference in walk.references {
            let (severity, code) = match reference.status {
                XrefStatus::NotFound | XrefStatus::Unresolved | XrefStatus::NotLoaded => (Severity::Block, codes::XREF_MISSING),
                XrefStatus::Unloaded => (Severity::Warn, codes::XREF_UNLOADED),
                XrefStatus::Loaded => continue,
            };
            let mut message = format!("External reference `{}` at `{}` is {}.", reference.name, reference.path, reference.status.label());
            if ctx.web && severity == Severity::Block {
                message.push_str(" external references are not loaded on the web.");
            }
            let mut essential = false;
            for sheet in &mut plan.sheets {
                if intersects_sheet(d, sheet, &reference) {
                    essential = true;
                    sheet.issues.push(issue(severity, code, &message, Some(&sheet.sheet_id)));
                }
            }
            if !essential && severity == Severity::Block && !walk.incomplete {
                plan.issues.push(issue(Severity::Info, codes::XREF_MISSING_OUTSIDE, message, None));
            }
        }
        if walk.incomplete {
            // A truncated traversal cannot prove that the remaining sheets have all their references.
            for sheet in &mut plan.sheets {
                sheet.issues.push(issue(
                    Severity::Block,
                    codes::XREF_SCAN_LIMIT,
                    "External reference traversal exceeded its safe limits.",
                    Some(&sheet.sheet_id),
                ));
            }
        }
    }
    check_devices(plan, ctx.device, ctx.device_missing, ctx.web);
    check_metadata(d, plan, ctx);
    refresh_status(plan);
}

pub(crate) fn check_devices(plan: &mut SheetPlan, device: Option<&cadcraft_doc::PlotterConfig>, missing: bool, web: bool) {
    use super::split::{DEFAULT_OVERLAP_MM, device_limits, propose_split, split_approved};

    if plan.device.is_none() {
        plan.device = device.map(|device| device.name.clone());
    }
    let resolved = plan.device.as_deref().and_then(crate::cmd::plotters::find_device);
    let device = device.or(resolved.as_ref());
    let requested = plan.device.is_some() || device.is_some() || missing;
    let pdf = device.is_some_and(|device| device.name.eq_ignore_ascii_case("CADCraft PDF"));
    let unavailable = requested && (missing || device.is_none() || (web && !pdf));
    let limits = device.and_then(device_limits);
    plan.issues.retain(|entry| !matches!(entry.code.as_str(), codes::DEVICE_LIMIT_NEAR | codes::DEVICE_LIMIT_EXCEEDED | codes::DEVICE_UNAVAILABLE));
    for sheet in &mut plan.sheets {
        sheet
            .issues
            .retain(|entry| !matches!(entry.code.as_str(), codes::DEVICE_LIMIT_NEAR | codes::DEVICE_LIMIT_EXCEEDED | codes::DEVICE_UNAVAILABLE));
        if unavailable || (requested && limits.is_none()) {
            sheet.split = None;
            let message = if web {
                "Printer devices are only available in the desktop app; plotting to PDF."
            } else {
                "The requested device is unavailable; plotting to PDF."
            };
            sheet.issues.push(issue(Severity::Warn, codes::DEVICE_UNAVAILABLE, message, Some(&sheet.sheet_id)));
            continue;
        }
        let Some((max_long, max_short)) = limits else {
            sheet.split = None;
            continue;
        };
        let [a, b] = sheet.paper_mm;
        if a.max(b) > max_long || a.min(b) > max_short {
            if split_approved(sheet, &plan.audit, device) {
                continue;
            }
            let overlap = sheet.split.as_ref().map_or(DEFAULT_OVERLAP_MM, |split| split.overlap_mm);
            sheet.split = propose_split(sheet, max_long, max_short, overlap);
            sheet.issues.push(issue(
                Severity::Block,
                codes::DEVICE_LIMIT_EXCEEDED,
                format!("Paper {} x {} mm exceeds device limits {max_short} x {max_long} mm; approve a split or choose PDF.", a.min(b), a.max(b)),
                Some(&sheet.sheet_id),
            ));
        } else {
            sheet.split = None;
            if a.max(b) >= max_long * 0.95 || a.min(b) >= max_short * 0.95 {
                sheet.issues.push(issue(
                    Severity::Warn,
                    codes::DEVICE_LIMIT_NEAR,
                    format!("Paper {} x {} mm is at least 95% of device limits {max_short} x {max_long} mm.", a.min(b), a.max(b)),
                    Some(&sheet.sheet_id),
                ));
            }
        }
    }
}

pub fn rev_cmp(mut a: &str, mut b: &str) -> Ordering {
    fn segment(text: &str) -> (&str, &str, bool) {
        let numeric = text.starts_with(|ch: char| ch.is_ascii_digit());
        let end = text.find(|ch: char| ch.is_ascii_digit() != numeric).unwrap_or(text.len());
        let (part, rest) = text.split_at(end);
        (part, rest, numeric)
    }
    while !a.is_empty() && !b.is_empty() {
        let (left, rest_a, left_numeric) = segment(a);
        let (right, rest_b, right_numeric) = segment(b);
        let order = if left_numeric && right_numeric {
            // Length comparison avoids overflow for revision numbers of arbitrary size.
            let left = left.trim_start_matches('0');
            let right = right.trim_start_matches('0');
            left.len().cmp(&right.len()).then_with(|| left.cmp(right))
        } else {
            left.chars().flat_map(char::to_lowercase).cmp(right.chars().flat_map(char::to_lowercase))
        };
        if order != Ordering::Equal {
            return order;
        }
        a = rest_a;
        b = rest_b;
    }
    a.is_empty().cmp(&b.is_empty()).reverse()
}

pub(crate) fn sheet_revision<'a>(sheet: &'a Sheet, registry: &FrameRegistry) -> Option<&'a str> {
    let template = sheet.frame_template_id.as_deref().and_then(|id| super::registry_store::resolve(registry, id))?;
    template.attributes.revision.iter().find_map(|tag| {
        sheet.attributes.iter().find(|(key, value)| key.eq_ignore_ascii_case(tag) && !value.trim().is_empty()).map(|(_, value)| value.trim())
    })
}

fn check_metadata(d: &Drawing, plan: &mut SheetPlan, ctx: &PreflightContext<'_>) {
    for font in ctx.fonts {
        let replacement = font.with.as_deref().unwrap_or("the built-in stroke font");
        let message = format!("{} is drawn with {replacement}", font.font);
        if font.mapped {
            plan.issues.push(issue(Severity::Info, codes::FONT_MAPPED, message, None));
        } else if font.state == cadcraft_fonts::FontState::Missing {
            let code = if font.with.is_some() { codes::FONT_FALLBACK } else { codes::FONT_MISSING };
            plan.issues.push(issue(Severity::Warn, code, message, None));
        }
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut latest = BTreeMap::<String, String>::new();
    for sheet in &plan.sheets {
        let count = counts.entry(&sheet.sheet_id).or_default();
        *count = count.saturating_add(1);
        if let Some(revision) = sheet_revision(sheet, ctx.registry) {
            let highest = latest.entry(sheet.sheet_id.clone()).or_insert_with(|| revision.to_string());
            if rev_cmp(revision, highest) == Ordering::Greater {
                *highest = revision.to_string();
            }
        }
    }
    let duplicates: BTreeSet<String> = counts.into_iter().filter(|(_, count)| *count > 1).map(|(id, _)| id.to_string()).collect();
    let mut plot_styles = BTreeSet::new();
    for sheet in &mut plan.sheets {
        if duplicates.contains(&sheet.sheet_id) {
            sheet.issues.push(issue(
                Severity::Warn,
                codes::SHEET_ID_DUPLICATE,
                format!("Sheet id `{}` is used by more than one sheet.", sheet.sheet_id),
                Some(&sheet.sheet_id),
            ));
        }
        let template = sheet.frame_template_id.as_ref().and_then(|id| super::registry_store::resolve(ctx.registry, id));
        if let Some(template) = template
            && (template.status == TemplateStatus::Superseded || template.superseded_by.is_some())
        {
            let successor = template.superseded_by.as_deref().unwrap_or("an unspecified successor");
            sheet.issues.push(issue(
                Severity::Warn,
                codes::TEMPLATE_SUPERSEDED,
                format!("Template `{}` is superseded by `{successor}`.", template.id),
                Some(&sheet.sheet_id),
            ));
        }
        let revision = sheet_revision(sheet, ctx.registry);
        let old_revision = revision.and_then(|revision| {
            latest.get(&sheet.sheet_id).filter(|highest| rev_cmp(revision, highest) == Ordering::Less).map(|highest| (revision, highest))
        });
        if let Some((revision, highest)) = old_revision {
            sheet.issues.push(issue(
                Severity::Warn,
                codes::REVISION_OLD,
                format!("Revision `{revision}` is older than highest revision `{highest}` for sheet id `{}` in this plan.", sheet.sheet_id),
                Some(&sheet.sheet_id),
            ));
        } else if template.is_some() {
            if let Some(expected) = ctx.revisions.get(&sheet.sheet_id) {
                let revision = revision.unwrap_or("");
                let (severity, code) = match rev_cmp(revision, expected.trim()) {
                    Ordering::Less => (Severity::Warn, codes::REVISION_OLD),
                    Ordering::Greater => (Severity::Info, codes::REVISION_NEWER),
                    Ordering::Equal => (Severity::Pass, ""),
                };
                if severity != Severity::Pass {
                    sheet.issues.push(issue(
                        severity,
                        code,
                        format!("Revision `{revision}` compared with registered revision `{expected}`."),
                        Some(&sheet.sheet_id),
                    ));
                }
            }
        } else if let Some(expected) = ctx.revisions.get(&sheet.sheet_id)
            && rev_cmp("", expected.trim()) == Ordering::Less
        {
            sheet.issues.push(issue(
                Severity::Warn,
                codes::REVISION_OLD,
                format!("Revision is missing; registered revision is `{expected}`."),
                Some(&sheet.sheet_id),
            ));
        }
        let space = sheet_space(d, sheet);
        let table =
            space.as_ref().and_then(Space::layout).and_then(|name| d.layout(name)).map(|layout| layout.page.plot_style_table.as_str()).unwrap_or("");
        plot_styles.insert(if cadcraft_doc::no_plot_style_table(table) { "colors as displayed" } else { table });
    }
    if plot_styles.is_empty() {
        plot_styles.insert("colors as displayed");
    }
    for table in plot_styles {
        plan.issues.push(issue(Severity::Info, codes::PLOT_STYLE, table, None));
    }
}

/// Blocks the plan when the chosen plot style table is neither the drawing's nor a built-in one.
pub(crate) fn check_plot_style_table(d: &Drawing, plan: &mut SheetPlan, table: Option<&str>) {
    let before = plan.issues.len();
    plan.issues.retain(|entry| entry.code != codes::PLOT_PROFILE_MISSING);
    let mut changed = plan.issues.len() != before;
    if let Some(table) = table.filter(|table| !cadcraft_doc::no_plot_style_table(table))
        && cadcraft_doc::plot_style_table(d, table).is_none()
    {
        plan.issues.push(issue(
            Severity::Block,
            codes::PLOT_PROFILE_MISSING,
            format!("Plot style table `{table}` is not in this drawing or built in; choose another table."),
            None,
        ));
        changed = true;
    }
    // Statuses such as Done from an earlier publish stay unless the verdict actually changed.
    if changed {
        refresh_status(plan);
    }
}

fn sheet_space(d: &Drawing, sheet: &Sheet) -> Option<Space> {
    match &sheet.source {
        SheetSource::Block { block_handle, .. } => cadcraft_doc::Handle::parse_hex(block_handle).and_then(|handle| d.space_of(handle)),
        SheetSource::Window { .. } => Some(Space::Model),
        SheetSource::Layout { name } => Some(Space::Paper(name.clone())),
    }
}

const MAX_XREF_ENTITIES: usize = 200_000;

struct Reference {
    space: Space,
    bounds: Bounds2,
    name: String,
    path: String,
    status: XrefStatus,
}

#[derive(Default)]
struct Xrefs {
    references: Vec<Reference>,
    visited: usize,
    incomplete: bool,
}

impl Xrefs {
    fn visit(&mut self, d: &Drawing, entity: &Entity, space: &Space, parent: Mat3, depth: usize) {
        if self.visited >= MAX_XREF_ENTITIES {
            self.incomplete = true;
            return;
        }
        self.visited += 1;
        if !d.is_visible(entity) {
            return;
        }
        let EntityKind::Insert(insert) = &entity.kind else { return };
        if depth >= MAX_BLOCK_DEPTH {
            self.incomplete = true;
            return;
        }
        let Some(block) = d.block(&insert.block) else { return };
        if let Some(path) = &block.xref_path {
            if path.len() > MAX_XREF_ENTITIES || block.name.len() > MAX_XREF_ENTITIES {
                self.incomplete = true;
                return;
            }
            let bounds = entity_bounds(d, entity, depth);
            let bounds = if bounds.is_empty() {
                // A missing xref has no geometry; its insertion point is where the drawing expects it.
                let at = parent.apply(insert.insert.xy());
                let e = 1e-6 * at.x.abs().max(at.y.abs()).max(1.0);
                let eps = cadcraft_geom::Vec2::new(e, e);
                Bounds2::new(at - eps, at + eps)
            } else {
                Bounds2::from_points(bounds.corners().map(|point| parent.apply(point)))
            };
            self.references.push(Reference { space: space.clone(), bounds, name: block.name.clone(), path: path.clone(), status: block.xref_status });
            if block.xref_status != XrefStatus::Loaded {
                return;
            }
        }
        let transform = parent.then_before(insert.transform(block.base.xy()));
        if ![transform.a, transform.b, transform.c, transform.d, transform.e, transform.f].into_iter().all(f64::is_finite) {
            self.incomplete = true;
            return;
        }
        let hidden = super::detect::insert_hidden(d, entity);
        if hidden.len() > MAX_XREF_ENTITIES {
            self.incomplete = true;
            return;
        }
        for child in block.entities.iter().take(MAX_XREF_ENTITIES + 1) {
            if self.visited >= MAX_XREF_ENTITIES {
                self.incomplete = true;
                break;
            }
            if hidden.binary_search(&child.handle).is_err() {
                self.visit(d, child, space, transform, depth + 1);
            } else {
                self.visited += 1;
            }
            if self.incomplete {
                break;
            }
        }
    }
}

fn overlaps(bounds: Bounds2, outline: &[[f64; 2]; 4]) -> bool {
    !bounds.is_empty() && relation(&bounds.corners().map(|point| [point.x, point.y]), outline) != Relation::Disjoint
}

fn intersects_sheet(d: &Drawing, sheet: &Sheet, reference: &Reference) -> bool {
    intersects_bounds(d, sheet, &reference.space, reference.bounds)
}

fn intersects_bounds(d: &Drawing, sheet: &Sheet, space: &Space, bounds: Bounds2) -> bool {
    match &sheet.source {
        SheetSource::Block { block_handle, outline, .. } => {
            let owner = cadcraft_doc::Handle::parse_hex(block_handle).and_then(|handle| d.space_of(handle));
            owner.as_ref() == Some(space) && overlaps(bounds, outline)
        }
        SheetSource::Window { outline, .. } => *space == Space::Model && overlaps(bounds, outline),
        SheetSource::Layout { name } => {
            let Some(layout) = d.layout(name) else { return false };
            if *space == Space::Paper(name.clone()) {
                let page = &layout.page;
                let (w, h) = if page.landscape {
                    (page.width_mm.max(page.height_mm), page.width_mm.min(page.height_mm))
                } else {
                    (page.width_mm.min(page.height_mm), page.width_mm.max(page.height_mm))
                };
                return overlaps(bounds, &[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]);
            }
            *space == Space::Model
                && layout.entities.iter().take(MAX_XREF_ENTITIES).any(|entity| {
                    let EntityKind::Viewport(viewport) = &entity.kind else { return false };
                    if !viewport.on || !d.is_visible(entity) {
                        return false;
                    }
                    let half_height = viewport.view_height / 2.0;
                    let half_width = half_height * viewport.width / viewport.height;
                    if ![half_width, half_height].into_iter().all(|value| value.is_finite() && value > 0.0) {
                        return false;
                    }
                    let center = viewport.view_center;
                    let view = Bounds2::new(
                        center - cadcraft_geom::Vec2::new(half_width, half_height),
                        center + cadcraft_geom::Vec2::new(half_width, half_height),
                    );
                    overlaps(bounds, &view.corners().map(|point| [point.x, point.y]))
                })
        }
    }
}

pub(crate) fn styles_on_sheets(d: &Drawing, plan: &SheetPlan) -> Result<BTreeSet<String>, String> {
    let mut walk = UsedStyles { names: BTreeSet::new(), remaining: MAX_XREF_ENTITIES };
    if plan.sheets.is_empty() {
        return Ok(walk.names);
    }
    if d.layouts.len() > MAX_XREF_ENTITIES {
        return Err("Font traversal exceeded its safe layout limit.".into());
    }
    for (space, store) in
        std::iter::once((Space::Model, &d.model)).chain(d.layouts.iter().map(|layout| (Space::Paper(layout.name.clone()), &layout.entities)))
    {
        for entity in store.iter() {
            walk.visit(d, plan, entity, &space, Mat3::IDENTITY, 0)?;
        }
    }
    Ok(walk.names)
}

struct UsedStyles {
    names: BTreeSet<String>,
    remaining: usize,
}

impl UsedStyles {
    fn consume(&mut self) -> Result<(), String> {
        self.remaining = self.remaining.checked_sub(1).ok_or_else(|| "Font traversal exceeded its safe entity limit.".to_string())?;
        Ok(())
    }

    fn add(&mut self, name: &str) {
        self.names.insert(name.to_ascii_lowercase());
    }

    fn visit(&mut self, d: &Drawing, plan: &SheetPlan, entity: &Entity, space: &Space, parent: Mat3, depth: usize) -> Result<(), String> {
        self.consume()?;
        if !d.is_visible(entity) {
            return Ok(());
        }
        let local = entity_bounds(d, entity, depth);
        let bounds = Bounds2::from_points(local.corners().map(|point| parent.apply(point)));
        if !local.is_empty() && plan.sheets.iter().any(|sheet| intersects_bounds(d, sheet, space, bounds)) {
            match &entity.kind {
                EntityKind::Text(text) => self.add(&text.style),
                EntityKind::MText(text) => self.add(&text.style),
                EntityKind::Insert(insert) => {
                    for attribute in &insert.attribs {
                        self.consume()?;
                        let mode = d.header.i64("ATTMODE", 1);
                        if mode != 0 && (!attribute.invisible || mode == 2) {
                            if let Some(text) = attribute.multiline_text() {
                                self.add(&text.style);
                            } else {
                                self.add(&attribute.text.style);
                            }
                        }
                    }
                }
                EntityKind::Dimension(dimension) => {
                    if dimension.block.as_ref().and_then(|name| d.block(name)).is_none() || depth >= MAX_BLOCK_DEPTH {
                        let style = d.dim_style(&dimension.style).cloned().unwrap_or_default().with_overrides(&dimension.overrides);
                        self.add(&style.text_style);
                    }
                }
                EntityKind::Tolerance(tolerance) => {
                    self.add(&d.dim_style(&tolerance.style).cloned().unwrap_or_default().text_style);
                }
                EntityKind::MLeader(leader) => {
                    if let Some(text) = &leader.text {
                        self.add(&text.style);
                    }
                }
                EntityKind::Table(table) => {
                    let style = d.table_styles.iter().find(|style| style.name.eq_ignore_ascii_case(&table.style)).cloned().unwrap_or_default();
                    for (row, cells) in table.cells.iter().enumerate() {
                        self.consume()?;
                        for cell in cells {
                            self.consume()?;
                            let name = if cell.style.trim().is_empty() { table.row_style(row) } else { &cell.style };
                            self.add(&style.cell_style(name).text_style);
                        }
                    }
                }
                _ => {}
            }
        }
        if let EntityKind::Dimension(dimension) = &entity.kind
            && let Some(block) = dimension.block.as_ref().and_then(|name| d.block(name))
            && depth < MAX_BLOCK_DEPTH
        {
            for child in block.entities.iter() {
                self.visit(d, plan, child, space, parent, depth + 1)?;
            }
        }
        let insert = match &entity.kind {
            EntityKind::Insert(insert) => Some(insert),
            EntityKind::MLeader(leader) => leader.block.as_ref(),
            _ => None,
        };
        if let Some(insert) = insert {
            let Some(block) = d.block(&insert.block) else { return Ok(()) };
            if block.xref_path.is_some() && block.xref_status != XrefStatus::Loaded {
                return Ok(());
            }
            if depth >= MAX_BLOCK_DEPTH {
                // The renderer also omits block contents beyond this depth.
                return Ok(());
            }
            let hidden = super::detect::insert_hidden(d, entity);
            if hidden.len() > MAX_XREF_ENTITIES {
                return Err("Font traversal exceeded its safe visibility limit.".into());
            }
            for row in 0..insert.rows.clamp(1, 10_000) {
                for column in 0..insert.cols.clamp(1, 10_000) {
                    self.consume()?;
                    let offset =
                        cadcraft_geom::Vec2::new(insert.col_spacing * f64::from(column), insert.row_spacing * f64::from(row)).rotate(insert.rotation);
                    let transform = parent.then_before(Mat3::translate(offset).then_before(insert.transform(block.base.xy())));
                    if ![transform.a, transform.b, transform.c, transform.d, transform.e, transform.f].into_iter().all(f64::is_finite) {
                        return Err("Font traversal found a non-finite block transform.".into());
                    }
                    for child in block.entities.iter() {
                        if hidden.binary_search(&child.handle).is_ok() || matches!(&child.kind, EntityKind::AttDef(_)) {
                            self.consume()?;
                        } else {
                            self.visit(d, plan, child, space, transform, depth + 1)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
