use std::collections::{BTreeMap, BTreeSet, HashMap};

use cadcraft_doc::{Drawing, Entity, EntityKind, Handle, Insert, MAX_BLOCK_DEPTH, Space, dynblock, entity_bounds};
use cadcraft_geom::{Bounds2, Mat3, Vec2};

use super::codes::{self, issue};
use super::outline::{Relation, area, normalize_outline, points, rect_of, relation};
use super::{FrameRegistry, GeometryRule, PreflightIssue, Severity};

const MAX_SCAN_FRAMES: usize = 10_000;
const MAX_FRAME_LEAVES: usize = 200_000;
const MAX_SPACE_RECTS: usize = MAX_SCAN_FRAMES * 10;

#[derive(Clone, Debug, PartialEq)]
pub struct FrameCandidate {
    pub insert: Handle,
    pub effective_name: String,
    pub template_id: Option<String>,
    pub outline: Option<[[f64; 2]; 4]>,
    pub rotation_deg: f64,
    pub mirrored: bool,
    pub local_mm: [f64; 2],
    pub insert_scale: f64,
    pub attributes: BTreeMap<String, String>,
    pub issues: Vec<PreflightIssue>,
}

impl FrameCandidate {
    fn new(insert: Handle) -> Self {
        Self {
            insert,
            effective_name: String::new(),
            template_id: None,
            outline: None,
            rotation_deg: 0.0,
            mirrored: false,
            local_mm: [0.0; 2],
            insert_scale: 0.0,
            attributes: BTreeMap::new(),
            issues: Vec::new(),
        }
    }

    fn report(&mut self, severity: Severity, code: &str, message: &str) {
        if !self.issues.iter().any(|issue| issue.code == code) {
            self.issues.push(issue(severity, code, message, None));
        }
    }
}

pub(super) fn insert_hidden(d: &Drawing, entity: &Entity) -> Vec<Handle> {
    let EntityKind::Insert(insert) = &entity.kind else { return Vec::new() };
    if d.entity(entity.handle).is_some() {
        return dynblock::hidden_objects(d, entity.handle);
    }
    if d.foreign.objects.is_empty() {
        return Vec::new();
    }
    // Definition entities need an addressable reference for the dynamic visibility lookup.
    let mut context = Drawing { foreign: d.foreign.clone(), blocks: d.blocks.clone(), ..Drawing::default() };
    context.model.push(Entity::new(
        entity.handle,
        EntityKind::Insert(Insert {
            block: insert.block.clone(),
            insert: insert.insert,
            scale: insert.scale,
            rotation: insert.rotation,
            attribs: Vec::new(),
            cols: insert.cols,
            rows: insert.rows,
            col_spacing: insert.col_spacing,
            row_spacing: insert.row_spacing,
        }),
    ));
    dynblock::hidden_objects(&context, entity.handle)
}

#[derive(Clone)]
struct Rectangle {
    corners: [Vec2; 4],
    transform: Mat3,
    layer: String,
}

impl Rectangle {
    fn outline(&self) -> [[f64; 2]; 4] {
        normalize_outline(self.corners, self.transform.apply_vec(Vec2::Y)).0
    }
}

fn visible(d: &Drawing, e: &Entity) -> bool {
    !matches!(e.kind, EntityKind::AttDef(_))
        && e.common.visible
        && (e.common.layer == "0" || d.layer(&e.common.layer).is_none_or(cadcraft_doc::Layer::visible))
}

pub(super) fn rectangle(e: &Entity) -> Option<[Vec2; 4]> {
    match &e.kind {
        EntityKind::LwPolyline(p) if p.closed && matches!(p.vertices.len(), 4 | 5) => {
            if p.vertices.iter().any(|v| v.bulge != 0.0) {
                return None;
            }
            rect_of(&p.vertices.iter().map(|v| v.p).collect::<Vec<_>>())
        }
        EntityKind::Polyline3d(p) if p.closed && matches!(p.points.len(), 4 | 5) => {
            let z = p.points.first()?.z;
            if p.points.iter().any(|p| !p.is_finite() || (p.z - z).abs() > 1e-6) {
                return None;
            }
            rect_of(&p.points.iter().map(|p| p.xy()).collect::<Vec<_>>())
        }
        _ => None,
    }
}

