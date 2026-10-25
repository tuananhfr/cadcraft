//! Box against slab boundary, holes and zones, reusing the fill footprint predicates.

use cadcraft_geom::Bounds2;

use super::resolve::{Geom, Placed};
use super::work::Work;
use super::{Issue, IssueCode, Severity, issue};
use crate::error::UbotError;
use crate::fill::{NEAR_BAND_MM, ZoneInput, ZoneType};
use crate::geometry::{
    EdgeIndex, Footprint, Ring, RingInput, distance_footprint_ring_edges_with, footprint_intersects_ring_with, ring_contains_footprint_with,
    ring_from_input,
};
use crate::model::Product;

/// Budget units per ring edge tested against a footprint: one segment test per footprint side.
const EDGE_COST: usize = 4;
/// Distances this close to the required clearance count as meeting it (float noise, not geometry).
const MEETS_TOL_MM: f64 = 1e-6;
/// Added to query margins so edges exactly at the clearance distance are still returned.
const QUERY_PAD_MM: f64 = 1e-3;

pub(super) struct LayoutCtx<'c> {
    boundary: Ring,
    holes: Vec<Ring>,
    hole_bounds: Vec<Bounds2>,
    index: EdgeIndex,
    pub edge_mm: f64,
    pub exclusion_mm: f64,
    pub product: Option<&'c Product>,
}

