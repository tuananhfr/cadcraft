//! UBFILL / UBOT (spec §7): lays U-BOT boxes on a slab. `preview` returns the diff; `commit` writes it in one step.

use cadcraft_doc::{Drawing, Handle};
use cadcraft_ubot::{Catalog, FillRequest, FillResult, LayoutRecord, fill, geometry_hash};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::fill_apply::{Written, apply};
use super::fill_input::{CMD, MAX_HOLES, Stored, boundary, checked_zones, existing, holes};
use super::fill_interactive::FillM;
use super::state::FreshLayout;
use super::units::unit_scale;
use crate::cmd::{CommandSpec, bad, str_param};
use crate::{Result, Session};

/// Stored and new grids closer than this are the same grid (float noise after a save).
const SAME_MM: f64 = 1e-6;
const SAME_DEG: f64 = 1e-9;

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Fill", run)
        .menu(&["Tools", "U-BOT", "Fill Boxes"])
        .alias(&["ubot"])
        .params(
            "{boundary: handle (closed LWPOLYLINE), holes?: [handle] (closed polylines or circles; tagged zones are always used), product: id, rib?: mm, pitchX?: mm, pitchY?: mm, angle?: deg, origin?: [x, y] (centre of cell 0,0), edgeClearance?: mm, exclusionClearance?: mm, units?: mm|cm|m|in, reset?: bool, mode?: preview|commit (default preview), expectHash?: inputHash of the preview} → {mode, inputHash, layoutId, previousLayoutId, reset, layout, summary, add, update, remove, problems, warnings, written?: {added, updated, removed}}",
        )
        .interactive(|_| Ok(Box::new(FillM::default())))
}

/// UBFILL parameters after parsing; the interactive command builds the same thing.
#[derive(Debug, Clone, Default)]
pub(super) struct Params {
    pub boundary: Handle,
    pub holes: Vec<Handle>,
    /// `zones` was passed: it is ignored (every tagged zone is used) and reported.
    pub zones_given: bool,
    pub product: String,
    pub rib: Option<f64>,
    pub pitch_x: Option<f64>,
    pub pitch_y: Option<f64>,
    pub angle: f64,
    pub origin: Option<[f64; 2]>,
    pub edge_clearance: Option<f64>,
    pub exclusion_clearance: Option<f64>,
    pub units: Option<String>,
    /// `None`: reset when the stored grid differs from this run's (ruling R4).
    pub reset: Option<bool>,
}

/// A computed fill, ready to show or to write.
pub(super) struct Plan {
    pub boundary: Handle,
    pub unit_scale: f64,
    pub result: FillResult,
    /// What the commit writes on the boundary (`revision` already bumped).
    pub record: LayoutRecord,
    pub previous: Option<String>,
    pub reset: bool,
    /// The grid moved since the stored fill, so this run resets (ruling R4).
    pub grid_changed: bool,
    /// Nothing to write: no box changes and the boundary tag already says this.
    pub noop: bool,
    /// The session catalog with mappings to blocks the drawing lacks dropped from their products.
    pub catalog: Catalog,
}

fn handle_of(v: &Value) -> Option<Handle> {
    v.as_str().and_then(Handle::parse_hex).or_else(|| v.as_u64().map(Handle))
}

fn number(p: &Value, key: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| bad(CMD, format!("`{key}` must be a number"))),
    }
}

