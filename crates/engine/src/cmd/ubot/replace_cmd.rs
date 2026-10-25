//! UBREPLACE (spec §7): swaps U-BOT boxes for another product at the same centre and orientation, keeping layer,
//! colour, attributes and the instance tag. `preview` returns the swaps; `commit` writes them in one step.

use std::collections::HashMap;

use cadcraft_doc::Handle;
use cadcraft_geom::Vec3;
use cadcraft_ubot::{ExistingInstance, Issue, Product, ReplaceRequest, ReplaceResult, input_hash, replace};
use serde_json::{Value, json};

use super::fill_cmd::placement_catalog;
use super::read::{generated_blocks, untagged_box_product};
use super::replace_interactive::ReplaceM;
use super::swap_apply::{Swap, apply, snap};
use super::swap_input::{Target, check_expected, commit_mode, handles, target};
use super::units::unit_scale;
use crate::cmd::{CommandSpec, bad, str_param};
use crate::{Result, Session};

pub(super) const CMD: &str = "ubreplace";
pub(super) const RESIZED_WARNING: &str = "box size changed: run UBCHECK to re-check cells";
/// Refusals quoted in the commit error; the preview lists them all.
const MAX_QUOTED: usize = 5;

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Replace", run)
        .menu(&["Tools", "U-BOT", "Replace Boxes"])
        .params(
            "{handles?: [handle] (default: the selection; U-BOT boxes only: tagged, legacy-mapped or UBOT_<SKU> references), to: product id, allowResize?: bool, units?: mm|cm|m|in, mode?: preview|commit (default preview), expectHash?: inputHash of the preview} → {mode, inputHash, to, summary: {targets, replaced, unchanged, refused}, changes, refused, warnings, written?: {replaced}}",
        )
        .interactive(|_| Ok(Box::new(ReplaceM::default())))
}

#[derive(Debug, Clone, Default)]
pub(super) struct Params {
    pub handles: Vec<Handle>,
    pub to: String,
    pub allow_resize: bool,
    pub units: Option<String>,
}

/// A planned replacement, ready to show or to write.
pub(super) struct Plan {
    pub input_hash: String,
    pub targets: usize,
    pub result: ReplaceResult,
    /// Only the references that change.
    pub swaps: Vec<Swap>,
    pub unchanged: usize,
    /// The generated product block to define when the drawing lacks it.
    pub generate: Vec<(Product, String)>,
    pub warnings: Vec<String>,
}

fn parse(s: &Session, p: &Value) -> Result<(Params, bool, Option<String>)> {
    let to = str_param(p, "to")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| bad(CMD, "`to` (the product id to replace with, see UBLIB) is required"))?;
    let allow_resize = match p.get("allowResize") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(CMD, "`allowResize` must be true or false"))?,
    };
    let commit = commit_mode(CMD, p)?;
    let params = Params { handles: handles(s, CMD, p)?, to: to.to_string(), allow_resize, units: str_param(p, "units").map(str::to_string) };
    Ok((params, commit, str_param(p, "expectHash").map(str::to_string)))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let (params, commit, expect) = parse(s, p)?;
    let (plan, written) = if commit {
        commit_checked(s, &params, expect.as_deref(), true)?
    } else {
        let mut plan = plan(s, &params)?;
        if expect.is_some() {
            plan.warnings.push("expectHash ignored in preview".into());
        }
        s.echo(summary_line(&plan));
        (plan, None)
    };
    Ok(response(&plan, &params, commit, written))
}

pub(super) fn summary_line(plan: &Plan) -> String {
    format!("UBREPLACE: {} boxes to replace, {} unchanged, {} refused", plan.swaps.len(), plan.unchanged, plan.result.refused.len())
}

fn response(plan: &Plan, p: &Params, commit: bool, written: Option<usize>) -> Value {
    let mut out = json!({
        "mode": if commit { "commit" } else { "preview" },
        "inputHash": plan.input_hash,
        "to": p.to,
        "summary": { "targets": plan.targets, "replaced": plan.swaps.len(), "unchanged": plan.unchanged, "refused": plan.result.refused.len() },
        "changes": plan.result.changes,
        "refused": plan.result.refused,
        "warnings": plan.warnings,
    });
    if let Some(n) = written {
        out["written"] = json!({ "replaced": n });
    }
    out
}

/// `CODE handle: message`, as warnings are printed.
pub(super) fn issue_line(i: &Issue) -> String {
    let code = serde_json::to_value(i.code).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    format!("{code} {}: {}", i.handle.as_deref().unwrap_or("-"), i.message)
}

/// A selected reference as the core sees it; anything that is not a U-BOT box is an error naming it.
fn existing(t: &Target, catalog: &cadcraft_ubot::Catalog, generated: &HashMap<String, String>) -> Result<ExistingInstance> {
    let i = &t.insert;
    let (instance_id, layout_id, product_id) = match &t.tag {
        Some(tag) => (Some(tag.instance_id.clone()), Some(tag.layout_id.clone()), Some(tag.product_id.clone())),
        None => {
            let product =
                untagged_box_product(catalog, generated, &i.block).ok_or_else(|| bad(CMD, format!("{} is not a U-BOT box", t.handle.hex())))?;
            (None, None, product)
        }
    };
    Ok(ExistingInstance {
        handle: t.handle.hex(),
        instance_id,
        layout_id,
        product_id,
        block_name: i.block.clone(),
        insert: [i.insert.x, i.insert.y],
        rotation_deg: i.rotation.to_degrees(),
        scale: i.scale.x,
        scale_y: (i.scale.y != i.scale.x).then_some(i.scale.y),
    })
}

