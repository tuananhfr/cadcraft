//! TUBAB (LPC's TUBxy): swaps the references of one block for another block, keeping insert point, rotation, scale,
//! layer and attributes; a box's product tag follows the new block. `preview` lists the swaps; `commit` writes them.

use std::collections::HashMap;

use cadcraft_doc::{EntityKind, Handle};
use cadcraft_geom::Vec2;
use cadcraft_ubot::{Anchor, BlockMapping, Catalog, Product, center_from_insert, input_hash, insert_from_center, relative_scale};
use serde_json::{Value, json};

use super::read::{generated_blocks, untagged_box_product};
use super::swap_apply::{Swap, apply};
use super::swap_input::{Target, check_expected, commit_mode, handles, target};
use super::tubab_interactive::TubabM;
use crate::cmd::{CommandSpec, bad, str_param};
use crate::{Result, Session};

pub(super) const CMD: &str = "tubab";
/// Footprints closer than this are the same size.
const SIZE_TOL_MM: f64 = 1e-6;

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Swap Block", run)
        .menu(&["Tools", "U-BOT", "Swap Box Block"])
        .params(
            "{handles?: [handle] (default: the selection; references of other blocks are left out), from: block name (any case), to: existing block name, mode?: preview|commit (default preview), expectHash?: inputHash of the preview} → {mode, inputHash, from, to, summary: {selected, matched, replaced, unchanged}, changes: [{handle, blockName, insert, rotationDeg, scale, productId}], written?: {replaced}}",
        )
        .interactive(|_| Ok(Box::new(TubabM::default())))
}

#[derive(Debug, Clone, Default)]
pub(super) struct Params {
    pub handles: Vec<Handle>,
    pub from: String,
    pub to: String,
}

pub(super) struct Plan {
    pub input_hash: String,
    /// The `to` block's own spelling.
    pub to: String,
    pub selected: usize,
    pub matched: usize,
    pub swaps: Vec<Swap>,
    pub unchanged: usize,
}

fn name_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    str_param(p, key).map(str::trim).filter(|t| !t.is_empty())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let from = name_param(p, "from").ok_or_else(|| bad(CMD, "`from` (the block name to replace) is required"))?.to_string();
    let to = name_param(p, "to").ok_or_else(|| bad(CMD, "`to` (the block name to replace it with) is required"))?.to_string();
    let commit = commit_mode(CMD, p)?;
    let expect = str_param(p, "expectHash").map(str::to_string);
    let params = Params { handles: handles(s, CMD, p)?, from, to };
    let (plan, written) = if commit {
        commit_checked(s, &params, expect.as_deref(), true)?
    } else {
        let plan = plan(s, &params)?;
        s.echo(summary_line(&params, &plan));
        (plan, None)
    };
    let mut out = json!({
        "mode": if commit { "commit" } else { "preview" },
        "inputHash": plan.input_hash,
        "from": params.from,
        "to": plan.to,
        "summary": { "selected": plan.selected, "matched": plan.matched, "replaced": plan.swaps.len(), "unchanged": plan.unchanged },
        "changes": plan.swaps.iter().map(Swap::to_json).collect::<Vec<_>>(),
    });
    if !commit && expect.is_some() {
        out["warnings"] = json!(["expectHash ignored in preview"]);
    }
    if let Some(n) = written {
        out["written"] = json!({ "replaced": n });
    }
    Ok(out)
}

pub(super) fn summary_line(p: &Params, plan: &Plan) -> String {
    format!("TUBAB: {} references of {} to replace with {}, {} unchanged", plan.swaps.len(), p.from, plan.to, plan.unchanged)
}

/// The product a reference of `block` is a box of, by the catalog mapping or the generated `UBOT_<SKU>` name.
fn block_product<'a>(catalog: &'a Catalog, generated: &HashMap<String, String>, block: &str) -> Option<&'a Product> {
    let id = match catalog.mapping_for_block(block) {
        Some(m) => m.product_id.clone(),
        None => untagged_box_product(catalog, generated, block).flatten(),
    };
    id.and_then(|id| catalog.product(&id))
}

fn same_size(a: &Product, b: &Product) -> bool {
    (a.width_mm - b.width_mm).abs() <= SIZE_TOL_MM && (a.length_mm - b.length_mm).abs() <= SIZE_TOL_MM
}

fn anchor(m: Option<&BlockMapping>) -> Anchor {
    m.map_or(Anchor::Center, |m| m.anchor)
}

fn rotation_offset(m: Option<&BlockMapping>) -> f64 {
    m.map_or(0.0, |m| m.rotation_offset_deg)
}

/// Both blocks put the insert point at the same place of the box, so the LPC rule (keep the insert point) keeps
/// the box where it was too.
fn same_frame(a: Option<&BlockMapping>, b: Option<&BlockMapping>) -> bool {
    let mm = |m: Option<&BlockMapping>| m.map_or(1.0, |m| m.units_mm_per_block_unit);
    anchor(a) == anchor(b) && rotation_offset(a) == rotation_offset(b) && (anchor(a) == Anchor::Center || mm(a) == mm(b))
}

fn needs_length(m: Option<&BlockMapping>) -> bool {
    anchor(m) == Anchor::TopCenter
}

