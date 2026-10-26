//! Diff of the classified grid against boxes already in the drawing (spec §14.1), keyed by the `:<i>:<j>` cell of
//! each instance id so ids stay stable across runs.

use std::collections::{HashMap, HashSet};

use cadcraft_geom::{Bounds2, Vec2};

use super::classify::{Cell, Obstacles};
use super::grid::{Budget, Grid, Scanline, inside};
use super::{Candidate, ExistingInstance, Placement, Reason, Status, UpdatePlacement, ZoneReach};
use crate::catalog::Catalog;
use crate::error::UbotError;
use crate::geometry::{Footprint, Ring, RingInput, footprint};
use crate::model::{BlockMapping, Product};
use crate::placement::{center_from_insert_xy, insert_from_center, relative_scale};

/// Existing boxes per run (spec §6).
pub(super) const MAX_EXISTING: usize = 100_000;
/// Spec §14.1: a box more than 1 mm from its computed spot counts as moved by hand.
pub(crate) const MOVE_TOL_MM: f64 = 1.0;
pub(crate) const TURN_TOL_DEG: f64 = 1e-6;

/// An existing box of this layout, with its cell from the instance id.
pub(super) struct Owned<'e> {
    ex: &'e ExistingInstance,
    cell: [i64; 2],
    center_mm: Vec2,
    /// Footprint rotation: INSERT rotation minus the block's rotation offset.
    angle_deg: f64,
    fp: Footprint,
}

/// Turns centres and footprint angles into placements for one layout.
pub(super) struct Placer<'c> {
    pub layout_id: &'c str,
    pub unit_scale: f64,
    mappings: HashMap<&'c str, Option<&'c BlockMapping>>,
}

impl<'c> Placer<'c> {
    /// `products` are the ones that can be placed (main and alt), so mapping lookups are done once.
    pub fn new(catalog: &'c Catalog, layout_id: &'c str, unit_scale: f64, products: &[&'c Product]) -> Self {
        let mappings = products.iter().map(|p| (p.id.as_str(), catalog.mapping_for_product(&p.id))).collect();
        Placer { layout_id, unit_scale, mappings }
    }

    pub fn place(&self, cell: [i64; 2], center_mm: Vec2, product: &Product, angle_deg: f64) -> Placement {
        let mapping = self.mappings.get(product.id.as_str()).copied().flatten();
        let block_name = mapping.map_or_else(|| Catalog::default_block_name(product), |m| m.block_name.clone());
        let rotation_deg = angle_deg + mapping.map_or(0.0, |m| m.rotation_offset_deg);
        let insert = insert_from_center(mapping, product.length_mm, center_mm, rotation_deg, 1.0);
        Placement {
            instance_id: format!("{}:{}:{}", self.layout_id, cell[0], cell[1]),
            cell,
            product_id: product.id.clone(),
            block_name,
            insert: self.drawing(insert),
            center: self.drawing(center_mm),
            rotation_deg,
        }
    }

    pub fn drawing(&self, mm: Vec2) -> [f64; 2] {
        [mm.x / self.unit_scale, mm.y / self.unit_scale]
    }
}

/// `"<layout>:<i>:<j>"` → `(i, j)`.
pub(crate) fn parse_cell(id: &str) -> Option<[i64; 2]> {
    let mut parts = id.rsplitn(3, ':');
    let j = parts.next()?.parse().ok()?;
    let i = parts.next()?.parse().ok()?;
    parts.next().filter(|prefix| !prefix.is_empty())?;
    Some([i, j])
}

fn bad_existing(e: &ExistingInstance, what: &str) -> UbotError {
    UbotError::BadInput(format!("existing box {}: {what}", e.handle))
}

/// Where the core places an existing box: footprint centre (mm), footprint angle and footprint.
fn locate(e: &ExistingInstance, mapping: Option<&BlockMapping>, size: Vec2, unit_scale: f64) -> Result<(Vec2, f64, Footprint), UbotError> {
    if !(e.insert.iter().all(|c| c.is_finite()) && e.rotation_deg.is_finite()) {
        return Err(bad_existing(e, "insert and rotation must be finite"));
    }
    let scale_y = e.scale_y.unwrap_or(e.scale);
    if !(e.scale.is_finite() && e.scale != 0.0 && scale_y.is_finite() && scale_y != 0.0) {
        return Err(bad_existing(e, "scale must be finite and non-zero"));
    }
    let insert_mm = Vec2::new(e.insert[0], e.insert[1]) * unit_scale;
    // Per-axis signed scales: a mirrored box flips its anchor offset with it.
    let rel = [relative_scale(mapping, e.scale, unit_scale), relative_scale(mapping, scale_y, unit_scale)];
    let center_mm = center_from_insert_xy(mapping, size.y, insert_mm, e.rotation_deg, rel);
    let angle_deg = e.rotation_deg - mapping.map_or(0.0, |m| m.rotation_offset_deg);
    if !(center_mm.is_finite() && angle_deg.is_finite()) {
        return Err(bad_existing(e, "position overflows in mm"));
    }
    Ok((center_mm, angle_deg, footprint(center_mm, size.x, size.y, angle_deg)))
}

/// Legacy blocks carry no product; they are the size of the box being laid out.
fn size_of(e: &ExistingInstance, catalog: &Catalog, main: &Product) -> Vec2 {
    e.product_id.as_deref().and_then(|id| catalog.product(id)).map_or(main, |p| p).size()
}

/// Which existing boxes can touch a slab's cells: those whose footprint meets the [`ZoneReach`] of the boundary grown
/// by the laid box's diagonal. Spec §6 caps existing boxes per run of a slab, so only these count and are used.
#[derive(Debug, Clone, Copy)]
pub struct ExistingReach {
    reach: Bounds2,
}

impl ExistingReach {
    pub fn new(unit_scale: f64, boundary: &RingInput, main: &Product) -> Result<Self, UbotError> {
        let diagonal = main.width_mm.hypot(main.length_mm);
        Ok(ExistingReach { reach: ZoneReach::new(unit_scale, boundary)?.bounds().expand(diagonal) })
    }

