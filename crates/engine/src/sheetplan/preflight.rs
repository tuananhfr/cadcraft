use std::collections::BTreeMap;

use cadcraft_doc::{Drawing, Entity, EntityKind, MAX_BLOCK_DEPTH, Space, XrefStatus, entity_bounds};
use cadcraft_geom::{Bounds2, Mat3};

use super::codes::{self, issue};
use super::ids::{sha256_hex, sheet_key};
use super::outline::{Relation, relation};
use super::{AuditEntry, FrameRegistry, PreflightIssue, Severity, Sheet, SheetPlan, SheetSource, SheetStatus};

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

fn approved(entry: &PreflightIssue, key: &str, audit: &[AuditEntry]) -> bool {
    if !entry.approved_by_user {
        return false;
    }
    let fingerprint: String = sha256_hex(format!("{}|{key}|{}", entry.code, entry.message).as_bytes()).chars().take(12).collect();
    audit.iter().any(|record| record.action == "approve" && record.sheet_key == key && record.code == entry.code && record.fingerprint == fingerprint)
}

fn owned(code: &str) -> bool {
    matches!(code, codes::XREF_MISSING | codes::XREF_UNLOADED | codes::XREF_MISSING_OUTSIDE | codes::XREF_SCAN_LIMIT)
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
    let blocked = plan.issues.iter().any(|entry| entry.severity == Severity::Block);
    for sheet in &mut plan.sheets {
        sheet.status = if blocked { SheetStatus::Blocked } else { derive_status(sheet, &[]) };
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
    match &sheet.source {
        SheetSource::Block { block_handle, outline, .. } => {
            let space = cadcraft_doc::Handle::parse_hex(block_handle).and_then(|handle| d.space_of(handle));
            space.as_ref() == Some(&reference.space) && overlaps(reference.bounds, outline)
        }
        SheetSource::Window { outline, .. } => reference.space == Space::Model && overlaps(reference.bounds, outline),
        SheetSource::Layout { name } => {
            let Some(layout) = d.layout(name) else { return false };
            if reference.space == Space::Paper(name.clone()) {
                let page = &layout.page;
                let (w, h) = if page.landscape {
                    (page.width_mm.max(page.height_mm), page.width_mm.min(page.height_mm))
                } else {
                    (page.width_mm.min(page.height_mm), page.width_mm.max(page.height_mm))
                };
                return overlaps(reference.bounds, &[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]);
            }
            reference.space == Space::Model
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
                    let bounds = Bounds2::new(
                        center - cadcraft_geom::Vec2::new(half_width, half_height),
                        center + cadcraft_geom::Vec2::new(half_width, half_height),
                    );
                    overlaps(reference.bounds, &bounds.corners().map(|point| [point.x, point.y]))
                })
        }
    }
}