fn parse(p: &Value) -> Result<(Params, bool, Option<String>)> {
    let boundary = p
        .get("boundary")
        .and_then(handle_of)
        .ok_or_else(|| bad(CMD, "`boundary` must be the handle of a closed polyline (a hex string such as 2F)"))?;
    let holes = match p.get("holes") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) if a.len() > MAX_HOLES => return Err(bad(CMD, format!("too many objects (at most {MAX_HOLES})"))),
        Some(Value::Array(a)) => {
            a.iter().map(|v| handle_of(v).ok_or_else(|| bad(CMD, format!("`holes`: {v} is not an object handle")))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(CMD, "`holes` must be an array of object handles")),
    };
    let product = str_param(p, "product")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad(CMD, "`product` (a product id, see UBLIB) is required"))?;
    let origin = match p.get("origin") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_array().map(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>()) {
            Some(Some(xy)) if xy.len() == 2 => xy.first().zip(xy.get(1)).map(|(x, y)| [*x, *y]),
            _ => return Err(bad(CMD, "`origin` must be [x, y]")),
        },
    };
    let reset = match p.get("reset") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(CMD, "`reset` must be true or false"))?),
    };
    let commit = match str_param(p, "mode").map(|m| m.trim().to_ascii_lowercase()).as_deref() {
        None | Some("preview") => false,
        Some("commit") => true,
        Some(other) => {
            let shown: String = other.chars().take(20).collect();
            return Err(bad(CMD, format!("unknown mode '{shown}' (use preview or commit)")));
        }
    };
    let params = Params {
        boundary,
        holes,
        zones_given: p.get("zones").is_some_and(|z| !z.is_null()),
        product: product.to_string(),
        rib: number(p, "rib")?,
        pitch_x: number(p, "pitchX")?,
        pitch_y: number(p, "pitchY")?,
        angle: number(p, "angle")?.unwrap_or(0.0),
        origin,
        edge_clearance: number(p, "edgeClearance")?,
        exclusion_clearance: number(p, "exclusionClearance")?,
        units: str_param(p, "units").map(str::to_string),
        reset,
    };
    Ok((params, commit, str_param(p, "expectHash").map(str::to_string)))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let (params, commit, expect) = parse(p)?;
    let (plan, written) = if commit {
        commit_checked(s, &params, expect.as_deref(), true)?
    } else {
        let plan = plan(s, &params)?;
        s.echo(summary_line(&plan.result));
        (plan, None)
    };
    Ok(response(&plan, commit, written.as_ref()))
}

/// Plans again and writes when the inputs still hash to `expect`; `None` written means there was nothing to change.
pub(super) fn commit_checked(s: &mut Session, params: &Params, expect: Option<&str>, echo_summary: bool) -> Result<(Plan, Option<Written>)> {
    let plan = plan(s, params)?;
    if let Some(expect) = expect.filter(|e| *e != plan.result.input_hash) {
        let shown: String = expect.chars().take(64).collect();
        return Err(bad(CMD, format!("CONFLICT: inputs changed since preview (expected {shown}, now {})", plan.result.input_hash)));
    }
    if echo_summary {
        s.echo(summary_line(&plan.result));
    }
    if plan.noop {
        s.echo("UBFILL: nothing to change.");
        return Ok((plan, None));
    }
    let w = apply(s, &plan)?;
    s.echo(format!("UBFILL: wrote {} new, {} updated and {} removed boxes.", w.added, w.updated, w.removed));
    Ok((plan, Some(w)))
}

pub(super) fn summary_line(r: &FillResult) -> String {
    let m = &r.summary;
    format!("UBFILL: +{} boxes, -{} boxes, {} updated, {} invalid cells, {} uncertain cells", m.added, m.removed, m.updated, m.invalid, m.uncertain)
}

fn response(plan: &Plan, commit: bool, written: Option<&Written>) -> Value {
    let r = &plan.result;
    let mut out = json!({
        "mode": if commit { "commit" } else { "preview" },
        "inputHash": r.input_hash,
        "layoutId": plan.record.layout_id,
        "previousLayoutId": plan.previous,
        "reset": plan.reset,
        "layout": plan.record,
        "summary": r.summary,
        "add": r.add,
        "update": r.update,
        "remove": r.remove,
        "problems": r.problems,
        "warnings": r.warnings,
    });
    if let Some(w) = written {
        out["written"] = json!({ "added": w.added, "updated": w.updated, "removed": w.removed });
    }
    out
}

/// The session catalog where a product mapping to a block the drawing lacks is dropped (spec §7: then the
/// generated `UBOT_<SKU>` block is used); legacy detection by block name keeps working.
fn placement_catalog(catalog: &Catalog, d: &Drawing) -> Catalog {
    let mut c = catalog.clone();
    for m in c.mappings.iter_mut().filter(|m| m.product_id.is_some() && d.block(&m.block_name).is_none()) {
        m.product_id = None;
    }
    c
}

