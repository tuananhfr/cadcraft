//! UBFILL inputs from the drawing: the slab boundary and its layout tag, holes, checked zones and the boxes in the way.

use std::collections::{HashMap, HashSet};

use cadcraft_doc::{Drawing, Entity, EntityKind, Handle, Insert, Space, entity_bounds};
use cadcraft_geom::Vec2;
use cadcraft_ubot::{
    Catalog, ExistingInstance, ExistingReach, LayoutRecord, MAX_CLEARANCE_MM, Product, RingInput, ZoneInput, ZoneReach, ZoneType, ring_from_input,
};

use super::read::{generated_blocks, invalid_tags, ring_of, untagged_box_product, zone_ring_of, zones};
use super::xdata::{self, UbotTag};
use super::zone_id;
use crate::Result;
use crate::cmd::bad;

pub(super) const CMD: &str = "ubfill";
/// Existing boxes near the slab per run, MINSERT cells counted one by one (spec §6).
const MAX_EXISTING: u64 = 100_000;
pub(super) const MAX_HOLES: usize = 10_000;
/// Arc subdivision tolerance of the core; zones are checked with it so a rejected zone is named before the fill.
const RING_TOL_MM: f64 = 0.5;

/// What the boundary says about an earlier fill.
pub(super) enum Stored {
    None,
    /// Tagged by a fill of this very object.
    Own(LayoutRecord),
    /// Tagged by a fill of another object: this boundary is a copy (spec §14.1).
    Copied(LayoutRecord),
}

pub(super) fn boundary(d: &Drawing, h: Handle) -> Result<(RingInput, Stored)> {
    let e = d.entity(h).ok_or_else(|| bad(CMD, format!("no object {}", h.hex())))?;
    if d.space_of(h) != Some(Space::Model) {
        return Err(bad(CMD, format!("{} is not in model space", h.hex())));
    }
    let ring = ring_of(e).ok_or_else(|| bad(CMD, format!("the slab boundary {} must be a closed polyline", h.hex())))?;
    let stored = match xdata::read(&e.common) {
        Some(Ok(UbotTag::Layout { record, boundary_handle })) if boundary_handle.trim().eq_ignore_ascii_case(&h.hex()) => Stored::Own(record),
        Some(Ok(UbotTag::Layout { record, .. })) => Stored::Copied(record),
        // Writing the layout tag would erase the zone.
        Some(Ok(UbotTag::Zone { .. })) => return Err(bad(CMD, format!("{} is tagged as a U-BOT zone; select the slab boundary", h.hex()))),
        // A malformed record is replaced by the layout tag on commit.
        _ => Stored::None,
    };
    Ok((ring, stored))
}