impl<'c> LayoutCtx<'c> {
    pub fn new(
        boundary: &RingInput,
        holes: &[RingInput],
        unit_scale: f64,
        edge_mm: f64,
        exclusion_mm: f64,
        product: Option<&'c Product>,
    ) -> Result<Self, UbotError> {
        let ring = |r: &RingInput| ring_from_input(r, unit_scale, NEAR_BAND_MM);
        let boundary = ring(boundary)?;
        let holes = holes.iter().map(ring).collect::<Result<Vec<Ring>, UbotError>>()?;
        let hole_bounds = holes.iter().map(Ring::bounds).collect();
        let refs: Vec<&Ring> = std::iter::once(&boundary).chain(holes.iter()).collect();
        let index = EdgeIndex::new(&refs, 2000.0);
        Ok(LayoutCtx { boundary, holes, hole_bounds, index, edge_mm, exclusion_mm, product })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blocker,
    Alt,
}

struct Zone {
    id: String,
    ring: Ring,
    bounds: Bounds2,
    kind: Kind,
    clearance_mm: Option<f64>,
}

pub(super) struct Zones {
    zones: Vec<Zone>,
    index: EdgeIndex,
    margin: f64,
}

impl Zones {
    /// `max_default_mm` is the widest clearance a zone without its own value can inherit.
    pub fn new(inputs: &[ZoneInput], unit_scale: f64, max_default_mm: f64) -> Result<Self, UbotError> {
        let mut zones = Vec::with_capacity(inputs.len());
        let mut widest = max_default_mm;
        for z in inputs {
            let kind = if z.zone_type == ZoneType::AltProduct { Kind::Alt } else { Kind::Blocker };
            if let Some(c) = z.clearance_mm {
                widest = widest.max(super::clearance_mm(c, &format!("zone {} clearanceMm", z.id))?);
            }
            let ring = ring_from_input(&z.ring, unit_scale, NEAR_BAND_MM)?;
            zones.push(Zone { id: z.id.clone(), bounds: ring.bounds(), ring, kind, clearance_mm: z.clearance_mm });
        }
        let refs: Vec<&Ring> = zones.iter().map(|z| &z.ring).collect();
        let index = EdgeIndex::new(&refs, 2000.0);
        Ok(Zones { zones, index, margin: widest + NEAR_BAND_MM + QUERY_PAD_MM })
    }
}

/// Edge indices of `ring` among the sorted `(ring, edge)` candidates.
fn ring_edges(near: &[(usize, usize)], ring: usize) -> Vec<usize> {
    let lo = near.partition_point(|n| n.0 < ring);
    let hi = near.partition_point(|n| n.0 <= ring);
    near.get(lo..hi).unwrap_or(&[]).iter().map(|n| n.1).collect()
}

fn near_edges(index: &EdgeIndex, bounds: Bounds2, margin: f64, work: &mut Work) -> Result<Vec<(usize, usize)>, UbotError> {
    // Charged before the query runs so a wide reach fails fast instead of collecting first.
    work.spend(index.query_cost(bounds, margin, work.remaining()))?;
    let near: Vec<(usize, usize)> = index.query(bounds, margin).collect();
    work.spend(near.len().saturating_mul(EDGE_COST))?;
    Ok(near)
}

fn clearance(
    p: &Placed<'_>,
    fp: &Footprint,
    (ring, edges): (&Ring, &[usize]),
    required_mm: f64,
    what: &str,
    work: &mut Work,
    out: &mut Vec<Issue>,
) -> Result<(), UbotError> {
    if required_mm <= 0.0 || edges.is_empty() {
        return Ok(());
    }
    work.spend(edges.len().saturating_mul(EDGE_COST))?;
    let (d, _) = distance_footprint_ring_edges_with(fp, ring, edges.iter().copied());
    let short = required_mm - d;
    if short > MEETS_TOL_MM {
        // Within the arc tolerance band the shortfall may be chord noise, so it is only a warning.
        let severity = if short > NEAR_BAND_MM { Severity::Error } else { Severity::Warning };
        out.push(issue(IssueCode::Clearance, severity, p, format!("{what}: {d:.1} mm to the edge, {required_mm} mm required")));
    }
    Ok(())
}

pub(super) fn check_layout(p: &Placed<'_>, g: &Geom, ctx: &LayoutCtx<'_>, work: &mut Work, out: &mut Vec<Issue>) -> Result<(), UbotError> {
    let margin = ctx.edge_mm + NEAR_BAND_MM + QUERY_PAD_MM;
    let near = near_edges(&ctx.index, g.bounds, margin, work)?;
    let edges = ring_edges(&near, 0);
    work.spend(ctx.boundary.edge_count())?;
    if ring_contains_footprint_with(&ctx.boundary, &g.fp, edges.iter().copied()) {
        clearance(p, &g.fp, (&ctx.boundary, &edges), ctx.edge_mm, "slab edge", work, out)?;
    } else {
        out.push(issue(IssueCode::OutsideBoundary, Severity::Error, p, "box is not inside the slab boundary".into()));
    }
    for (k, hole) in ctx.holes.iter().enumerate() {
        let Some(hb) = ctx.hole_bounds.get(k) else { continue };
        if !hb.expand(margin).intersects(&g.bounds) {
            continue;
        }
        let edges = ring_edges(&near, k + 1);
        work.spend(hole.edge_count())?;
        if footprint_intersects_ring_with(&g.fp, hole, edges.iter().copied()) {
            out.push(issue(IssueCode::OutsideBoundary, Severity::Error, p, "box overlaps a hole of the slab".into()));
        } else {
            clearance(p, &g.fp, (hole, &edges), ctx.edge_mm, "hole edge", work, out)?;
        }
    }
    Ok(())
}

/// `default_mm` is the clearance of zones that carry none (the rulepack's exclusion clearance).
pub(super) fn check_zones(p: &Placed<'_>, g: &Geom, zones: &Zones, default_mm: f64, work: &mut Work, out: &mut Vec<Issue>) -> Result<(), UbotError> {
    if zones.zones.is_empty() {
        return Ok(());
    }
    let near = near_edges(&zones.index, g.bounds, zones.margin, work)?;
    for (k, z) in zones.zones.iter().enumerate() {
        work.spend(1)?;
        if z.kind == Kind::Alt {
            continue;
        }
        let required = z.clearance_mm.unwrap_or(default_mm);
        if !z.bounds.expand(required + NEAR_BAND_MM + QUERY_PAD_MM).intersects(&g.bounds) {
            continue;
        }
        let edges = ring_edges(&near, k);
        work.spend(z.ring.edge_count())?;
        if footprint_intersects_ring_with(&g.fp, &z.ring, edges.iter().copied()) {
            out.push(issue(IssueCode::InZone, Severity::Error, p, format!("box is inside zone {}", z.id)));
        } else {
            clearance(p, &g.fp, (&z.ring, &edges), required, &format!("zone {}", z.id), work, out)?;
        }
    }
    Ok(())
}
