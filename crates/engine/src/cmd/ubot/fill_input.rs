//! UBFILL inputs from the drawing: the slab boundary and its layout tag, holes, checked zones and the boxes in the way.

use std::collections::HashSet;

use cadcraft_doc::{Drawing, EntityKind, Handle, Insert, Space};
use cadcraft_geom::Vec2;
use cadcraft_ubot::{Catalog, ExistingInstance, LayoutRecord, MAX_CLEARANCE_MM, Product, RingInput, ZoneInput, ZoneType, ring_from_input};

use super::read::{ring_of, zone_ring_of, zones};
use super::xdata::{self, UbotTag};
use crate::Result;
use crate::cmd::bad;

pub(super) const CMD: &str = "ubfill";
/// Existing boxes per run, MINSERT cells counted one by one (spec §6).
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

/// Untagged closed polylines and circles are holes; a tagged zone is used anyway, so selecting it changes nothing.
pub(super) fn holes(d: &Drawing, boundary: Handle, handles: &[Handle]) -> Result<Vec<RingInput>> {
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
        if matches!(xdata::read(&e.common), Some(Ok(UbotTag::Zone { .. }))) {
            continue;
        }
        out.push(zone_ring_of(e).ok_or_else(|| bad(CMD, format!("hole {} is not a closed polyline or circle", h.hex())))?);
    }
    Ok(out)
}

/// Every tagged zone in model space (the core filters them by geometry), each checked so a bad one is named.
pub(super) fn checked_zones(d: &Drawing, unit_scale: f64, catalog: &Catalog, main: &Product) -> Result<Vec<ZoneInput>> {
    let mut out = Vec::new();
    for (h, z) in zones(d) {
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

fn role(e: &cadcraft_doc::Entity, ins: &Insert, catalog: &Catalog) -> Option<Role> {
    let single = cells(ins) <= 1;
    match xdata::read(&e.common) {
        Some(Ok(UbotTag::Instance { instance_id, product_id, layout_id, .. })) if single => {
            Some(Role::Instance { instance_id, product_id, layout_id })
        }
        // A tagged MINSERT is never edited cell by cell; it only blocks.
        Some(Ok(UbotTag::Instance { product_id, .. })) => Some(Role::Obstacle { product_id: Some(product_id) }),
        // Malformed or misplaced U-BOT data still marks a box.
        Some(_) => Some(Role::Obstacle { product_id: None }),
        None if catalog.mapping_for_block(&ins.block).is_some() => Some(Role::Obstacle { product_id: None }),
        None => None,
    }
}

fn cells(ins: &Insert) -> u64 {
    u64::from(ins.cols.max(1)).saturating_mul(u64::from(ins.rows.max(1)))
}

/// Boxes already in model space for the core: tagged single boxes with their ids, everything else as obstacles
/// (malformed tags, tagged MINSERT cells one by one, untagged mapped blocks).
pub(super) fn existing(d: &Drawing, catalog: &Catalog) -> Result<Vec<ExistingInstance>> {
    let refs: Vec<(&cadcraft_doc::Entity, &Insert, Role)> = d
        .model
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::Insert(ins) => role(e, ins, catalog).map(|r| (e.as_ref(), ins, r)),
            _ => None,
        })
        .collect();
    let total = refs.iter().fold(0u64, |n, (_, ins, _)| n.saturating_add(cells(ins)));
    if total > MAX_EXISTING {
        return Err(bad(CMD, format!("{total} existing boxes (MINSERT cells counted), the limit is {MAX_EXISTING}")));
    }
    let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for (e, ins, role) in refs {
        let base = |instance_id, layout_id, product_id, at: Vec2| ExistingInstance {
            handle: e.handle.hex(),
            instance_id,
            layout_id,
            product_id,
            block_name: ins.block.clone(),
            insert: [at.x, at.y],
            rotation_deg: ins.rotation.to_degrees(),
            scale: ins.scale.x,
        };
        let origin = Vec2::new(ins.insert.x, ins.insert.y);
        match role {
            Role::Instance { instance_id, product_id, layout_id } => out.push(base(Some(instance_id), Some(layout_id), Some(product_id), origin)),
            Role::Obstacle { product_id } => {
                for r in 0..ins.rows.max(1) {
                    for c in 0..ins.cols.max(1) {
                        let offset = Vec2::new(f64::from(c) * ins.col_spacing, f64::from(r) * ins.row_spacing).rotate(ins.rotation);
                        out.push(base(None, None, product_id.clone(), origin + offset));
                    }
                }
            }
        }
    }
    Ok(out)
}
