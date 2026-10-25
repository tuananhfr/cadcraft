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
use crate::fill::{ExistingInstance, LayoutRecord, MAX_CLEARANCE_MM, MAX_VERTICES, ZoneInput, ZoneMm, ZoneReach, geometry_hash_of, handle_key};
use crate::geometry::RingInput;
use crate::model::RulePack;
use cadcraft_geom::Bounds2;
use containment::{LayoutCtx, Zones, check_layout, check_zones};
use resolve::{Lookup, Placed, resolve};
use work::Work;
use zone_grid::ZoneGrid;

const MAX_INSTANCES: usize = 100_000;
const MAX_LAYOUTS: usize = 1_000;
const MAX_ZONES: usize = 20_000;
/// Zone vertices in a request before any are looked at; only zones near a slab or box count towards `MAX_VERTICES`.
const MAX_RAW_ZONE_VERTICES: usize = 1_000_000;

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
    let raw = req.zones.iter().fold(0usize, |n, z| n.saturating_add(z.ring.vertices.len()));
    if raw > MAX_RAW_ZONE_VERTICES {
        return Err(UbotError::Limit(format!("{raw} zone vertices, the limit is {MAX_RAW_ZONE_VERTICES}")));
    }
    for l in &req.layouts {
        let n = l.holes.iter().fold(l.boundary.vertices.len(), |n, h| n.saturating_add(h.vertices.len()));
        if n > MAX_VERTICES {
            return Err(UbotError::Limit(format!("layout {} has {n} boundary and hole vertices, the limit is {MAX_VERTICES}", l.layout_id)));
        }
    }
    Ok(())
}

/// Every zone in mm once (a non-finite zone is an error wherever it is), bucketed by extent so a slab or box only
/// visits the zones near it (ruling R17).
struct ZoneIndex<'r> {
    zones: &'r [ZoneInput],
    mm: Vec<ZoneMm>,
    grid: ZoneGrid,
    digests: Vec<Option<String>>,
}

impl<'r> ZoneIndex<'r> {
    fn new(zones: &'r [ZoneInput], unit_scale: f64) -> Result<Self, UbotError> {
        let mm = zones.iter().map(|z| ZoneMm::new(unit_scale, z)).collect::<Result<Vec<_>, _>>()?;
        let extents: Vec<_> = mm.iter().map(|z| z.extent).collect();
        Ok(ZoneIndex { zones, grid: ZoneGrid::new(&extents), digests: vec![None; mm.len()], mm })
    }

    /// Zones within `reach` of a slab, exactly as [`crate::geometry_hash`] picks them, ascending.
    fn reached(&self, reach: &ZoneReach, work: &mut Work) -> Result<Vec<usize>, UbotError> {
        let candidates = self.grid.candidates(&reach.bounds());
        work.spend(candidates.len().saturating_add(1))?;
        Ok(candidates.into_iter().filter(|&k| self.mm.get(k).is_some_and(|z| reach.meets(&z.extent))).collect())
    }

    /// The layout's geometry hash from its pre-filtered zones; equals `geometry_hash(boundary, holes, all zones)`.
    fn layout_hash(&mut self, l: &LayoutInput, unit_scale: f64, work: &mut Work) -> Result<String, UbotError> {
        let reach = ZoneReach::new(unit_scale, &l.boundary)?;
        let mut digests = Vec::new();
        for k in self.reached(&reach, work)? {
            let (Some(z), Some(mm), Some(slot)) = (self.zones.get(k), self.mm.get(k), self.digests.get_mut(k)) else { continue };
            work.spend(z.ring.vertices.len())?;
            digests.push(slot.get_or_insert_with(|| mm.digest(z)).clone());
        }
        geometry_hash_of(unit_scale, &l.boundary, &l.holes, digests)
    }
}

/// Zones relevant to the check: within reach of any layout or near any box with a footprint, so loose boxes in a
/// zone are checked with or without layouts. Only these count towards `MAX_VERTICES`.
fn relevant_zones(req: &CheckRequest, index: &ZoneIndex<'_>, placed: &[Placed<'_>], work: &mut Work) -> Result<Vec<ZoneInput>, UbotError> {
    let mut relevant = vec![false; req.zones.len()];
    for l in &req.layouts {
        for k in index.reached(&ZoneReach::new(req.unit_scale, &l.boundary)?, work)? {
            if let Some(r) = relevant.get_mut(k) {
                *r = true;
            }
        }
    }
    // Per zone, the first box within reach settles it; buckets of twice the reach keep each query to a few buckets.
    let boxes: Vec<Bounds2> = placed.iter().filter_map(|p| p.geom.as_ref().map(|g| g.bounds)).collect();
    if !boxes.is_empty() {
        let grid = ZoneGrid::with_cell(&boxes, 2.0 * MAX_CLEARANCE_MM);
        for (z, r) in index.mm.iter().zip(relevant.iter_mut()).filter(|(_, r)| !**r) {
            let grown = z.extent.expand(MAX_CLEARANCE_MM);
            *r = grid.any(&grown, work, |b| boxes.get(b).is_some_and(|b| b.intersects(&grown)))?;
        }
    }
    let out: Vec<ZoneInput> = req.zones.iter().zip(&relevant).filter(|(_, r)| **r).map(|(z, _)| z.clone()).collect();
    let zone_vertices = out.iter().fold(0usize, |n, z| n.saturating_add(z.ring.vertices.len()));
    if zone_vertices > MAX_VERTICES {
        return Err(UbotError::Limit(format!("{zone_vertices} zone vertices, the limit is {MAX_VERTICES}")));
    }
    Ok(out)
}

