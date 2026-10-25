//! Writes a UBFILL plan: blocks, new and changed box references, erased boxes and the boundary's layout tag, all in
//! one commit (one undo step).

use std::collections::{BTreeMap, HashSet};

use cadcraft_doc::{Common, Drawing, EntityKind, Handle, Insert, Space};
use cadcraft_geom::Vec3;
use cadcraft_ubot::{Catalog, Placement, Product, relative_scale};
use serde_json::Value;

use super::fill_cmd::Plan;
use super::fill_input::CMD;
use super::write::{LAYER, commit, ensure_layer_ubot, ensure_product_block, set_tag};
use super::xdata::{self, UbotTag};
use crate::cmd::bad;
use crate::{EngineError, Result, Session};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Written {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

fn parse(h: &str) -> Result<Handle> {
    Handle::parse_hex(h).ok_or_else(|| bad(CMD, format!("no object {h}")))
}

/// Uniform INSERT scale that draws the block at true size: the inverse of the core's relative scale at 1.
fn insert_scale(catalog: &Catalog, block: &str, unit_scale: f64) -> Result<f64> {
    let s = 1.0 / relative_scale(catalog.mapping_for_block(block), 1.0, unit_scale);
    if s.is_finite() && s > 0.0 { Ok(s) } else { Err(bad(CMD, format!("block {block} has no usable scale"))) }
}

fn instance_tag(p: &Placement, layout_id: &str, count_multiplier: i16) -> UbotTag {
    UbotTag::Instance { instance_id: p.instance_id.clone(), product_id: p.product_id.clone(), layout_id: layout_id.to_string(), count_multiplier }
}

fn tag_error(h: Handle, why: String) -> EngineError {
    EngineError::Other(format!("{}: {why}", h.hex()))
}

/// Applies `plan` as one edit; refuses up front when the boundary, a hole to tag or a box to change sits on a locked
/// layer.
pub(super) fn apply(s: &mut Session, plan: &Plan) -> Result<Written> {
    let r = &plan.result;
    let remove: Vec<Handle> = r.remove.iter().map(|h| parse(h)).collect::<Result<_>>()?;
    let update: Vec<(Handle, &Placement)> = r.update.iter().map(|u| Ok((parse(&u.handle)?, &u.placement))).collect::<Result<_>>()?;
    let holes = plan.hole_tags.iter().map(|(h, _)| *h);
    for h in std::iter::once(plan.boundary).chain(holes).chain(remove.iter().copied()).chain(update.iter().map(|u| u.0)) {
        if crate::cmd::curves::is_locked(s, h) {
            return Err(bad(CMD, format!("{} is on a locked layer", h.hex())));
        }
    }
    let mut products: BTreeMap<&str, &Product> = BTreeMap::new();
    for p in r.add.iter().chain(update.iter().map(|u| u.1)) {
        if !products.contains_key(p.block_name.as_str()) {
            let product = plan.catalog.product(&p.product_id).ok_or_else(|| bad(CMD, format!("unknown product '{}' (see UBLIB)", p.product_id)))?;
            products.insert(&p.block_name, product);
        }
    }
    let mut scales: BTreeMap<&str, f64> = BTreeMap::new();
    for name in products.keys() {
        scales.insert(name, insert_scale(&plan.catalog, name, plan.unit_scale)?);
    }
    let scale_of = |name: &str| scales.get(name).copied().ok_or_else(|| bad(CMD, format!("block {name} has no usable scale")));
    let layout_id = plan.record.layout_id.as_str();
    let written = Written { added: r.add.len(), updated: update.len(), removed: remove.len() };
    let (doc_uid, boundary_hex) = (s.state()?.uid, plan.boundary.hex());
    commit(s, |d: &mut Drawing| {
        if !products.is_empty() {
            ensure_layer_ubot(d);
        }
        for (name, product) in &products {
            ensure_product_block(d, product, name)?;
        }
        for &h in &remove {
            d.remove_entity(h).ok_or_else(|| bad(CMD, format!("no object {}", h.hex())))?;
        }
        for &(h, p) in &update {
            let scale = scale_of(&p.block_name)?;
            let multiplier = match d.entity(h).map(|e| xdata::read(&e.common)) {
                Some(Some(Ok(UbotTag::Instance { count_multiplier, .. }))) => count_multiplier,
                _ => 1,
            };
            let tag = instance_tag(p, layout_id, multiplier);
            let mut tagged = Ok(());
            d.modify_entity(h, |e| {
                if let EntityKind::Insert(ins) = &mut e.kind {
                    // Attributes belong to the old block's definitions; all of them go with it.
                    if !ins.block.eq_ignore_ascii_case(&p.block_name) {
                        ins.attribs.clear();
                    }
                    ins.insert = Vec3::new(p.insert[0], p.insert[1], ins.insert.z);
                    ins.rotation = p.rotation_deg.to_radians();
                    ins.block = p.block_name.clone();
                    ins.scale = Vec3::new(scale, scale, scale);
                }
                tagged = xdata::write(&mut e.common, &tag);
            })?;
            tagged.map_err(|why| tag_error(h, why))?;
        }
        for p in &r.add {
            let scale = scale_of(&p.block_name)?;
            let mut common = Common { layer: LAYER.into(), ..Common::default() };
            xdata::write(&mut common, &instance_tag(p, layout_id, 1)).map_err(|why| EngineError::Other(format!("{}: {why}", p.instance_id)))?;
            let insert = Insert {
                block: p.block_name.clone(),
                insert: Vec3::new(p.insert[0], p.insert[1], 0.0),
                scale: Vec3::new(scale, scale, scale),
                rotation: p.rotation_deg.to_radians(),
                attribs: Vec::new(),
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            };
            d.add(&Space::Model, common, EntityKind::Insert(insert))?;
        }
        set_tag(d, plan.boundary, &UbotTag::Layout { record: plan.record.clone(), boundary_handle: plan.boundary.hex() })?;
        for (h, tag) in &plan.hole_tags {
            set_tag(d, *h, tag)?;
        }
        Ok(Value::Null)
    })?;
    // The id is on the boundary now; erased boxes leave the selection.
    s.ubot.fresh_layout.retain(|f| !(f.doc_uid == doc_uid && f.boundary == boundary_hex));
    if !remove.is_empty() {
        let gone: HashSet<Handle> = remove.into_iter().collect();
        let kept: Vec<Handle> = s.selection().into_iter().filter(|h| !gone.contains(h)).collect();
        s.set_selection(kept);
    }
    Ok(written)
}