fn valid_transform(m: Mat3) -> bool {
    [m.a, m.b, m.c, m.d, m.e, m.f, m.det()].into_iter().all(f64::is_finite) && m.inverse().is_some()
}

#[derive(Default)]
struct Walk {
    rectangles: Vec<Rectangle>,
    bounds: Bounds2,
    visited: usize,
    vertices: usize,
    incomplete: bool,
    bounds_incomplete: bool,
    minsert: bool,
}

impl Walk {
    fn insert(&mut self, d: &Drawing, ins: &Insert, hidden: BTreeSet<Handle>, parent: Mat3, depth: usize, layer: &str) {
        if depth >= MAX_BLOCK_DEPTH || ins.block.len() > MAX_FRAME_LEAVES {
            self.incomplete = true;
            return;
        }
        if u64::from(ins.cols) * u64::from(ins.rows) > 1 {
            self.minsert = true;
            return;
        }
        let Some(block) = d.block(&ins.block) else {
            self.incomplete = true;
            return;
        };
        let m = parent.then_before(ins.transform(block.base.xy()));
        if !valid_transform(m) {
            self.incomplete = true;
            return;
        }
        for e in block.entities.iter().take(MAX_FRAME_LEAVES + 1) {
            if self.incomplete || self.visited >= MAX_FRAME_LEAVES {
                self.incomplete = true;
                break;
            }
            self.visited += 1;
            if e.common.layer.len() > MAX_FRAME_LEAVES {
                self.incomplete = true;
                break;
            }
            if !visible(d, e) || hidden.contains(&e.handle) {
                continue;
            }
            if let EntityKind::Insert(inner) = &e.kind {
                if inner.block.len() > MAX_FRAME_LEAVES {
                    self.incomplete = true;
                    break;
                }
                let nested_hidden = insert_hidden(d, e);
                if nested_hidden.len() > MAX_FRAME_LEAVES {
                    self.incomplete = true;
                    break;
                }
                let inner_layer = if e.common.layer == "0" { layer } else { &e.common.layer };
                self.insert(d, inner, nested_hidden.into_iter().take(MAX_FRAME_LEAVES).collect(), m, depth + 1, inner_layer);
                continue;
            }
            if let Some(corners) = rectangle(e) {
                let corners = corners.map(|p| m.apply(p));
                if rect_of(&corners).is_some() {
                    let layer = if e.common.layer == "0" { layer.to_string() } else { e.common.layer.clone() };
                    self.rectangles.push(Rectangle { corners, transform: m, layer });
                }
            }
            self.leaf_bounds(d, e, m);
        }
        for a in ins.attribs.iter().take(MAX_FRAME_LEAVES + 1) {
            if self.incomplete || self.visited >= MAX_FRAME_LEAVES {
                self.incomplete = true;
                break;
            }
            self.visited += 1;
            if !a.invisible {
                if a.text.value.len() > MAX_FRAME_LEAVES {
                    self.incomplete = true;
                    break;
                }
                let attribute = Entity::new(Handle(0), EntityKind::Text(a.text.clone()));
                self.leaf_bounds(d, &attribute, parent);
            }
        }
    }

