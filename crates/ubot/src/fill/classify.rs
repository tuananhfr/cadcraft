//! Per-cell classification (GEOM-03/04): boundary, holes, zones and boxes of other layouts.

use cadcraft_geom::{Bounds2, Vec2};

use super::grid::{Budget, Grid, RowCover, Scanline};
use super::{Reason, Status};
use crate::error::UbotError;
use crate::geometry::{EdgeIndex, Footprint, Ring, any_edge_enters, distance_footprint_ring_edges_with, footprint};
use crate::model::Product;

/// Tessellation tolerance of arcs, and the band around a clearance threshold that is "uncertain" instead of decided.
pub(super) const NEAR_BAND_MM: f64 = 0.5;
/// Distances this close to the required clearance count as meeting it (float noise, not geometry).
const MEETS_TOL_MM: f64 = 1e-6;
/// Touching footprints share an edge; only overlap deeper than this blocks a cell.
const OVERLAP_TOL_MM: f64 = 1e-6;
/// Bucket entries for existing boxes; only boxes far larger than a cell can approach it.
const MAX_OBSTACLE_ENTRIES: usize = 10_000_000;

#[derive(Clone, Copy)]
pub(super) enum Role<'a> {
    Boundary,
    Hole,
    Opening { clearance_mm: f64 },
    Blocker { clearance_mm: f64 },
    Alt { product: &'a Product },
}

/// Rings in mm: index 0 is the boundary, then holes, then zones in request order.
pub(super) struct Domain<'a> {
    pub rings: Vec<Ring>,
    pub roles: Vec<Role<'a>>,
    pub edge_clearance_mm: f64,
}

impl Domain<'_> {
    /// Reach of the widest clearance check plus the tessellation band.
    fn margin(&self) -> f64 {
        let zone_max = self.roles.iter().fold(0.0_f64, |m, r| match r {
            Role::Opening { clearance_mm } | Role::Blocker { clearance_mm } => m.max(*clearance_mm),
            _ => m,
        });
        self.edge_clearance_mm.max(zone_max) + NEAR_BAND_MM + 1e-3
    }
}

pub(super) struct Cell<'a> {
    pub cell: [i64; 2],
    pub center_mm: Vec2,
    pub status: Status,
    pub reasons: Vec<Reason>,
    pub product: &'a Product,
}

/// Footprints bucketed by the grid cells they overlap.
pub(super) struct Obstacles {
    fps: Vec<Footprint>,
    buckets: Vec<Vec<u32>>,
}

impl Obstacles {
    pub fn new(grid: &Grid, fps: Vec<Footprint>) -> Result<Obstacles, UbotError> {
        let mut buckets: Vec<Vec<u32>> = Vec::new();
        if !fps.is_empty() {
            buckets.resize_with(grid.cell_count(), Vec::new);
        }
        let mut entries = 0usize;
        for (k, fp) in fps.iter().enumerate() {
            let bounds = Bounds2::from_points(fp.corners.iter().map(|c| grid.to_grid(*c)));
            let (Some((i0, i1, j0, j1)), Ok(id)) = (grid.cells_overlapping(&bounds), u32::try_from(k)) else { continue };
            for j in j0..=j1 {
                for i in i0..=i1 {
                    entries += 1;
                    if entries > MAX_OBSTACLE_ENTRIES {
                        return Err(UbotError::Limit("existing boxes cover too many cells".into()));
                    }
                    if let Some(b) = grid.index(i, j).and_then(|n| buckets.get_mut(n)) {
                        b.push(id);
                    }
                }
            }
        }
        Ok(Obstacles { fps, buckets })
    }

    pub fn blocks(&self, grid: &Grid, i: i64, j: i64, fp: &Footprint) -> bool {
        let Some(bucket) = grid.index(i, j).and_then(|n| self.buckets.get(n)) else { return false };
        bucket.iter().filter_map(|&k| self.fps.get(k as usize)).any(|o| footprints_overlap(fp, o))
    }
}