/// Audits boxes, layouts and zones as given; nothing is modified. Issues run top-down, then left to right.
pub fn check(req: &CheckRequest, catalog: &Catalog) -> Result<Vec<Issue>, UbotError> {
    validate(req)?;
    let mut index = ZoneIndex::new(&req.zones, req.unit_scale)?;
    let mut work = Work::new();
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

    let mut placed: Vec<Placed<'_>> = Vec::with_capacity(req.instances.len());
    let mut layout_of: Vec<Option<usize>> = Vec::with_capacity(req.instances.len());
    for inst in &req.instances {
        let layout = inst.base.layout_id.as_deref().and_then(|id| owner.get(id).copied());
        placed.push(resolve(inst, layout.and_then(|k| ctxs.get(k)).and_then(|c| c.product), &mut lookup, req.unit_scale)?);
        layout_of.push(layout);
    }
    let near_zones = relevant_zones(req, &index, &placed, &mut work)?;
    let zones = Zones::new(&near_zones, req.unit_scale, widest_default, &mut points)?;

    // Stale: the slab geometry no longer matches what the fill saw.
    for l in &req.layouts {
        let Some(stored) = &l.stored_geometry_hash else { continue };
        if *stored != index.layout_hash(l, req.unit_scale, &mut work)? {
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

    for (p, &layout) in placed.iter().zip(&layout_of) {
        let inst = p.inst;
        if !inst.xdata_valid {
            out.push(issue(IssueCode::XdataInvalid, Severity::Error, p, "the LPC_UBOT data of this box cannot be read".into()));
        }
        if inst.legacy {
            out.push(issue(IssueCode::LegacyBlock, Severity::Warning, p, format!("legacy block {}", inst.base.block_name)));
        }
        if p.unknown_product {
            let id = inst.base.product_id.as_deref().unwrap_or_default();
            out.push(issue(IssueCode::UnknownProduct, Severity::Error, p, format!("product {id} is not in the catalog")));
        }
        if let Some(k) = layout {
            if let Some(l) = req.layouts.get(k)
                && let Some(i) = moved::moved(p, l)?
            {
                out.push(i);
            }
            if let (Some(g), Some(ctx)) = (&p.geom, ctxs.get(k)) {
                check_layout(p, g, ctx, &mut work, &mut out)?;
            }
        }
        if let Some(g) = &p.geom {
            let default_mm = layout.and_then(|k| ctxs.get(k)).map_or(default_exclusion, |c| c.exclusion_mm);
            check_zones(p, g, &zones, default_mm, &mut work, &mut out)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill::ZoneType;
    use crate::fill::geometry_hash;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RingInput {
        RingInput { vertices: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].iter().map(|p| [p[0], p[1], 0.0]).collect() }
    }

    fn zone(id: &str, ring: RingInput) -> ZoneInput {
        ZoneInput { id: id.into(), ring, zone_type: ZoneType::Exclusion, clearance_mm: None, alt_product_id: None }
    }

    #[test]
    fn layout_hash_equals_geometry_hash_over_all_zones() {
        let zones = vec![
            zone("near", rect(60.0, 10.0, 70.0, 20.0)),
            zone("arc", RingInput { vertices: vec![[200.0, 200.0, 1.0], [220.0, 200.0, 1.0]] }),
            zone("bulge", RingInput { vertices: vec![[1900.0, 300.0, 0.0], [1910.0, 300.0, 50.0]] }),
            zone("edge", rect(1660.0, 0.0, 1670.0, 10.0)),
            zone("far", rect(1.0e5, 1.0e5, 1.0e5 + 10.0, 1.0e5 + 10.0)),
        ];
        let layout = LayoutInput {
            layout_id: "0123456789abcdef".into(),
            boundary_handle: "A1".into(),
            boundary: rect(0.0, 0.0, 660.0, 660.0),
            holes: vec![rect(300.0, 300.0, 320.0, 320.0)],
            stored_geometry_hash: None,
            layout: None,
            edge_clearance_mm: None,
        };
        for us in [1.0, 10.0, 0.001] {
            let mut index = ZoneIndex::new(&zones, us).unwrap();
            let got = index.layout_hash(&layout, us, &mut Work::new()).unwrap();
            assert_eq!(got, geometry_hash(us, &layout.boundary, &layout.holes, &zones).unwrap(), "unit scale {us}");
        }
    }
}