    fn leaf_bounds(&mut self, d: &Drawing, e: &Entity, m: Mat3) {
        let size = match &e.kind {
            EntityKind::LwPolyline(p) => p.vertices.len(),
            EntityKind::Polyline3d(p) => p.points.len(),
            EntityKind::Text(t) => t.value.len(),
            EntityKind::MText(t) => t.contents.len().saturating_add(t.columns.as_ref().map_or(0, |c| c.heights.len())),
            EntityKind::Line(_) | EntityKind::Point(_) | EntityKind::Circle(_) | EntityKind::Arc(_) | EntityKind::Ellipse(_) => 1,
            _ => {
                self.bounds_incomplete = true;
                return;
            }
        };
        if size > MAX_FRAME_LEAVES.saturating_sub(self.vertices) {
            self.incomplete = true;
            return;
        }
        self.vertices += size;
        let bounds = entity_bounds(d, e, 0);
        if !bounds.is_empty() {
            for p in bounds.corners().map(|p| m.apply(p)) {
                if p.is_finite() {
                    self.bounds.add(p);
                } else {
                    self.bounds_incomplete = true;
                }
            }
        }
    }
}

struct SpaceRectangles {
    rectangles: HashMap<Space, Vec<Rectangle>>,
    incomplete: bool,
}

impl SpaceRectangles {
    fn new(d: &Drawing) -> Self {
        let mut result = Self { rectangles: HashMap::new(), incomplete: false };
        let mut visited = 0;
        for (space, store) in std::iter::once((Space::Model, &d.model))
            .chain(d.layouts.iter().take(MAX_SPACE_RECTS + 1).map(|l| (Space::Paper(l.name.clone()), &l.entities)))
        {
            if visited >= MAX_SPACE_RECTS {
                result.incomplete = true;
                break;
            }
            let mut rectangles = Vec::new();
            for e in store.iter().take(MAX_SPACE_RECTS + 1) {
                if visited >= MAX_SPACE_RECTS {
                    result.incomplete = true;
                    break;
                }
                visited += 1;
                if d.is_visible(e)
                    && let Some(corners) = rectangle(e)
                {
                    rectangles.push(Rectangle { corners, transform: Mat3::IDENTITY, layer: e.common.layer.clone() });
                }
            }
            result.rectangles.insert(space, rectangles);
        }
        result.incomplete |= d.layouts.len() > MAX_SPACE_RECTS;
        result
    }
}

fn largest<'a>(rectangles: impl Iterator<Item = &'a Rectangle> + Clone, candidate: &mut FrameCandidate) -> Option<Rectangle> {
    let best = rectangles.clone().take(MAX_FRAME_LEAVES).max_by(|a, b| area(&a.outline()).total_cmp(&area(&b.outline())))?;
    let best_area = area(&best.outline());
    if rectangles.take(MAX_FRAME_LEAVES).any(|other| {
        (best_area - area(&other.outline())).abs() < best_area * 0.01 && relation(&best.outline(), &other.outline()) != Relation::Duplicate
    }) {
        candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Several distinct frame rectangles have nearly equal areas; select a Window.");
        return None;
    }
    Some(best.clone())
}

fn encloses(outline: &[[f64; 2]; 4], bounds: Bounds2) -> bool {
    let corners = points(outline);
    let [a, b, c, _] = corners;
    let tolerance = (b - a).len().max((c - b).len()) * 1e-6;
    corners.into_iter().zip(corners.into_iter().cycle().skip(1)).take(4).all(|(a, b)| {
        let edge = (b - a) / (b - a).len();
        bounds.corners().into_iter().all(|p| edge.cross(p - a) >= -tolerance)
    })
}

