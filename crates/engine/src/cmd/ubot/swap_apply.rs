//! Writes UBREPLACE / TUBAB swaps: new block, placement and tag per reference, attributes kept by tag, one commit.

use cadcraft_doc::{Attrib, Drawing, EntityKind, Handle};
use cadcraft_geom::Vec3;
use cadcraft_ubot::Product;
use serde_json::{Value, json};

use super::swap_input::Target;
use super::write::{commit, ensure_product_block};
use super::xdata::{self, UbotTag};
use crate::cmd::bad;
use crate::cmd::blocks::attribs::synced;
use crate::{EngineError, Result, Session};

/// Attributes written per run, over all references (as BLOCKREPLACE).
const MAX_ATTRIBUTES: usize = 1_000_000;

/// What one reference becomes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Swap {
    pub handle: Handle,
    pub block: String,
    pub insert: [f64; 2],
    /// Radians, as the drawing stores it.
    pub rotation: f64,
    pub scale: Vec3,
    /// The instance tag to write; `None` leaves the reference's U-BOT data as it is.
    pub tag: Option<UbotTag>,
}

impl Swap {
    /// Whether writing this would change `t` at all (block names compare in any case).
    pub fn changes(&self, t: &Target) -> bool {
        let i = &t.insert;
        let tag_changes = match (&self.tag, &t.tag) {
            (Some(new), Some(old)) => *new != old.with_product(&old.product_id),
            (Some(_), None) => true,
            (None, _) => false,
        };
        !i.block.eq_ignore_ascii_case(&self.block)
            || i.insert.x != self.insert[0]
            || i.insert.y != self.insert[1]
            || i.rotation != self.rotation
            || i.scale != self.scale
            || tag_changes
    }

    pub fn to_json(&self) -> Value {
        let product_id = match &self.tag {
            Some(UbotTag::Instance { product_id, .. }) => Some(product_id.as_str()),
            _ => None,
        };
        json!({
            "handle": self.handle.hex(),
            "blockName": self.block,
            "insert": self.insert,
            "rotationDeg": self.rotation.to_degrees(),
            "scale": [self.scale.x, self.scale.y, self.scale.z],
            "productId": product_id,
        })
    }
}

/// `new` unless it only differs from `old` by float noise, so an unchanged value stays bit for bit.
pub(super) fn snap(old: f64, new: f64) -> f64 {
    if (old - new).abs() <= 1e-9 * old.abs().max(1.0) { old } else { new }
}

/// The attribute definitions a reference of `block` takes, as BLOCKREPLACE reads them.
fn definitions(block: &cadcraft_doc::Block) -> Vec<&Attrib> {
    block
        .entities
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::AttDef(a) if !a.constant => Some(a),
            _ => None,
        })
        .take(MAX_ATTRIBUTES.saturating_add(1))
        .collect()
}

/// Writes `swaps` in one commit (one undo step) after defining the `generate` product blocks the drawing lacks.
/// Attributes are synchronised with the new block's definitions by tag, as BLOCKREPLACE does.
pub(super) fn apply(s: &mut Session, cmd: &str, swaps: &[Swap], generate: &[(Product, String)]) -> Result<usize> {
    commit(s, |d: &mut Drawing| {
        for (product, name) in generate {
            ensure_product_block(d, product, name)?;
        }
        let mut attributes = 0usize;
        for sw in swaps {
            let block = d.block(&sw.block).cloned().ok_or_else(|| bad(cmd, format!("no block `{}`", sw.block)))?;
            let defs = definitions(&block);
            attributes = attributes.saturating_add(defs.len());
            if attributes > MAX_ATTRIBUTES {
                return Err(bad(cmd, format!("too many attributes (at most {MAX_ATTRIBUTES})")));
            }
            let mut tagged = Ok(());
            d.modify_entity(sw.handle, |e| {
                if let EntityKind::Insert(ins) = &mut e.kind {
                    ins.block = block.name.clone();
                    ins.insert = Vec3::new(sw.insert[0], sw.insert[1], ins.insert.z);
                    ins.rotation = sw.rotation;
                    ins.scale = sw.scale;
                    ins.attribs = synced(&block, &defs, ins);
                }
                if let Some(tag) = &sw.tag {
                    tagged = xdata::write(&mut e.common, tag);
                }
            })?;
            tagged.map_err(|why| EngineError::Other(format!("{}: {why}", sw.handle.hex())))?;
        }
        Ok(Value::Null)
    })?;
    Ok(swaps.len())
}
