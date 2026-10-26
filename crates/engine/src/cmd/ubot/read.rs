//! Drawing → core requests: rings, tagged zones, boxes already placed and count items, all from model space.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use cadcraft_doc::{Common, Drawing, Entity, EntityKind, Handle, Insert, entity_bounds};
use cadcraft_geom::{Bounds2, Mat3, Vec2};
use cadcraft_ubot::{Catalog, CountItem, CountSource, ExistingInstance, RingInput, ZoneInput, center_from_insert_xy, relative_scale};

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
                scale_y: (ins.scale.y != ins.scale.x).then_some(ins.scale.y),
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

/// References expanded while locating boxes (MINSERT cells and nested copies one by one); past it the walk stops.
/// No more than the core counts in one call, so a drawing too large to locate is reported as such.
pub const MAX_LOCATE_WORK: usize = cadcraft_ubot::MAX_COUNT_ITEMS;

/// Box references to count: tagged, mapped or generated-block references are items (MINSERT cells multiply); any
/// other reference is a cluster whose boxes are counted through it, never itself.
pub fn count_items(drawing: &Drawing, catalog: &Catalog) -> Vec<CountItem> {
    let mut walker = Walker::new(drawing, catalog);
    let totals: Totals = walker.contents(drawing.model.iter().map(|e| e.as_ref()), 0);
    totals.into_items()
}

/// [`count_items`] one box cell at a time with its footprint centre in drawing units, for counting inside a slab
/// boundary (spec §14.3). `Err` when the drawing expands to more than [`MAX_LOCATE_WORK`] references.
pub fn count_items_at(drawing: &Drawing, catalog: &Catalog) -> Result<Vec<(CountItem, [f64; 2])>, String> {
    let mut walker = Walker::new(drawing, catalog);
    let located: Located = walker.contents(drawing.model.iter().map(|e| e.as_ref()), 0);
    if located.overflow {
        return Err(format!("the boxes expand to more than {MAX_LOCATE_WORK} block references"));
    }
    Ok(located.items.into_iter().map(|(item, c)| ((*item).clone(), [c.x, c.y])).collect())
}

fn cells(ins: &Insert) -> u64 {
    u64::from(ins.cols.max(1)).saturating_mul(u64::from(ins.rows.max(1)))
}

/// The cells of one reference; MINSERT copies are offset along its rotated columns and rows, unscaled (as drawn).
struct Cells<'a> {
    ins: &'a Insert,
    base: Vec2,
}

impl Cells<'_> {
    fn count(&self) -> u64 {
        cells(self.ins)
    }

    fn transforms(&self) -> impl Iterator<Item = Mat3> + '_ {
        let (ins, m) = (self.ins, self.ins.transform(self.base));
        (0..ins.rows.max(1)).flat_map(move |r| {
            (0..ins.cols.max(1)).map(move |c| {
                let off = Vec2::new(ins.col_spacing * f64::from(c), ins.row_spacing * f64::from(r)).rotate(ins.rotation);
                Mat3::translate(off).then_before(m)
            })
        })
    }
}

/// What the references of one space or block add up to, in its coordinates.
trait Contents: Default {
    /// Box centres are wanted (a box of unknown size costs one extents pass over its block).
    const LOCATED: bool;
    /// A box reference: `item` stands for one cell, `centre` is the footprint centre in the box block's coordinates.
    fn add_box(&mut self, item: CountItem, cells: &Cells<'_>, centre: Vec2, budget: &mut usize);
    /// A cluster reference whose block holds `inner`.
    fn add_cluster(&mut self, inner: &Self, cells: &Cells<'_>, budget: &mut usize);
}

/// Boxes summed per (product, block, legacy).
#[derive(Default)]
struct Totals(BTreeMap<(Option<String>, String, bool), u64>);

impl Totals {
    fn add(&mut self, key: (Option<String>, String, bool), n: u64) {
        let slot = self.0.entry(key).or_insert(0);
        *slot = slot.saturating_add(n);
    }

    fn into_items(self) -> Vec<CountItem> {
        self.0
            .into_iter()
            .map(|((product_id, block_name, legacy), multiplier)| CountItem {
                product_id,
                block_name,
                multiplier,
                source: if legacy { CountSource::Legacy } else { CountSource::Tagged },
            })
            .collect()
    }
}

impl Contents for Totals {
    const LOCATED: bool = false;

    fn add_box(&mut self, item: CountItem, cells: &Cells<'_>, _centre: Vec2, _budget: &mut usize) {
        let n = item.multiplier.saturating_mul(cells.count());
        self.add((item.product_id, item.block_name, item.source == CountSource::Legacy), n);
    }

    fn add_cluster(&mut self, inner: &Self, cells: &Cells<'_>, _budget: &mut usize) {
        let k = cells.count();
        for (key, n) in &inner.0 {
            self.add(key.clone(), n.saturating_mul(k));
        }
    }
}

/// Every box cell with its centre; `overflow` once the walk ran out of budget.
#[derive(Default)]
struct Located {
    items: Vec<(Rc<CountItem>, Vec2)>,
    overflow: bool,
}

impl Located {
    fn spend(&mut self, n: u64, budget: &mut usize) -> bool {
        match usize::try_from(n) {
            Ok(n) if !self.overflow && n <= *budget => {
                *budget -= n;
                true
            }
            _ => {
                self.overflow = true;
                false
            }
        }
    }
}

impl Contents for Located {
    const LOCATED: bool = true;

    fn add_box(&mut self, item: CountItem, cells: &Cells<'_>, centre: Vec2, budget: &mut usize) {
        if !self.spend(cells.count(), budget) {
            return;
        }
        let item = Rc::new(item);
        self.items.extend(cells.transforms().map(|m| (Rc::clone(&item), m.apply(centre))));
    }

    fn add_cluster(&mut self, inner: &Self, cells: &Cells<'_>, budget: &mut usize) {
        self.overflow |= inner.overflow;
        let n = u64::try_from(inner.items.len()).unwrap_or(u64::MAX).saturating_mul(cells.count());
        if !self.spend(n, budget) {
            return;
        }
        for m in cells.transforms() {
            self.items.extend(inner.items.iter().map(|(item, c)| (Rc::clone(item), m.apply(*c))));
        }
    }
}

struct Walker<'a, C> {
    drawing: &'a Drawing,
    catalog: &'a Catalog,
    generated: HashMap<String, String>,
    /// Contents of one reference of a block, by upper-case block name; each block is walked once.
    memo: HashMap<String, Rc<C>>,
    visiting: HashSet<String>,
    /// Extents centres of box blocks, by upper-case block name.
    extents: HashMap<String, Option<Vec2>>,
    budget: usize,
}

impl<'a, C: Contents> Walker<'a, C> {
    fn new(drawing: &'a Drawing, catalog: &'a Catalog) -> Self {
        Walker {
            drawing,
            catalog,
            generated: generated_blocks(catalog),
            memo: HashMap::new(),
            visiting: HashSet::new(),
            extents: HashMap::new(),
            budget: MAX_LOCATE_WORK,
        }
    }