fn detect_one(d: &Drawing, h: Handle, reg: &FrameRegistry, spaces: &SpaceRectangles) -> FrameCandidate {
    let mut candidate = FrameCandidate::new(h);
    let Some(entity) = d.entity(h) else {
        candidate.report(Severity::Block, codes::FRAME_NOT_FOUND, "Frame INSERT was not found.");
        return candidate;
    };
    let EntityKind::Insert(ins) = &entity.kind else {
        candidate.report(Severity::Block, codes::FRAME_NOT_FOUND, "Selected entity is not a frame INSERT.");
        return candidate;
    };
    if ins.block.len() > MAX_FRAME_LEAVES {
        candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame block name exceeds the input limit.");
        return candidate;
    }
    candidate.effective_name = dynblock::effective_name(d, h).unwrap_or_else(|| ins.block.clone());
    let template = reg.find(&candidate.effective_name);
    candidate.template_id = template.map(|t| t.versioned_id());
    let mut attribute_bytes = 0_usize;
    for a in ins.attribs.iter().take(MAX_FRAME_LEAVES) {
        let bytes = a.tag.len().saturating_add(a.text.value.len());
        if bytes > MAX_FRAME_LEAVES.saturating_sub(attribute_bytes) {
            candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame attribute exceeds the input limit.");
            return candidate;
        }
        attribute_bytes += bytes;
        candidate.attributes.insert(a.tag.to_ascii_uppercase(), a.text.value.clone());
    }
    let mut walk = Walk::default();
    let hidden = dynblock::hidden_objects(d, h);
    if hidden.len() > MAX_FRAME_LEAVES {
        walk.incomplete = true;
    }
    walk.insert(d, ins, hidden.into_iter().take(MAX_FRAME_LEAVES).collect(), Mat3::IDENTITY, 0, &entity.common.layer);
    if walk.minsert {
        candidate.report(Severity::Block, codes::FRAME_MINSERT, "MINSERT arrays require individual frame references.");
        return candidate;
    }
    if walk.incomplete {
        candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Invalid geometry or frame traversal limit reached; select a Window.");
        return candidate;
    }
    let root = ins.transform(d.block(&ins.block).map_or(Vec2::ZERO, |b| b.base.xy()));
    let mut chosen = None;
    if let Some(template) = template {
        for rule in template.geometry.iter().take(MAX_FRAME_LEAVES) {
            chosen = match rule {
                GeometryRule::LargestClosedRect => largest(walk.rectangles.iter(), &mut candidate),
                GeometryRule::LayerRect { layer } => largest(walk.rectangles.iter().filter(|r| r.layer.eq_ignore_ascii_case(layer)), &mut candidate),
                GeometryRule::LocalRect { min: [x0, y0], max: [x1, y1] } => {
                    let corners = [Vec2::new(*x0, *y0), Vec2::new(*x1, *y0), Vec2::new(*x1, *y1), Vec2::new(*x0, *y1)];
                    let corners = corners.map(|p| root.apply(p));
                    rect_of(&corners).map(|corners| Rectangle { corners, transform: root, layer: String::new() })
                }
                GeometryRule::EnclosingRect => {
                    if spaces.incomplete || walk.bounds_incomplete || walk.bounds.is_empty() {
                        None
                    } else {
                        d.space_of(h)
                            .and_then(|sp| spaces.rectangles.get(&sp))
                            .into_iter()
                            .flatten()
                            .take(MAX_SPACE_RECTS)
                            .filter(|r| encloses(&r.outline(), walk.bounds))
                            .min_by(|a, b| area(&a.outline()).total_cmp(&area(&b.outline())))
                            .cloned()
                            .map(|mut rect| {
                                rect.transform = root;
                                rect
                            })
                    }
                }
            };
            if chosen.is_some() || candidate.issues.iter().any(|i| i.severity == Severity::Block) {
                break;
            }
        }
    }
    let Some(rect) = chosen else {
        candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "No unique frame rectangle was found; select a Window.");
        return candidate;
    };
    let m = rect.transform;
    let sx = m.apply_vec(Vec2::X).len();
    let sy = m.apply_vec(Vec2::Y).len();
    let (outline, rotation) = normalize_outline(rect.corners, m.apply_vec(Vec2::Y));
    let [a, b, c, _] = points(&outline);
    let Some(inverse) = m.inverse() else {
        candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame transform is singular; select a Window.");
        return candidate;
    };
    candidate.local_mm = [inverse.apply_vec(b - a).len(), inverse.apply_vec(c - b).len()];
    candidate.insert_scale = m.scale_factor();
    if !candidate.local_mm.into_iter().all(|v| v.is_finite() && v > 0.0) {
        candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame dimensions are invalid; select a Window.");
        return candidate;
    }
    candidate.outline = Some(outline);
    candidate.rotation_deg = rotation;
    candidate.mirrored = m.is_mirroring();
    if candidate.mirrored {
        candidate.report(Severity::Warn, codes::FRAME_MIRRORED, "Frame INSERT is mirrored.");
    }
    if (sx - sy).abs() > sx.max(sy) * 1e-6 {
        candidate.report(Severity::Warn, codes::FRAME_NONUNIFORM, "Frame INSERT has nonuniform axis scales.");
    }
    candidate
}