    /// Whether any of `boxes` can reach the slab. Pass one box, or the four corner cells of a MINSERT: every other
    /// cell's footprint lies within theirs. A box the core cannot place is an error, as in [`crate::fill`].
    pub fn reaches(&self, boxes: &[ExistingInstance], catalog: &Catalog, main: &Product, unit_scale: f64) -> Result<bool, UbotError> {
        let mut hull = Bounds2::default();
        for e in boxes {
            let (_, _, fp) = locate(e, catalog.mapping_for_block(&e.block_name), size_of(e, catalog, main), unit_scale)?;
            hull = hull.union(&Bounds2::from_points(fp.corners));
        }
        Ok(self.meets(&hull))
    }

    fn meets(&self, bounds: &Bounds2) -> bool {
        !bounds.is_empty() && bounds.intersects(&self.reach)
    }
}

/// Splits the existing boxes within `reach` into this layout's own (by layout id, centre inside the boundary and a cell
/// id) and obstacles; boxes beyond it can neither block a cell nor be this layout's, so they are left out.
pub(super) fn split_existing<'e>(
    existing: &'e [ExistingInstance],
    layout_ids: [Option<&str>; 2],
    catalog: &Catalog,
    main: &Product,
    (unit_scale, reach): (f64, &ExistingReach),
    boundary: &Ring,
) -> Result<(Vec<Owned<'e>>, Vec<Footprint>), UbotError> {
    let mut blocks: HashMap<String, Option<&BlockMapping>> = HashMap::new();
    let mut sizes: HashMap<&str, Vec2> = HashMap::new();
    let mut pending = Vec::new();
    let mut obstacles = Vec::new();
    let mut counted = 0usize;
    for e in existing {
        let mapping = *blocks.entry(e.block_name.to_ascii_lowercase()).or_insert_with(|| catalog.mapping_for_block(&e.block_name));
        let size = match e.product_id.as_deref() {
            Some(id) => *sizes.entry(id).or_insert_with(|| size_of(e, catalog, main)),
            None => main.size(),
        };
        let (center_mm, angle_deg, fp) = locate(e, mapping, size, unit_scale)?;
        if !reach.meets(&Bounds2::from_points(fp.corners)) {
            continue;
        }
        counted += 1;
        if counted > MAX_EXISTING {
            return Err(UbotError::Limit(format!("more than {MAX_EXISTING} existing boxes near the slab, the limit is {MAX_EXISTING}")));
        }
        let ours = e.layout_id.as_deref().is_some_and(|l| layout_ids.contains(&Some(l)));
        match e.instance_id.as_deref().and_then(parse_cell) {
            Some(cell) if ours => pending.push(Owned { ex: e, cell, center_mm, angle_deg, fp }),
            _ => obstacles.push(fp),
        }
    }
    // One downward sweep answers point-in-boundary for every candidate.
    pending.sort_by(|a, b| b.center_mm.y.total_cmp(&a.center_mm.y));
    let mut scan = Scanline::new((0..boundary.edge_count()).filter_map(|k| boundary.edge(k)).map(|(a, b)| (0, a, b)));
    let mut budget = Budget::new();
    let mut owned = Vec::with_capacity(pending.len());
    for o in pending {
        if inside(scan.at(o.center_mm.y, &mut budget)?, 0, o.center_mm.x) {
            owned.push(o);
        } else {
            obstacles.push(o.fp);
        }
    }
    Ok((owned, obstacles))
}

trait Size {
    fn size(&self) -> Vec2;
}

impl Size for Product {
    fn size(&self) -> Vec2 {
        Vec2::new(self.width_mm, self.length_mm)
    }
}

