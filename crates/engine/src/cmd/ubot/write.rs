//! Writing U-BOT results: one edit on a clone of the drawing, published only when it all succeeds.

use std::sync::Arc;

use cadcraft_color::Color;
use cadcraft_doc::{Block, Common, Drawing, Entity, EntityKind, Handle, LwPolyline};
use cadcraft_geom::{PolyVertex, Vec2};
use cadcraft_ubot::Product;
use serde_json::Value;

use super::xdata::{self, UbotTag};
use crate::{EngineError, Result, Session};

pub const LAYER: &str = "UBOT";
const LAYER_ACI: i16 = 55;

/// Adds layer `UBOT` (colour 55, as the LPC template) when the drawing lacks it; an existing one is left as it is.
pub fn ensure_layer_ubot(drawing: &mut Drawing) {
    if drawing.ensure_layer(LAYER)
        && let Some(layer) = drawing.layer_mut(LAYER)
    {
        layer.color = Color::from_aci(LAYER_ACI);
    }
}

/// Defines `block_name` as the product's footprint square in mm (one block unit = 1 mm, as the core's
/// `relative_scale` assumes), origin at its centre; references take scale 1/unit_scale. An existing block is kept.
pub fn ensure_product_block(drawing: &mut Drawing, product: &Product, block_name: &str) -> Result<()> {
    if drawing.block(block_name).is_some() {
        return Ok(());
    }
    if block_name.trim().is_empty() {
        return Err(EngineError::Other("a box block needs a name".into()));
    }
    let (w, l) = (product.width_mm, product.length_mm);
    if !(w.is_finite() && l.is_finite() && w > 0.0 && l > 0.0) {
        return Err(EngineError::Other(format!("product '{}' has no usable footprint", product.id)));
    }
    ensure_layer_ubot(drawing);
    let (hw, hl) = (w / 2.0, l / 2.0);
    let vertices =
        [(-hw, -hl), (hw, -hl), (hw, hl), (-hw, hl)].map(|(x, y)| PolyVertex { p: Vec2::new(x, y), bulge: 0.0, start_width: 0.0, end_width: 0.0 });
    let outline = LwPolyline { vertices: vertices.to_vec(), closed: true, const_width: 0.0, elevation: 0.0, plinegen: false };
    let mut block = Block::new(block_name);
    block.description = product.name.clone();
    // INSUNITS 4: the block is in millimetres.
    block.units = 4;
    let handle = drawing.new_handle();
    block.entities.push(Entity { handle, common: Common { layer: LAYER.into(), ..Common::default() }, kind: EntityKind::LwPolyline(outline) });
    drawing.blocks.insert(block_name.to_string(), Arc::new(block));
    Ok(())
}

/// Puts `tag` on object `handle` (replacing any earlier U-BOT record).
pub fn set_tag(drawing: &mut Drawing, handle: Handle, tag: &UbotTag) -> Result<()> {
    let mut written = Ok(());
    drawing.modify_entity(handle, |e| written = xdata::write(&mut e.common, tag))?;
    written.map_err(|why| EngineError::Other(format!("{}: {why}", handle.hex())))
}

/// Runs `edit` on a copy of the drawing and makes it current only when `edit` succeeds: one undo step, and an
/// error leaves the drawing untouched. Callers with nothing to change should not call it (it always publishes).
pub fn commit(s: &mut Session, edit: impl FnOnce(&mut Drawing) -> Result<Value>) -> Result<Value> {
    let mut draft: Drawing = s.doc()?.clone();
    let value = edit(&mut draft)?;
    s.state_mut()?.doc = Arc::new(draft);
    Ok(value)
}
