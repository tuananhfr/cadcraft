//! Drawing audit (UBCHECK): duplicate ids, unreadable or legacy boxes, boxes moved by hand, overlaps, boundary, zone and
//! clearance violations, and layouts whose slab geometry changed since the fill.

mod containment;
mod moved;
mod overlap;
mod resolve;
mod work;
mod zone_grid;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::error::UbotError;
use crate::fill::{ExistingInstance, LayoutRecord, MAX_CLEARANCE_MM, MAX_VERTICES, ZoneInput, ZoneReach, geometry_hash, handle_key};
use crate::geometry::RingInput;
use crate::model::RulePack;
use containment::{LayoutCtx, Zones, check_layout, check_zones};
use resolve::{Lookup, Placed, resolve};
use work::Work;

const MAX_INSTANCES: usize = 100_000;
const MAX_LAYOUTS: usize = 1_000;
const MAX_ZONES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IssueCode {
    DuplicateId,
    DuplicateLayout,
    ManualMoved,
    XdataInvalid,
    UnknownProduct,
    Overlap,
    OutsideBoundary,
    InZone,
    Clearance,
    StaleLayout,
    LegacyBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// `at` is a drawing-unit point to zoom to; `handle` is the entity the issue is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub code: IssueCode,
    pub severity: Severity,
    #[serde(default)]
    pub handle: Option<String>,
    pub at: [f64; 2],
    pub message: String,
}

/// A slab boundary with the layout stored on it; rings are in drawing units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutInput {
    pub layout_id: String,
    pub boundary_handle: String,
    pub boundary: RingInput,
    #[serde(default)]
    pub holes: Vec<RingInput>,
    #[serde(default)]
    pub stored_geometry_hash: Option<String>,
    /// Gives the grid cell positions that `ManualMoved` is judged against.
    #[serde(default)]
    pub layout: Option<LayoutRecord>,
    /// `None` falls back to the layout's rulepack.
    #[serde(default)]
    pub edge_clearance_mm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckInstance {
    pub base: ExistingInstance,
    /// False when the box's `LPC_UBOT` xdata could not be read.
    pub xdata_valid: bool,
    /// A block from the legacy mapping rather than a tagged box.
    pub legacy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRequest {
    /// Millimetres per drawing unit.
    pub unit_scale: f64,
    pub instances: Vec<CheckInstance>,
    #[serde(default)]
    pub layouts: Vec<LayoutInput>,
    #[serde(default)]
    pub zones: Vec<ZoneInput>,
}

fn issue(code: IssueCode, severity: Severity, p: &Placed<'_>, message: String) -> Issue {
    Issue { code, severity, handle: Some(p.inst.base.handle.clone()), at: p.at, message }
}

fn clearance_mm(v: f64, name: &str) -> Result<f64, UbotError> {
    if !v.is_finite() || v < 0.0 {
        return Err(UbotError::BadInput(format!("{name} must be a finite number >= 0")));
    }
    if v > MAX_CLEARANCE_MM {
        return Err(UbotError::BadInput(format!("{name} must be <= {MAX_CLEARANCE_MM} mm")));
    }
    Ok(v)
}

/// The rulepack named by the stored layout (`"<id>@<version>"`), else the catalog's first.
fn rulepack_for<'c>(catalog: &'c Catalog, layout: &LayoutInput) -> Option<&'c RulePack> {
    let id = layout.layout.as_ref().and_then(|r| r.rulepack.split('@').next());
    id.and_then(|id| catalog.rulepack(Some(id))).or_else(|| catalog.rulepack(None))
}

fn validate(req: &CheckRequest) -> Result<(), UbotError> {
    if !(req.unit_scale.is_finite() && req.unit_scale > 0.0) {
        return Err(UbotError::BadInput("unitScale must be a finite number > 0".into()));
    }
    for (what, len, max) in
        [("instances", req.instances.len(), MAX_INSTANCES), ("layouts", req.layouts.len(), MAX_LAYOUTS), ("zones", req.zones.len(), MAX_ZONES)]
    {
        if len > max {
            return Err(UbotError::Limit(format!("{len} {what}, the limit is {max}")));
        }
    }
    for l in &req.layouts {
        let n = l.holes.iter().fold(l.boundary.vertices.len(), |n, h| n.saturating_add(h.vertices.len()));
        if n > MAX_VERTICES {
            return Err(UbotError::Limit(format!("layout {} has {n} boundary and hole vertices, the limit is {MAX_VERTICES}", l.layout_id)));
        }
    }
    Ok(())
}

