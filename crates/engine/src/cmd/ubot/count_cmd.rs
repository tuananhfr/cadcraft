//! UBCOUNT (spec §7, §14.3): U-BOT boxes per product in model space, through nested blocks and MINSERT cells,
//! never counting a cluster block itself; with `boundary`, only boxes whose footprint centre lies inside it.

use cadcraft_doc::{Handle, Space};
use cadcraft_ubot::{Counts, count, count_in};
use serde_json::{Value, json};

use super::read::{count_items, count_items_at, ring_of};
use super::units::unit_scale;
use crate::cmd::{CommandSpec, bad, str_param};
use crate::{Result, Session};

const CMD: &str = "ubcount";

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Count", run)
        .menu(&["Tools", "U-BOT", "Count Boxes"])
        .params(
            "{boundary?: handle (closed polyline in model space; only boxes whose centre is inside count), units?: mm|cm|m|in} → {rows: [{productId, name, tagged, legacy, total}], total, boundary?}",
        )
        .noundo()
}

/// Where to count: all of model space, or inside one slab boundary.
#[derive(Debug, Clone, Default)]
pub(super) struct Scope {
    pub boundary: Option<Handle>,
    pub units: Option<String>,
}

impl Scope {
    pub fn parse(cmd: &str, p: &Value) -> Result<Self> {
        let boundary = match p.get("boundary") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(Handle::parse_hex)
                    .or_else(|| v.as_u64().map(Handle))
                    .ok_or_else(|| bad(cmd, "`boundary` must be an object handle (a hex string such as \"1A2\")"))?,
            ),
        };
        Ok(Scope { boundary, units: str_param(p, "units").map(str::to_string) })
    }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let scope = Scope::parse(CMD, p)?;
    let counts = counts(s, CMD, &scope)?;
    for r in &counts.rows {
        s.echo(format!("{}: {} (tagged {}, legacy {})", r.name, r.total, r.tagged, r.legacy));
    }
    if counts.total == 0 {
        s.echo("UBCOUNT: no U-BOT boxes.");
    } else {
        s.echo(format!("UBCOUNT: {} boxes.", counts.total));
    }
    let mut out = json!({ "rows": counts.rows, "total": counts.total });
    if let Some(b) = scope.boundary {
        out["boundary"] = json!(b.hex());
    }
    Ok(out)
}

/// The boxes of model space, or of those whose footprint centre lies inside the scope's boundary.
/// Only the boundary test needs the drawing units (its arcs and the box anchors are in mm), so only it asks for them.
pub(super) fn counts(s: &Session, cmd: &str, scope: &Scope) -> Result<Counts> {
    let d = s.doc()?;
    let catalog = &s.ubot.catalog;
    let Some(h) = scope.boundary else {
        if scope.units.is_some() {
            unit_scale(s, scope.units.as_deref())?;
        }
        return Ok(count(&count_items(d, catalog), catalog));
    };
    let unit_scale = unit_scale(s, scope.units.as_deref())?;
    let e = d.entity(h).ok_or_else(|| bad(cmd, format!("no object {}", h.hex())))?;
    if d.space_of(h) != Some(Space::Model) {
        return Err(bad(cmd, format!("{} is not in model space", h.hex())));
    }
    let ring = ring_of(e).ok_or_else(|| bad(cmd, format!("the slab boundary {} must be a closed polyline", h.hex())))?;
    let items = count_items_at(d, catalog).map_err(|why| bad(cmd, why))?;
    count_in(&items, Some((&ring, unit_scale)), catalog).map_err(|e| bad(cmd, format!("boundary {}: {e}", h.hex())))
}