fn angle_gap_deg(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Origin, angle, pitch or units differ, so the boxes would all read as moved by hand.
fn grid_differs(stored: &LayoutRecord, now: &LayoutRecord) -> bool {
    let off = |a: f64, b: f64| (a - b).abs() * now.unit_scale > SAME_MM;
    off(stored.origin[0], now.origin[0])
        || off(stored.origin[1], now.origin[1])
        || angle_gap_deg(stored.angle_deg, now.angle_deg) > SAME_DEG
        || (stored.pitch_x_mm - now.pitch_x_mm).abs() > SAME_MM
        || (stored.pitch_y_mm - now.pitch_y_mm).abs() > SAME_MM
        || (stored.unit_scale - now.unit_scale).abs() > 1e-12 * now.unit_scale
}

/// 16 hex of SHA-256 over the boundary handle, its geometry and the time, so copies and refills get fresh ids.
fn new_layout_id(boundary: Handle, geometry_hash: &str) -> String {
    let digest = Sha256::digest(format!("{}|{geometry_hash}|{}", boundary.hex(), crate::cmd::ubplot::now_ms()).as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// A fresh id is remembered for its boundary and geometry, so the commit after a preview gets the same id and hash.
fn fresh_layout_id(s: &mut Session, boundary: Handle, geometry_hash: &str, previous: Option<&str>) -> Result<String> {
    let key = FreshLayout {
        doc_uid: s.state()?.uid,
        boundary: boundary.hex(),
        geometry_hash: geometry_hash.to_string(),
        previous: previous.map(str::to_string),
        layout_id: String::new(),
    };
    if let Some(f) = &s.ubot.fresh_layout
        && (FreshLayout { layout_id: String::new(), ..f.clone() }) == key
    {
        return Ok(f.layout_id.clone());
    }
    let layout_id = new_layout_id(boundary, geometry_hash);
    s.ubot.fresh_layout = Some(FreshLayout { layout_id: layout_id.clone(), ..key });
    Ok(layout_id)
}

pub(super) fn plan(s: &mut Session, p: &Params) -> Result<Plan> {
    let unit_scale = unit_scale(s, p.units.as_deref())?;
    let d = s.doc()?;
    let catalog = placement_catalog(&s.ubot.catalog, d);
    let main = catalog.product(&p.product).ok_or_else(|| {
        let shown: String = p.product.chars().take(64).collect();
        bad(CMD, format!("unknown product '{shown}' (see UBLIB)"))
    })?;
    let (ring, stored) = boundary(d, p.boundary)?;
    let holes = holes(d, p.boundary, &p.holes)?;
    let zones = checked_zones(d, unit_scale, &catalog, main)?;
    let existing = existing(d, &catalog)?;
    let geometry = geometry_hash(unit_scale, &ring, &holes, &zones).map_err(|e| bad(CMD, format!("boundary {}: {e}", p.boundary.hex())))?;
    let (own, previous) = match stored {
        Stored::Own(r) => (Some(r), None),
        Stored::Copied(r) => (None, Some(r.layout_id)),
        Stored::None => (None, None),
    };
    let layout_id = match &own {
        Some(r) => r.layout_id.clone(),
        None => fresh_layout_id(s, p.boundary, &geometry, previous.as_deref())?,
    };
    let mut req = FillRequest {
        unit_scale,
        layout_id,
        previous_layout_id: previous.clone(),
        reset: p.reset.unwrap_or(false),
        boundary: ring,
        holes,
        zones,
        product_id: p.product.clone(),
        rulepack_id: None,
        rib_mm: p.rib,
        pitch_x_mm: p.pitch_x,
        pitch_y_mm: p.pitch_y,
        angle_deg: p.angle,
        origin: p.origin,
        edge_clearance_mm: p.edge_clearance,
        exclusion_clearance_mm: p.exclusion_clearance,
        existing,
    };
    let core = |req: &FillRequest| fill(req, &catalog).map_err(|e| bad(CMD, e.to_string()));
    let mut result = core(&req)?;
    let grid_changed = own.as_ref().is_some_and(|r| grid_differs(r, &result.layout));
    if grid_changed && p.reset.is_none() {
        req.reset = true;
        result = core(&req)?;
        result.warnings.push("GRID_CHANGED".into());
    }
    if p.zones_given {
        result.warnings.push("ZONES_PARAM_IGNORED: UBFILL uses every tagged zone in model space".into());
    }
    let tag_current = own.as_ref().is_some_and(|r| LayoutRecord { revision: r.revision, ..result.layout.clone() } == *r);
    let no_box_change = result.add.is_empty() && result.update.is_empty() && result.remove.is_empty();
    let noop = no_box_change && tag_current;
    let revision = match &own {
        Some(r) if noop => r.revision,
        Some(r) => r.revision.saturating_add(1),
        None => 1,
    };
    let record = LayoutRecord { revision, ..result.layout.clone() };
    Ok(Plan {
        boundary: p.boundary,
        unit_scale,
        result,
        record,
        previous,
        reset: req.reset,
        grid_changed: grid_changed && p.reset.is_none(),
        noop,
        catalog,
    })
}