fn reference_data_within_limits(d: &Drawing, reg: &FrameRegistry) -> bool {
    if d.blocks.len() > MAX_FRAME_LEAVES
        || d.layers.len() > MAX_FRAME_LEAVES
        || d.layouts.len() > MAX_SPACE_RECTS
        || d.foreign.objects.len() > MAX_FRAME_LEAVES
        || d.foreign.records.len() > MAX_FRAME_LEAVES
        || reg.templates.len() > super::MAX_TEMPLATES
    {
        return false;
    }
    let mut tags = 0_usize;
    for object in d.foreign.objects.iter().take(MAX_FRAME_LEAVES) {
        if object.tags.len() > MAX_FRAME_LEAVES.saturating_sub(tags) {
            return false;
        }
        tags += object.tags.len();
    }
    reg.templates.iter().take(super::MAX_TEMPLATES).all(|t| t.block_names.len() <= 64 && t.geometry.len() <= MAX_FRAME_LEAVES)
}

pub fn detect_frames(d: &Drawing, inserts: &[Handle], reg: &FrameRegistry) -> Vec<FrameCandidate> {
    if !reference_data_within_limits(d, reg) {
        return inserts
            .iter()
            .map(|h| {
                let mut candidate = FrameCandidate::new(*h);
                candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame reference data exceeds the input limit.");
                candidate
            })
            .collect();
    }
    let spaces = SpaceRectangles::new(d);
    // The output loop preserves every slot even when callers exceed the scan limit.
    let mut candidates: Vec<_> = inserts
        .iter()
        .enumerate()
        .map(|(i, h)| {
            if i < MAX_SCAN_FRAMES {
                detect_one(d, *h, reg, &spaces)
            } else {
                let mut candidate = FrameCandidate::new(*h);
                candidate.report(Severity::Block, codes::FRAME_AMBIGUOUS, "Frame scan exceeds 10000 references.");
                candidate
            }
        })
        .collect();
    for i in 0..candidates.len().min(MAX_SCAN_FRAMES) {
        let Some(a) = candidates.get(i).and_then(|c| c.outline) else { continue };
        let bounds_a = Bounds2::from_points(points(&a));
        let Some(tail) = candidates.get_mut(i..) else { continue };
        let Some((first, rest)) = tail.split_first_mut() else { continue };
        for second in rest.iter_mut().take(MAX_SCAN_FRAMES.saturating_sub(i + 1)) {
            let Some(b) = second.outline else { continue };
            if !bounds_a.intersects(&Bounds2::from_points(points(&b))) {
                continue;
            }
            match relation(&a, &b) {
                Relation::Duplicate => second.report(Severity::Warn, codes::FRAME_DUPLICATE, "Frame duplicates an earlier selected frame."),
                Relation::AInsideB => first.report(Severity::Warn, codes::FRAME_NESTED, "Frame is inside another selected frame."),
                Relation::BInsideA => second.report(Severity::Warn, codes::FRAME_NESTED, "Frame is inside another selected frame."),
                Relation::Overlap => {
                    first.report(Severity::Warn, codes::FRAME_OVERLAP, "Frame overlaps another selected frame.");
                    second.report(Severity::Warn, codes::FRAME_OVERLAP, "Frame overlaps another selected frame.");
                }
                Relation::Disjoint => {}
            }
        }
    }
    candidates
}

pub fn is_frame_candidate(d: &Drawing, h: Handle, reg: &FrameRegistry) -> bool {
    detect_frames(d, &[h], reg).first().is_some_and(|c| c.outline.is_some())
}