/// Insert point and rotation (radians) of the new block that keep the box centre and orientation. The INSERT scale
/// is kept, so the unit scale cancels out and the core helpers work in drawing units.
fn placement(
    t: &Target,
    from: Option<&BlockMapping>,
    to: Option<&BlockMapping>,
    lengths: [Option<f64>; 2],
    names: [&str; 2],
) -> Result<([f64; 2], f64)> {
    let i = &t.insert;
    if same_frame(from, to) {
        return Ok(([i.insert.x, i.insert.y], i.rotation));
    }
    let [from_len, to_len] = lengths;
    for (m, len, name) in [(from, from_len, names[0]), (to, to_len, names[1])] {
        if needs_length(m) && len.is_none() {
            return Err(bad(CMD, format!("{}: the box size of block {name} is unknown, so its centre cannot be kept", t.handle.hex())));
        }
    }
    let rotation_deg = i.rotation.to_degrees();
    let center =
        center_from_insert(from, from_len.unwrap_or(0.0), Vec2::new(i.insert.x, i.insert.y), rotation_deg, relative_scale(from, i.scale.x, 1.0));
    let new_deg = rotation_deg - rotation_offset(from) + rotation_offset(to);
    let insert = insert_from_center(to, to_len.unwrap_or(0.0), center, new_deg, relative_scale(to, i.scale.x, 1.0));
    let rotation = if rotation_offset(from) == rotation_offset(to) { i.rotation } else { new_deg.to_radians() };
    if !(insert.is_finite() && rotation.is_finite()) {
        return Err(bad(CMD, format!("{}: the insert point and rotation must be finite numbers", t.handle.hex())));
    }
    Ok(([insert.x, insert.y], rotation))
}

pub(super) fn plan(s: &Session, p: &Params) -> Result<Plan> {
    let d = s.doc()?;
    let to_block = d.block(&p.to).ok_or_else(|| bad(CMD, format!("no block `{}`", p.to)))?;
    if to_block.xref_path.is_some() {
        return Err(bad(CMD, "external references cannot be replaced"));
    }
    let to_name = to_block.name.clone();
    let catalog = &s.ubot.catalog;
    let generated = generated_blocks(catalog);
    let to_product = block_product(catalog, &generated, &to_name);
    let to_mapping = catalog.mapping_for_block(&to_name);
    let mut targets = Vec::new();
    for &h in &p.handles {
        let e = d.entity(h).ok_or_else(|| bad(CMD, format!("no object {}", h.hex())))?;
        if matches!(&e.kind, EntityKind::Insert(ins) if ins.block.eq_ignore_ascii_case(&p.from)) {
            targets.push(target(s, CMD, h)?);
        }
    }
    if targets.is_empty() {
        return Err(bad(CMD, format!("no references of block `{}` in the selection", p.from)));
    }
    let mut swaps = Vec::new();
    let mut unchanged = 0usize;
    for t in &targets {
        let block = t.insert.block.as_str();
        let from_mapping = catalog.mapping_for_block(block);
        let from_product = t.tag.as_ref().and_then(|tag| catalog.product(&tag.product_id)).or_else(|| block_product(catalog, &generated, block));
        // A tag must never claim a product its block no longer is (ruling R20).
        if t.tag.is_some() && to_product.is_none() {
            return Err(bad(CMD, format!("{}: target block is not a known U-BOT box ({to_name})", t.handle.hex())));
        }
        if let (Some(a), Some(b)) = (from_product, to_product)
            && !same_size(a, b)
        {
            return Err(bad(
                CMD,
                format!(
                    "{}: size differs: {} is {}x{} mm, {} is {}x{} mm; TUBAB keeps the box size (UBREPLACE with allowResize changes it)",
                    t.handle.hex(),
                    a.id,
                    a.width_mm,
                    a.length_mm,
                    b.id,
                    b.width_mm,
                    b.length_mm
                ),
            ));
        }
        let lengths = [from_product.or(to_product).map(|p| p.length_mm), to_product.or(from_product).map(|p| p.length_mm)];
        let (insert, rotation) = placement(t, from_mapping, to_mapping, lengths, [block, &to_name])?;
        let tag = t.tag.as_ref().zip(to_product).map(|(tag, product)| tag.with_product(&product.id));
        let swap = Swap { handle: t.handle, block: to_name.clone(), insert, rotation, scale: t.insert.scale, tag };
        if swap.changes(t) {
            swaps.push(swap);
        } else {
            unchanged = unchanged.saturating_add(1);
        }
    }
    let input_hash = input_hash(&json!({
        "command": CMD,
        "from": p.from,
        "to": to_name,
        "toProduct": to_product,
        "targets": targets.iter().map(Target::hash_value).collect::<Vec<_>>(),
        "changes": swaps.iter().map(Swap::to_json).collect::<Vec<_>>(),
    }));
    Ok(Plan { input_hash, to: to_name, selected: p.handles.len(), matched: targets.len(), swaps, unchanged })
}

/// Plans again and writes when the inputs still hash to `expect`; `None` written means there was nothing to change.
pub(super) fn commit_checked(s: &mut Session, p: &Params, expect: Option<&str>, echo_summary: bool) -> Result<(Plan, Option<usize>)> {
    let plan = plan(s, p)?;
    check_expected(CMD, expect, &plan.input_hash)?;
    if echo_summary {
        s.echo(summary_line(p, &plan));
    }
    if plan.swaps.is_empty() {
        s.echo("TUBAB: nothing to change.");
        return Ok((plan, None));
    }
    let n = apply(s, CMD, &plan.swaps, &[])?;
    s.echo(format!("TUBAB: replaced {n} block references."));
    Ok((plan, Some(n)))
}