    /// The reference as a box (multiplier for one cell), or `None` when it is not a box.
    fn item_of(&self, e: &Entity, ins: &Insert) -> Option<CountItem> {
        if let Some(Ok(UbotTag::Instance { product_id, count_multiplier, .. })) = xdata::read(&e.common) {
            let multiplier = u64::try_from(count_multiplier).unwrap_or(1).max(1);
            return Some(CountItem { product_id: Some(product_id), block_name: ins.block.clone(), multiplier, source: CountSource::Tagged });
        }
        let product_id = untagged_box_product(self.catalog, &self.generated, &ins.block)?;
        Some(CountItem { product_id, block_name: ins.block.clone(), multiplier: 1, source: CountSource::Legacy })
    }

    fn contents<'e>(&mut self, entities: impl Iterator<Item = &'e Entity>, depth: usize) -> C {
        let mut out = C::default();
        for e in entities {
            let EntityKind::Insert(ins) = &e.kind else { continue };
            let base = self.drawing.block(&ins.block).map_or(Vec2::ZERO, |b| b.base.xy());
            let cells = Cells { ins, base };
            match self.item_of(e, ins) {
                Some(item) => {
                    let centre = if C::LOCATED { self.box_centre(&item, ins, base) } else { base };
                    out.add_box(item, &cells, centre, &mut self.budget);
                }
                None => {
                    let inner = self.block_contents(&ins.block, depth + 1);
                    out.add_cluster(&inner, &cells, &mut self.budget);
                }
            }
        }
        out
    }

    fn block_contents(&mut self, name: &str, depth: usize) -> Rc<C> {
        let d = self.drawing;
        let Some(block) = d.block(name) else { return Rc::default() };
        let key = block.name.to_ascii_uppercase();
        if let Some(c) = self.memo.get(&key) {
            return Rc::clone(c);
        }
        // A cycle or a nesting past the limit contributes nothing (a hostile file); the walk stays finite.
        if depth > MAX_COUNT_DEPTH || !self.visiting.insert(key.clone()) {
            return Rc::default();
        }
        let c = Rc::new(self.contents(block.entities.iter().map(|e| e.as_ref()), depth));
        self.visiting.remove(&key);
        self.memo.insert(key, Rc::clone(&c));
        c
    }

    /// The footprint centre in the box block's coordinates: through the catalog anchor when the box size is known,
    /// as the core places boxes, else the centre of the block's extents.
    fn box_centre(&mut self, item: &CountItem, ins: &Insert, base: Vec2) -> Vec2 {
        let catalog = self.catalog;
        let mapping = catalog.mapping_for_block(&ins.block);
        let product = match item.product_id.as_deref() {
            Some(id) => catalog.product(id),
            None => mapping.and_then(|m| m.product_id.as_deref()).and_then(|id| catalog.product(id)),
        };
        if let Some(p) = product {
            // At insert scale 1 and 1 mm per drawing unit the core's offset comes out in block units.
            let k = relative_scale(mapping, 1.0, 1.0);
            let c = base + center_from_insert_xy(mapping, p.length_mm, Vec2::ZERO, 0.0, [k, k]);
            if c.is_finite() {
                return c;
            }
        }
        self.extents_centre(&ins.block).unwrap_or(base)
    }

    fn extents_centre(&mut self, name: &str) -> Option<Vec2> {
        let d = self.drawing;
        let block = d.block(name)?;
        let key = block.name.to_ascii_uppercase();
        if let Some(c) = self.extents.get(&key) {
            return *c;
        }
        let b =
            block.entities.iter().filter(|e| !matches!(e.kind, EntityKind::AttDef(_))).fold(Bounds2::EMPTY, |b, e| b.union(&entity_bounds(d, e, 1)));
        let c = (!b.is_empty()).then(|| b.center()).filter(|c| c.is_finite());
        self.extents.insert(key, c);
        c
    }
}
