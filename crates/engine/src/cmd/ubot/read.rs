//! Drawing → core requests: rings, tagged zones, boxes already placed and count items, all from model space.

use std::collections::{BTreeMap, HashMap, HashSet};

use cadcraft_doc::{Common, Drawing, Entity, EntityKind, Handle, Insert};
use cadcraft_ubot::{Catalog, CountItem, CountSource, ExistingInstance, RingInput, ZoneInput};

use super::xdata::{self, UbotTag};

/// Block nesting followed by [`count_items`]; deeper references are not counted.
pub const MAX_COUNT_DEPTH: usize = 32;

/// Planar OCS: `Some(true)` when the extrusion is -Z (the object is mirrored), `None` when tilted.
fn mirrored(common: &Common) -> Option<bool> {
    let e = common.extrusion;
    if !(e.x.abs() < 1e-9 && e.y.abs() < 1e-9 && e.z.is_finite() && e.z != 0.0) {
        return None;
    }
    Some(e.z < 0.0)
}

/// A closed LWPOLYLINE as a core ring (bulges kept). Explicitly closed by a repeated last vertex counts too.
pub fn ring_of(entity: &Entity) -> Option<RingInput> {
    let EntityKind::LwPolyline(pl) = &entity.kind else { return None };
    let flip = mirrored(&entity.common)?;
    let mut vertices: Vec<[f64; 3]> = pl.vertices.iter().map(|v| if flip { [-v.p.x, v.p.y, -v.bulge] } else { [v.p.x, v.p.y, v.bulge] }).collect();
    if !pl.closed {
        let repeated = vertices.len() >= 4 && matches!((vertices.first(), vertices.last()), (Some(a), Some(b)) if a[0] == b[0] && a[1] == b[1]);
        if !repeated {
            return None;
        }
        vertices.pop();
    }
    Some(RingInput { vertices })
}

/// A zone ring: a closed LWPOLYLINE, or a CIRCLE as two half arcs (bulge 1) the core subdivides.
pub fn zone_ring_of(entity: &Entity) -> Option<RingInput> {
    match &entity.kind {
        EntityKind::Circle(c) => {
            let flip = mirrored(&entity.common)?;
            let (x, y, r) = (if flip { -c.center.x } else { c.center.x }, c.center.y, c.radius);
            if !(x.is_finite() && y.is_finite() && r.is_finite() && r > 0.0) {
                return None;
            }
            Some(RingInput { vertices: vec![[x - r, y, 1.0], [x + r, y, 1.0]] })
        }
        _ => ring_of(entity),
    }
}

/// Every model space object carrying `LPC_UBOT` data, in drawing order.
pub fn tags(drawing: &Drawing) -> Vec<(Handle, Result<UbotTag, String>)> {
    drawing.model.iter().filter_map(|e| Some((e.handle, xdata::read(&e.common)?))).collect()
}

/// Every zone tag in model space whose object is a valid zone shape; see [`invalid_tags`] for the rest.
pub fn zones(drawing: &Drawing) -> Vec<(Handle, ZoneInput)> {
    drawing
        .model
        .iter()
        .filter_map(|e| match xdata::read(&e.common) {
            Some(Ok(UbotTag::Zone { zone_id, zone_type, clearance_mm, alt_product_id })) => {
                let ring = zone_ring_of(e)?;
                Some((e.handle, ZoneInput { id: zone_id, ring, zone_type, clearance_mm, alt_product_id }))
            }
            _ => None,
        })
        .collect()
}

/// Model space objects whose `LPC_UBOT` data is malformed or sits on the wrong kind of object.
pub fn invalid_tags(drawing: &Drawing) -> Vec<(Handle, String)> {
    drawing
        .model
        .iter()
        .filter_map(|e| {
            let why = match xdata::read(&e.common)? {
                Err(why) => why,
                Ok(UbotTag::Zone { .. }) if zone_ring_of(e).is_none() => "a zone must be a closed polyline or a circle".into(),
                Ok(UbotTag::Layout { .. }) if ring_of(e).is_none() => "a slab boundary must be a closed polyline".into(),
                Ok(UbotTag::Instance { .. }) if !matches!(e.kind, EntityKind::Insert(_)) => "a box must be a block reference".into(),
                Ok(_) => return None,
            };
            Some((e.handle, why))
        })
        .collect()
}

/// Product ids by the upper-case name of their generated `UBOT_<SKU>` block (spec §7).
pub fn generated_blocks(catalog: &Catalog) -> HashMap<String, String> {
    catalog.products.iter().map(|p| (Catalog::default_block_name(p), p.id.clone())).collect()
}

/// The product an untagged reference of `block` is a box of: `Some(None)` for a mapped (legacy) block whose product
/// the mapping leaves out, `Some(Some(id))` for a generated `UBOT_<SKU>` block, `None` when it is no box block.
pub fn untagged_box_product(catalog: &Catalog, generated: &HashMap<String, String>, block: &str) -> Option<Option<String>> {
    if catalog.mapping_for_block(block).is_some() {
        return Some(None);
    }
    generated.get(&block.to_ascii_uppercase()).map(|id| Some(id.clone()))
}