/// Zones within reach of some layout ([`ZoneReach`], as fill uses); without layouts every zone is kept, so loose
/// boxes are still checked against them. A non-finite zone is an error wherever it is.
fn reachable_zones(req: &CheckRequest) -> Result<Vec<ZoneInput>, UbotError> {
    let reaches = req.layouts.iter().map(|l| ZoneReach::new(req.unit_scale, &l.boundary)).collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for z in &req.zones {
        if z.ring.vertices.iter().flatten().any(|c| !(c * req.unit_scale).is_finite()) {
            return Err(UbotError::BadInput(format!("zone {}: ring vertex is not a finite number in mm", z.id)));
        }
        let mut near = reaches.is_empty();
        for r in &reaches {
            if near {
                break;
            }
            near = r.reaches(req.unit_scale, &z.ring)?;
        }
        if near {
            out.push(z.clone());
        }
    }
    let zone_vertices = out.iter().fold(0usize, |n, z| n.saturating_add(z.ring.vertices.len()));
    if zone_vertices > MAX_VERTICES {
        return Err(UbotError::Limit(format!("{zone_vertices} zone vertices, the limit is {MAX_VERTICES}")));
    }
    Ok(out)
}

/// Audits boxes, layouts and zones as given; nothing is modified. Issues run top-down, then left to right.
pub fn check(req: &CheckRequest, catalog: &Catalog) -> Result<Vec<Issue>, UbotError> {
    validate(req)?;
    let near_zones = reachable_zones(req)?;
    let mut out: Vec<Issue> = Vec::new();
    let mut lookup = Lookup::new(catalog);

    // Layouts: the lowest boundary handle owns an id, later copies are duplicates.
    let mut order: Vec<usize> = (0..req.layouts.len()).collect();
    order.sort_by_cached_key(|&k| req.layouts.get(k).map(|l| handle_key(&l.boundary_handle)));
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for &k in &order {
        let Some(l) = req.layouts.get(k) else { continue };
        if !owner.contains_key(l.layout_id.as_str()) {
            owner.insert(l.layout_id.as_str(), k);
        } else {
            let at = l.boundary.vertices.first().map_or([0.0, 0.0], |v| [v[0], v[1]]);
            out.push(Issue {
                code: IssueCode::DuplicateLayout,
                severity: Severity::Error,
                handle: Some(l.boundary_handle.clone()),
                at,
                message: format!("layout id {} is also used by another boundary", l.layout_id),
            });
        }
    }

    let mut points = 0usize;
    let mut ctxs: Vec<LayoutCtx<'_>> = Vec::with_capacity(req.layouts.len());
    for l in &req.layouts {
        let rulepack = rulepack_for(catalog, l);
        let edge_mm = match l.edge_clearance_mm {
            Some(v) => clearance_mm(v, "edgeClearanceMm")?,
            None => clearance_mm(rulepack.map_or(0.0, |r| r.edge_clearance_mm), "edgeClearanceMm")?,
        };
        let exclusion_mm = clearance_mm(rulepack.map_or(0.0, |r| r.exclusion_clearance_mm), "exclusionClearanceMm")?;
        let product = l.layout.as_ref().and_then(|r| lookup.product(&r.product_id));
        ctxs.push(LayoutCtx::new(&l.boundary, &l.holes, req.unit_scale, (edge_mm, exclusion_mm), product, &mut points)?);
    }
    let default_exclusion = clearance_mm(catalog.rulepack(None).map_or(0.0, |r| r.exclusion_clearance_mm), "exclusionClearanceMm")?;
    let widest_default = ctxs.iter().fold(default_exclusion, |m, c| m.max(c.exclusion_mm));
    let zones = Zones::new(&near_zones, req.unit_scale, widest_default, &mut points)?;

    // Stale: the slab geometry no longer matches what the fill saw.
    for l in &req.layouts {
        let Some(stored) = &l.stored_geometry_hash else { continue };
        if *stored != geometry_hash(req.unit_scale, &l.boundary, &l.holes, &req.zones)? {
            let at = l.boundary.vertices.first().map_or([0.0, 0.0], |v| [v[0], v[1]]);
            out.push(Issue {
                code: IssueCode::StaleLayout,
                severity: Severity::Warning,
                handle: Some(l.boundary_handle.clone()),
                at,
                message: format!("slab, holes or zones changed since layout {} was filled", l.layout_id),
            });
        }
    }

    let mut work = Work::new();
    let mut placed: Vec<Placed<'_>> = Vec::with_capacity(req.instances.len());
    for inst in &req.instances {
        let layout = inst.base.layout_id.as_deref().and_then(|id| owner.get(id).copied());
        let p = resolve(inst, layout.and_then(|k| ctxs.get(k)).and_then(|c| c.product), &mut lookup, req.unit_scale)?;
        if !inst.xdata_valid {
            out.push(issue(IssueCode::XdataInvalid, Severity::Error, &p, "the LPC_UBOT data of this box cannot be read".into()));
        }
        if inst.legacy {
            out.push(issue(IssueCode::LegacyBlock, Severity::Warning, &p, format!("legacy block {}", inst.base.block_name)));
        }
        if p.unknown_product {
            let id = inst.base.product_id.as_deref().unwrap_or_default();
            out.push(issue(IssueCode::UnknownProduct, Severity::Error, &p, format!("product {id} is not in the catalog")));
        }
        if let Some(k) = layout {
            if let Some(l) = req.layouts.get(k)
                && let Some(i) = moved::moved(&p, l)?
            {
                out.push(i);
            }
            if let (Some(g), Some(ctx)) = (&p.geom, ctxs.get(k)) {
                check_layout(&p, g, ctx, &mut work, &mut out)?;
            }
        }
        if let Some(g) = &p.geom {
            let default_mm = layout.and_then(|k| ctxs.get(k)).map_or(default_exclusion, |c| c.exclusion_mm);
            check_zones(&p, g, &zones, default_mm, &mut work, &mut out)?;
        }
        placed.push(p);
    }

    duplicate_ids(&placed, &mut out);

    let boxes: Vec<(usize, _)> = placed.iter().enumerate().filter_map(|(k, p)| p.geom.as_ref().map(|g| (k, g.fp))).collect();
    let fps: Vec<_> = boxes.iter().map(|b| b.1).collect();
    for (a, b) in overlap::overlapping_pairs(&fps, &mut work)? {
        let (Some(&(ka, _)), Some(&(kb, _))) = (boxes.get(a), boxes.get(b)) else { continue };
        let (Some(pa), Some(pb)) = (placed.get(ka), placed.get(kb)) else { continue };
        // The box with the later handle carries the issue, so each pair is reported once at a stable place.
        let (later, earlier) = if handle_key(&pa.inst.base.handle) > handle_key(&pb.inst.base.handle) { (pa, pb) } else { (pb, pa) };
        out.push(issue(IssueCode::Overlap, Severity::Error, later, format!("box overlaps box {}", earlier.inst.base.handle)));
    }

    // Stable, so issues at the same point keep the order they were found in.
    out.sort_by(|a, b| b.at[1].total_cmp(&a.at[1]).then(a.at[0].total_cmp(&b.at[0])));
    Ok(out)
}

/// One issue per extra copy of an instance id; the lowest handle is the original.
fn duplicate_ids(placed: &[Placed<'_>], out: &mut Vec<Issue>) {
    let mut groups: HashMap<&str, Vec<&Placed<'_>>> = HashMap::new();
    for p in placed {
        if let Some(id) = p.inst.base.instance_id.as_deref() {
            groups.entry(id).or_default().push(p);
        }
    }
    let mut dups: Vec<&Placed<'_>> = Vec::new();
    for mut copies in groups.into_values().filter(|g| g.len() > 1) {
        copies.sort_by_cached_key(|p| handle_key(&p.inst.base.handle));
        dups.extend(copies.into_iter().skip(1));
    }
    // HashMap order is random; fix it so the result is reproducible.
    dups.sort_by_cached_key(|p| handle_key(&p.inst.base.handle));
    for p in dups {
        let id = p.inst.base.instance_id.as_deref().unwrap_or_default();
        out.push(issue(IssueCode::DuplicateId, Severity::Error, p, format!("instance id {id} is used by another box")));
    }
}