/// Selected holes as opening zones the commit tags (ruling R15), so UBCHECK and later fills read the same geometry.
/// `clearance_mm` is this run's edge clearance and stays on the tag. A tagged zone is used anyway, so it is skipped.
pub(super) fn hole_zones(d: &Drawing, boundary: Handle, handles: &[Handle], unit_scale: f64, clearance_mm: f64) -> Result<Vec<(Handle, ZoneInput)>> {
    if handles.len() > MAX_HOLES {
        return Err(bad(CMD, format!("too many objects (at most {MAX_HOLES})")));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &h in handles.iter().filter(|h| seen.insert(**h)) {
        if h == boundary {
            return Err(bad(CMD, format!("{} is the slab boundary, not a hole", h.hex())));
        }
        let e = d.entity(h).ok_or_else(|| bad(CMD, format!("no object {}", h.hex())))?;
        if d.space_of(h) != Some(Space::Model) {
            return Err(bad(CMD, format!("{} is not in model space", h.hex())));
        }
        match xdata::read(&e.common) {
            Some(Ok(UbotTag::Zone { .. })) => continue,
            // Tagging it as a zone would erase that data.
            Some(_) => return Err(bad(CMD, format!("hole {} carries U-BOT data that is not a zone; select another object", h.hex()))),
            None => {}
        }
        let ring = zone_ring_of(e).ok_or_else(|| bad(CMD, format!("hole {} is not a closed polyline or circle", h.hex())))?;
        ring_from_input(&ring, unit_scale, RING_TOL_MM).map_err(|e| bad(CMD, format!("hole {}: {e}", h.hex())))?;
        let zone = ZoneInput { id: zone_id(h, &ring), ring, zone_type: ZoneType::Opening, clearance_mm: Some(clearance_mm), alt_product_id: None };
        out.push((h, zone));
    }
    Ok(out)
}

/// The bounding box of a zone object that is not a ring, so the reach filter can place it; `None` when unplaceable.
fn broken_zone_extent(d: &Drawing, e: &Entity) -> Option<RingInput> {
    if let EntityKind::LwPolyline(pl) = &e.kind
        && !pl.vertices.iter().all(|v| v.p.is_finite())
    {
        return None;
    }
    let b = entity_bounds(d, e, 0);
    if b.is_empty() || !(b.min.is_finite() && b.max.is_finite()) {
        return None;
    }
    Some(RingInput { vertices: vec![[b.min.x, b.min.y, 0.0], [b.max.x, b.min.y, 0.0], [b.max.x, b.max.y, 0.0], [b.min.x, b.max.y, 0.0]] })
}

/// The tagged zones in model space within `reach` of the slab, each checked so a bad one is named. A zone record on
/// a shape that is no ring is an error in reach too, never dropped; a non-finite zone is an error wherever it is.
pub(super) fn checked_zones(d: &Drawing, unit_scale: f64, reach: &ZoneReach, catalog: &Catalog, main: &Product) -> Result<Vec<ZoneInput>> {
    let named = |h: Handle, why: &dyn std::fmt::Display| bad(CMD, format!("zone {}: {why}", h.hex()));
    for (h, why) in invalid_tags(d) {
        let Some(e) = d.entity(h) else { continue };
        if !matches!(xdata::read(&e.common), Some(Ok(UbotTag::Zone { .. }))) {
            continue;
        }
        let near = match broken_zone_extent(d, e) {
            Some(extent) => reach.reaches(unit_scale, &extent).map_err(|e| named(h, &e))?,
            None => true,
        };
        if near {
            return Err(named(h, &why));
        }
    }
    let mut out = Vec::new();
    for (h, z) in zones(d) {
        if !reach.reaches(unit_scale, &z.ring).map_err(|e| named(h, &e))? {
            continue;
        }
        ring_from_input(&z.ring, unit_scale, RING_TOL_MM).map_err(|e| bad(CMD, format!("zone {}: {e}", h.hex())))?;
        if let Some(c) = z.clearance_mm.filter(|c| *c > MAX_CLEARANCE_MM) {
            return Err(bad(CMD, format!("zone {}: clearance {c} mm is over {MAX_CLEARANCE_MM} mm", h.hex())));
        }
        if z.zone_type == ZoneType::AltProduct {
            let id =
                z.alt_product_id.as_deref().ok_or_else(|| bad(CMD, format!("zone {}: an alt_product zone needs an alternative product", h.hex())))?;
            let shown: String = id.chars().take(64).collect();
            let alt =
                catalog.product(id).ok_or_else(|| bad(CMD, format!("zone {}: alternative product '{shown}' is not in the catalog", h.hex())))?;
            if alt.width_mm != main.width_mm || alt.length_mm != main.length_mm {
                return Err(bad(
                    CMD,
                    format!(
                        "zone {}: alternative product '{shown}' is {}x{} mm, the boxes are {}x{} mm",
                        h.hex(),
                        alt.width_mm,
                        alt.length_mm,
                        main.width_mm,
                        main.length_mm
                    ),
                ));
            }
        }
        out.push(z);
    }
    Ok(out)
}

/// How a model space reference takes part in the fill.
enum Role {
    /// A tagged single box: it may belong to the layout being filled.
    Instance { instance_id: String, product_id: String, layout_id: String },
    /// Only in the way: every cell of it blocks the cells it overlaps; `product_id` sizes it.
    Obstacle { product_id: Option<String> },
}

fn role(e: &Entity, ins: &Insert, catalog: &Catalog, generated: &HashMap<String, String>) -> Option<Role> {
    let single = cells(ins) <= 1;
    match xdata::read(&e.common) {
        Some(Ok(UbotTag::Instance { instance_id, product_id, layout_id, .. })) if single => {
            Some(Role::Instance { instance_id, product_id, layout_id })
        }
        // A tagged MINSERT is never edited cell by cell; it only blocks.
        Some(Ok(UbotTag::Instance { product_id, .. })) => Some(Role::Obstacle { product_id: Some(product_id) }),
        // Malformed or misplaced U-BOT data still marks a box.
        Some(_) => Some(Role::Obstacle { product_id: None }),
        // Untagged legacy or generated box blocks inserted by hand are only in the way (spec §14.1).
        None => untagged_box_product(catalog, generated, &ins.block).map(|product_id| Role::Obstacle { product_id }),
    }
}

fn cells(ins: &Insert) -> u64 {
    u64::from(ins.cols.max(1)).saturating_mul(u64::from(ins.rows.max(1)))
}

/// Which boxes of a reference to build.
#[derive(Clone, Copy)]
enum Cells {
    All,
    /// The four corner cells of a MINSERT: every other cell's footprint lies within theirs.
    Corners,
}

/// The reference's boxes: one for a tagged single box, MINSERT cells one by one for an obstacle.
fn boxes_of(e: &Entity, ins: &Insert, role: &Role, which: Cells) -> Vec<ExistingInstance> {
    let base = |instance_id, layout_id, product_id, at: Vec2| ExistingInstance {
        handle: e.handle.hex(),
        instance_id,
        layout_id,
        product_id,
        block_name: ins.block.clone(),
        insert: [at.x, at.y],
        rotation_deg: ins.rotation.to_degrees(),
        scale: ins.scale.x,
        scale_y: (ins.scale.y != ins.scale.x).then_some(ins.scale.y),
    };
    let origin = Vec2::new(ins.insert.x, ins.insert.y);
    let product_id = match role {
        Role::Instance { instance_id, product_id, layout_id } => {
            return vec![base(Some(instance_id.clone()), Some(layout_id.clone()), Some(product_id.clone()), origin)];
        }
        Role::Obstacle { product_id } => product_id,
    };
    let cell = |c: u32, r: u32| {
        let offset = Vec2::new(f64::from(c) * ins.col_spacing, f64::from(r) * ins.row_spacing).rotate(ins.rotation);
        base(None, None, product_id.clone(), origin + offset)
    };
    let (cols, rows) = (ins.cols.max(1), ins.rows.max(1));
    match which {
        Cells::Corners => {
            let mut at = vec![(0, 0), (cols - 1, 0), (0, rows - 1), (cols - 1, rows - 1)];
            at.dedup();
            at.into_iter().map(|(c, r)| cell(c, r)).collect()
        }
        Cells::All => (0..rows).flat_map(|r| (0..cols).map(move |c| (c, r))).map(|(c, r)| cell(c, r)).collect(),
    }
}

/// Boxes already in model space within `reach` of the slab for the core: tagged single boxes with their ids, everything
/// else as obstacles (malformed tags, tagged MINSERT cells one by one, untagged mapped or generated box blocks). Boxes on
/// other floors neither block nor belong to this slab, so the cap counts only these (spec §6: per run of a slab).
pub(super) fn existing(d: &Drawing, catalog: &Catalog, main: &Product, unit_scale: f64, reach: &ExistingReach) -> Result<Vec<ExistingInstance>> {
    let generated = generated_blocks(catalog);
    let mut near: Vec<(&Entity, &Insert, Role)> = Vec::new();
    let mut total = 0u64;
    for e in d.model.iter() {
        let EntityKind::Insert(ins) = &e.kind else { continue };
        let Some(role) = role(e, ins, catalog, &generated) else { continue };
        let corners = boxes_of(e, ins, &role, Cells::Corners);
        if reach.reaches(&corners, catalog, main, unit_scale).map_err(|err| bad(CMD, err.to_string()))? {
            total = total.saturating_add(cells(ins));
            near.push((e.as_ref(), ins, role));
        }
    }
    if total > MAX_EXISTING {
        return Err(bad(CMD, format!("{total} existing boxes near the slab (MINSERT cells counted), the limit is {MAX_EXISTING}")));
    }
    let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for (e, ins, role) in near {
        out.extend(boxes_of(e, ins, &role, Cells::All));
    }
    Ok(out)
}