/// Box references in model space: those with an instance tag, and untagged ones whose block has a (legacy) mapping
/// or is a generated `UBOT_<SKU>` block (then the product is known).
pub fn existing_instances(drawing: &Drawing, catalog: &Catalog) -> Vec<ExistingInstance> {
    let generated = generated_blocks(catalog);
    drawing
        .model
        .iter()
        .filter_map(|e| {
            let EntityKind::Insert(ins) = &e.kind else { return None };
            let base = |instance_id, layout_id, product_id| ExistingInstance {
                handle: e.handle.hex(),
                instance_id,
                layout_id,
                product_id,
                block_name: ins.block.clone(),
                insert: [ins.insert.x, ins.insert.y],
                rotation_deg: ins.rotation.to_degrees(),
                scale: ins.scale.x,
            };
            match xdata::read(&e.common) {
                Some(Ok(UbotTag::Instance { instance_id, product_id, layout_id, .. })) => {
                    Some(base(Some(instance_id), Some(layout_id), Some(product_id)))
                }
                _ => untagged_box_product(catalog, &generated, &ins.block).map(|product_id| base(None, None, product_id)),
            }
        })
        .collect()
}

/// Box references to count: tagged, mapped or generated-block references are items (MINSERT cells multiply); any
/// other reference is a cluster whose boxes are counted through it, never itself.
pub fn count_items(drawing: &Drawing, catalog: &Catalog) -> Vec<CountItem> {
    let mut walker = Walker { drawing, catalog, generated: generated_blocks(catalog), memo: HashMap::new(), visiting: HashSet::new() };
    let mut out = Vec::new();
    for e in drawing.model.iter() {
        let EntityKind::Insert(ins) = &e.kind else { continue };
        let cells = cells(ins);
        match walker.item_of(e, ins) {
            Some(mut item) => {
                item.multiplier = item.multiplier.saturating_mul(cells);
                out.push(item);
            }
            None => {
                for item in walker.block_items(&ins.block, 1) {
                    out.push(CountItem { multiplier: item.multiplier.saturating_mul(cells), ..item });
                }
            }
        }
    }
    out
}

fn cells(ins: &Insert) -> u64 {
    u64::from(ins.cols.max(1)).saturating_mul(u64::from(ins.rows.max(1)))
}

struct Walker<'a> {
    drawing: &'a Drawing,
    catalog: &'a Catalog,
    generated: HashMap<String, String>,
    /// Boxes in one reference of a block, by upper-case block name; each block is walked once.
    memo: HashMap<String, Vec<CountItem>>,
    visiting: HashSet<String>,
}

impl Walker<'_> {
    /// The reference as a box (multiplier for one cell), or `None` when it is not a box.
    fn item_of(&self, e: &Entity, ins: &Insert) -> Option<CountItem> {
        if let Some(Ok(UbotTag::Instance { product_id, count_multiplier, .. })) = xdata::read(&e.common) {
            let multiplier = u64::try_from(count_multiplier).unwrap_or(1).max(1);
            return Some(CountItem { product_id: Some(product_id), block_name: ins.block.clone(), multiplier, source: CountSource::Tagged });
        }
        let product_id = untagged_box_product(self.catalog, &self.generated, &ins.block)?;
        Some(CountItem { product_id, block_name: ins.block.clone(), multiplier: 1, source: CountSource::Legacy })
    }

    fn block_items(&mut self, name: &str, depth: usize) -> Vec<CountItem> {
        let Some(block) = self.drawing.block(name) else { return Vec::new() };
        let key = block.name.to_ascii_uppercase();
        if let Some(items) = self.memo.get(&key) {
            return items.clone();
        }
        // A cycle or a nesting past the limit contributes nothing (a hostile file); the walk stays finite.
        if depth > MAX_COUNT_DEPTH || !self.visiting.insert(key.clone()) {
            return Vec::new();
        }
        let block = block.clone();
        let mut sum: BTreeMap<(Option<String>, String, bool), u64> = BTreeMap::new();
        let mut add = |item: CountItem, by: u64| {
            let slot = sum.entry((item.product_id, item.block_name, item.source == CountSource::Legacy)).or_insert(0);
            *slot = slot.saturating_add(item.multiplier.saturating_mul(by));
        };
        for e in block.entities.iter() {
            let EntityKind::Insert(ins) = &e.kind else { continue };
            let cells = cells(ins);
            match self.item_of(e, ins) {
                Some(item) => add(item, cells),
                None => {
                    for item in self.block_items(&ins.block, depth + 1) {
                        add(item, cells);
                    }
                }
            }
        }
        self.visiting.remove(&key);
        let items: Vec<CountItem> = sum
            .into_iter()
            .map(|((product_id, block_name, legacy), multiplier)| CountItem {
                product_id,
                block_name,
                multiplier,
                source: if legacy { CountSource::Legacy } else { CountSource::Tagged },
            })
            .collect();
        self.memo.insert(key, items.clone());
        items
    }
}