pub(super) struct DiffOut {
    pub add: Vec<Placement>,
    pub update: Vec<UpdatePlacement>,
    pub remove: Vec<String>,
    pub keep: Vec<String>,
    /// `Info(ManualMoved)` for boxes left where the user put them.
    pub moved: Vec<Candidate>,
    /// Valid cells not filled because a box left in place overlaps them.
    pub blocked: Vec<Candidate>,
    pub warnings: Vec<String>,
}

/// Row-major order of the grid: rows top-down, then left to right.
fn order(cell: [i64; 2]) -> (i64, i64) {
    (cell[1], cell[0])
}

/// Handle order: AutoCAD handles are hex without leading zeros, so shorter means smaller.
pub(crate) fn handle_key(h: &str) -> (usize, String) {
    (h.len(), h.to_ascii_uppercase())
}

pub(crate) fn angle_gap_deg(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

pub(super) fn diff(cells: &[Cell<'_>], mut owned: Vec<Owned<'_>>, reset: bool, placer: &Placer<'_>, grid: &Grid) -> Result<DiffOut, UbotError> {
    let by_cell: HashMap<[i64; 2], &Cell<'_>> = cells.iter().map(|c| (c.cell, c)).collect();
    owned.sort_by_cached_key(|o| (order(o.cell), handle_key(&o.ex.handle)));
    let mut out = DiffOut { add: vec![], update: vec![], remove: vec![], keep: vec![], moved: vec![], blocked: vec![], warnings: vec![] };
    let mut removed: Vec<([i64; 2], &ExistingInstance)> = Vec::new();
    let mut taken: HashSet<[i64; 2]> = HashSet::new();
    let mut left_in_place: Vec<Footprint> = Vec::new();
    for o in &owned {
        if !taken.insert(o.cell) {
            out.warnings.push(format!("DUPLICATE_CELL {}", o.ex.instance_id.as_deref().unwrap_or_default()));
            removed.push((o.cell, o.ex));
            continue;
        }
        let judged = by_cell.get(&o.cell).copied();
        let spot_mm = judged.map_or_else(|| grid.to_world(grid.center(o.cell[0], o.cell[1])), |c| c.center_mm);
        let moved = o.center_mm.dist(spot_mm) > MOVE_TOL_MM || angle_gap_deg(o.angle_deg, grid.angle_deg) >= TURN_TOL_DEG;
        let usable = judged.filter(|c| matches!(c.status, Status::Valid | Status::Uncertain));
        // Spec §14.1: a box moved by hand is kept even when its old cell is no longer valid; only `reset` lays it again.
        let hand_placed = moved && !reset;
        if hand_placed {
            let product_id = judged.map(|c| c.product.id.clone()).or_else(|| o.ex.product_id.clone()).unwrap_or_default();
            out.moved.push(Candidate {
                cell: o.cell,
                center: placer.drawing(spot_mm),
                status: Status::Info,
                reasons: vec![Reason::ManualMoved],
                product_id,
            });
            left_in_place.push(o.fp);
        }
        let Some(cell) = usable else {
            if hand_placed {
                out.keep.push(o.ex.handle.clone());
            } else {
                removed.push((o.cell, o.ex));
            }
            continue;
        };
        let target = placer.place(cell.cell, cell.center_mm, cell.product, grid.angle_deg);
        let same = o.ex.product_id.as_deref() == Some(cell.product.id.as_str()) && o.ex.block_name.eq_ignore_ascii_case(&target.block_name);
        let current = o.ex.instance_id.as_deref() == Some(target.instance_id.as_str());
        let handle = o.ex.handle.clone();
        if hand_placed {
            if same && current {
                out.keep.push(handle);
            } else {
                let placement = placer.place(cell.cell, o.center_mm, cell.product, o.angle_deg);
                out.update.push(UpdatePlacement { handle, placement });
            }
        } else if !moved && same && current {
            out.keep.push(handle);
        } else {
            out.update.push(UpdatePlacement { handle, placement: target });
        }
    }
    removed.sort_by_cached_key(|(cell, e)| (order(*cell), handle_key(&e.handle)));
    out.remove = removed.into_iter().map(|(_, e)| e.handle.clone()).collect();

    let in_place = Obstacles::new(grid, left_in_place)?;
    for cell in cells.iter().filter(|c| c.status == Status::Valid && !taken.contains(&c.cell)) {
        let fp = footprint(cell.center_mm, grid.size_mm.x, grid.size_mm.y, grid.angle_deg);
        if in_place.blocks(grid, cell.cell[0], cell.cell[1], &fp) {
            out.blocked.push(Candidate {
                cell: cell.cell,
                center: placer.drawing(cell.center_mm),
                status: Status::Invalid,
                reasons: vec![Reason::OverlapsExisting],
                product_id: cell.product.id.clone(),
            });
        } else {
            out.add.push(placer.place(cell.cell, cell.center_mm, cell.product, grid.angle_deg));
        }
    }
    Ok(out)
}