/// Separating-axis test on the four edge directions; touching is not overlap.
fn footprints_overlap(a: &Footprint, b: &Footprint) -> bool {
    let axes = |f: &Footprint| [(f.corners[1] - f.corners[0]).normalized(), (f.corners[3] - f.corners[0]).normalized()];
    let project = |f: &Footprint, axis: Vec2| {
        f.corners.iter().map(|c| c.dot(axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| (lo.min(d), hi.max(d)))
    };
    for axis in axes(a).into_iter().chain(axes(b)) {
        let ((a0, a1), (b0, b1)) = (project(a, axis), project(b, axis));
        if !(a1 > b0 + OVERLAP_TOL_MM && b1 > a0 + OVERLAP_TOL_MM) {
            return false;
        }
    }
    true
}

/// Classifies every in-range cell that meets the boundary, rows top-down then left to right.
pub(super) fn classify<'a>(grid: &Grid, domain: &Domain<'a>, obstacles: &Obstacles, main: &'a Product) -> Result<Vec<Cell<'a>>, UbotError> {
    let refs: Vec<&Ring> = domain.rings.iter().collect();
    let index = EdgeIndex::new(&refs, grid.pitch_mm.x.max(grid.pitch_mm.y));
    let margin = domain.margin();
    let mut scan = Scanline::new(domain.rings.iter().enumerate().flat_map(|(r, ring)| {
        let ring_id = u32::try_from(r).unwrap_or(u32::MAX);
        (0..ring.edge_count()).filter_map(move |e| ring.edge(e)).map(move |(a, b)| (ring_id, grid.to_grid(a), grid.to_grid(b)))
    }));
    let mut budget = Budget::new();
    let mut out = Vec::new();
    let (mut covering, mut near) = (Vec::new(), Vec::new());
    for j in grid.j_range.0..=grid.j_range.1 {
        let mut cover = RowCover::new(scan.at(grid.row_y(j), &mut budget)?);
        for i in grid.i_range.0..=grid.i_range.1 {
            let g = grid.center(i, j);
            let center_mm = grid.to_world(g);
            let fp = footprint(center_mm, grid.size_mm.x, grid.size_mm.y, grid.angle_deg);
            cover.at(g.x, &mut covering, &mut budget)?;
            near.clear();
            near.extend(index.query(Bounds2::from_points(fp.corners), margin));
            near.sort_unstable();
            budget.spend(near.len())?;
            let probe = Probe { domain, fp: &fp, near: &near, covering: &covering };
            let Some(verdict) = probe.judge() else { continue };
            let product = verdict.alt.unwrap_or(main);
            let (status, reasons) = verdict.status(obstacles.blocks(grid, i, j, &fp));
            out.push(Cell { cell: [i, j], center_mm, status, reasons, product });
        }
    }
    Ok(out)
}

struct Probe<'p, 'a> {
    domain: &'p Domain<'a>,
    fp: &'p Footprint,
    /// `(ring, edge)` within reach of the footprint, sorted.
    near: &'p [(usize, usize)],
    /// Rings whose inside contains the footprint centre, sorted.
    covering: &'p [u32],
}

struct Verdict<'a> {
    invalid: Vec<Reason>,
    uncertain: Vec<Reason>,
    alt: Option<&'a Product>,
}

impl Verdict<'_> {
    fn status(mut self, blocked: bool) -> (Status, Vec<Reason>) {
        if blocked {
            self.invalid.push(Reason::OverlapsExisting);
        }
        for list in [&mut self.invalid, &mut self.uncertain] {
            list.sort_unstable();
            list.dedup();
        }
        if !self.invalid.is_empty() {
            (Status::Invalid, self.invalid)
        } else if !self.uncertain.is_empty() {
            (Status::Uncertain, self.uncertain)
        } else {
            (Status::Valid, Vec::new())
        }
    }
}

impl<'a> Probe<'_, 'a> {
    /// `None` when the footprint lies wholly outside the boundary (such cells are not reported).
    fn judge(&self) -> Option<Verdict<'a>> {
        let mut rings: Vec<usize> = self.near.iter().map(|n| n.0).chain(self.covering.iter().map(|&r| r as usize)).collect();
        rings.sort_unstable();
        rings.dedup();
        if rings.first() != Some(&0) {
            return None;
        }
        let mut v = Verdict { invalid: Vec::new(), uncertain: Vec::new(), alt: None };
        for r in rings {
            let (Some(ring), Some(role)) = (self.domain.rings.get(r), self.domain.roles.get(r)) else { continue };
            let edges = self.edges_of(r);
            let center_in = u32::try_from(r).is_ok_and(|id| self.covering.binary_search(&id).is_ok());
            let meets = || center_in || any_edge_enters(self.fp, ring, edges.iter().map(|e| e.1));
            match *role {
                Role::Boundary => {
                    if any_edge_enters(self.fp, ring, edges.iter().map(|e| e.1)) {
                        v.invalid.push(Reason::OutsideBoundary);
                    } else if !center_in {
                        return None;
                    } else {
                        self.clearance(&mut v, ring, edges, self.domain.edge_clearance_mm, Reason::EdgeClearance);
                    }
                }
                Role::Hole if meets() => v.invalid.push(Reason::InHole),
                Role::Hole => self.clearance(&mut v, ring, edges, self.domain.edge_clearance_mm, Reason::EdgeClearance),
                Role::Opening { .. } if meets() => v.invalid.push(Reason::InHole),
                Role::Blocker { .. } if meets() => v.invalid.push(Reason::InZone),
                Role::Opening { clearance_mm } | Role::Blocker { clearance_mm } => {
                    self.clearance(&mut v, ring, edges, clearance_mm, Reason::ZoneClearance)
                }
                Role::Alt { product } => {
                    if center_in && v.alt.is_none() {
                        v.alt = Some(product);
                    }
                }
            }
        }
        Some(v)
    }

    fn edges_of(&self, ring: usize) -> &[(usize, usize)] {
        let lo = self.near.partition_point(|n| n.0 < ring);
        let hi = self.near.partition_point(|n| n.0 <= ring);
        self.near.get(lo..hi).unwrap_or(&[])
    }

    /// R3.1/R3.2: short by more than the band → invalid; short within it → NearLimit; meeting it within the band of a
    /// curved edge → CurvedEdgeTolerance (the chords may hide up to the band).
    fn clearance(&self, v: &mut Verdict<'_>, ring: &Ring, edges: &[(usize, usize)], required_mm: f64, reason: Reason) {
        if edges.is_empty() {
            return;
        }
        let (d, curved) = distance_footprint_ring_edges_with(self.fp, ring, edges.iter().map(|e| e.1));
        let short = required_mm - d;
        if short > NEAR_BAND_MM {
            v.invalid.push(reason);
        } else if short > MEETS_TOL_MM {
            v.uncertain.push(Reason::NearLimit);
        } else if curved && d - required_mm < NEAR_BAND_MM {
            v.uncertain.push(Reason::CurvedEdgeTolerance);
        }
    }
}