pub(super) fn plan(s: &Session, p: &Params) -> Result<Plan> {
    if p.handles.is_empty() {
        return Err(bad(CMD, "no U-BOT boxes selected"));
    }
    let unit_scale = unit_scale(s, p.units.as_deref())?;
    let d = s.doc()?;
    let catalog = placement_catalog(&s.ubot.catalog, d);
    let to = catalog.product(&p.to).ok_or_else(|| {
        let shown: String = p.to.chars().take(64).collect();
        bad(CMD, format!("unknown product '{shown}' (see UBLIB)"))
    })?;
    let generated = generated_blocks(&catalog);
    let targets: Vec<Target> = p.handles.iter().map(|h| target(s, CMD, *h)).collect::<Result<_>>()?;
    let existing: Vec<ExistingInstance> = targets.iter().map(|t| existing(t, &catalog, &generated)).collect::<Result<_>>()?;
    let req = ReplaceRequest { unit_scale, targets: existing, to_product_id: to.id.clone(), allow_resize: p.allow_resize };
    let result = replace(&req, &catalog).map_err(|e| bad(CMD, e.to_string()))?;
    let by_handle: HashMap<String, &Target> = targets.iter().map(|t| (t.handle.hex(), t)).collect();
    let mut swaps = Vec::new();
    let mut unchanged = 0usize;
    for c in &result.changes {
        let t = by_handle.get(&c.handle).ok_or_else(|| bad(CMD, format!("no object {}", c.handle)))?;
        let i = &t.insert;
        let old_deg = i.rotation.to_degrees();
        let [sx, sy] = c.scale_xy;
        let sz = if i.scale.z < 0.0 { -c.scale } else { c.scale };
        let swap = Swap {
            handle: t.handle,
            block: c.block_name.clone(),
            insert: [snap(i.insert.x, c.insert[0]), snap(i.insert.y, c.insert[1])],
            rotation: if snap(old_deg, c.rotation_deg) == old_deg { i.rotation } else { c.rotation_deg.to_radians() },
            scale: Vec3::new(snap(i.scale.x, sx), snap(i.scale.y, sy), snap(i.scale.z, sz)),
            tag: t.tag.as_ref().map(|tag| tag.with_product(&c.product_id)),
        };
        if swap.changes(t) {
            swaps.push(swap);
        } else {
            unchanged = unchanged.saturating_add(1);
        }
    }
    // Every change names the same block: the product's mapped block, else its generated one.
    let generate = match swaps.first() {
        Some(sw) if d.block(&sw.block).is_none() => vec![(to.clone(), sw.block.clone())],
        _ => Vec::new(),
    };
    let mut warnings: Vec<String> = result.warnings.iter().map(issue_line).collect();
    // Cells around a box that changed size are re-checked by UBCHECK (ruling R22), not here.
    if result.changes.iter().any(|c| c.resized) {
        warnings.push(RESIZED_WARNING.into());
    }
    let input_hash = input_hash(&json!({
        "command": CMD,
        "unitScale": unit_scale,
        "to": to,
        "allowResize": p.allow_resize,
        "targets": targets.iter().map(Target::hash_value).collect::<Vec<_>>(),
        "changes": result.changes,
        "refused": result.refused,
    }));
    Ok(Plan { input_hash, targets: targets.len(), result, swaps, unchanged, generate, warnings })
}

/// Plans again and writes when the inputs still hash to `expect` and no box is refused; `None` written means there
/// was nothing to change.
pub(super) fn commit_checked(s: &mut Session, p: &Params, expect: Option<&str>, echo_summary: bool) -> Result<(Plan, Option<usize>)> {
    let plan = plan(s, p)?;
    check_expected(CMD, expect, &plan.input_hash)?;
    if echo_summary {
        s.echo(summary_line(&plan));
    }
    let refused = &plan.result.refused;
    if !refused.is_empty() {
        let quoted: Vec<String> = refused.iter().take(MAX_QUOTED).map(|i| format!("{}: {}", i.handle.as_deref().unwrap_or("-"), i.message)).collect();
        let list = quoted.join("; ");
        let count = refused.len();
        return Err(if refused.iter().any(|i| i.message.starts_with("size differs")) {
            bad(CMD, format!("{count} box(es) refused, nothing replaced: {list}; pass allowResize: true to change the box size"))
        } else {
            bad(CMD, format!("{count} box(es) refused, nothing replaced: {list}"))
        });
    }
    if plan.swaps.is_empty() {
        s.echo("UBREPLACE: nothing to change.");
        return Ok((plan, None));
    }
    let n = apply(s, CMD, &plan.swaps, &plan.generate)?;
    s.echo(format!("UBREPLACE: replaced {n} boxes."));
    Ok((plan, Some(n)))
}
