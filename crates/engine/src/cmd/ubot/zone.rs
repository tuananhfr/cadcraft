//! UBZONE: tags closed polylines and circles as layout zones (type, clearance, alternative product).

use cadcraft_doc::{EntityKind, Handle, Space};
use cadcraft_ubot::{MAX_CLEARANCE_MM, ZoneType, input_hash, ring_from_input};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::read::zone_ring_of;
use super::units::unit_scale;
use super::write::{commit, set_tag};
use super::xdata::{self, UbotTag, parse_zone_type, zone_type_name};
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::cmd::{CommandSpec, bad, str_param, targets};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const CMD: &str = "ubzone";
const MAX_ZONES: usize = 10_000;
/// Arc subdivision used only to validate the shape.
const CHECK_TOL_MM: f64 = 0.5;
const TYPE_KEYWORDS: [&str; 6] = ["Exclusion", "Solid", "Column", "Wall", "Opening", "AltProduct"];

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Zone", run)
        .menu(&["Tools", "U-BOT", "Tag Zone"])
        .params(
            "{handles?: [handle] (closed LWPOLYLINE or CIRCLE; default: the selection), type: exclusion|solid|column|wall|opening|alt_product, clearanceMm?: number (default: the rulepack's), altProduct?: product id (alt_product only), units?: mm|cm|m|in} → {zones: [{handle, zoneId}]}",
        )
        .interactive(|_| Ok(Box::new(ZoneM::default())))
}

/// 16 hex of SHA-256 over the object's handle and its ring, so a copied or reshaped zone gets a new id.
pub fn zone_id(handle: Handle, ring: &cadcraft_ubot::RingInput) -> String {
    let digest = Sha256::digest(format!("{}{}", handle.hex(), input_hash(ring)).as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let type_name = str_param(p, "type").ok_or_else(|| bad(CMD, "`type` is required (exclusion, solid, column, wall, opening or alt_product)"))?;
    let zone_type = parse_zone_type(type_name).ok_or_else(|| {
        let shown: String = type_name.chars().take(20).collect();
        bad(CMD, format!("unknown zone type '{shown}' (use exclusion, solid, column, wall, opening or alt_product)"))
    })?;
    let clearance_mm = match p.get("clearanceMm") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_f64().filter(|c| c.is_finite() && (0.0..=MAX_CLEARANCE_MM).contains(c)) {
            Some(c) => Some(c),
            None => return Err(bad(CMD, format!("`clearanceMm` must be a number from 0 to {MAX_CLEARANCE_MM}"))),
        },
    };
    let alt_product_id = match (zone_type, str_param(p, "altProduct").map(str::trim).filter(|a| !a.is_empty())) {
        (ZoneType::AltProduct, Some(id)) => {
            if s.ubot.catalog.product(id).is_none() {
                let shown: String = id.chars().take(64).collect();
                return Err(bad(CMD, format!("unknown product '{shown}' (see UBLIB)")));
            }
            Some(id.to_string())
        }
        (ZoneType::AltProduct, None) => return Err(bad(CMD, "an alt_product zone needs `altProduct` (a product id)")),
        (_, Some(_)) => return Err(bad(CMD, "`altProduct` applies to alt_product zones only")),
        (_, None) => None,
    };
    if p.get("handles").and_then(Value::as_array).is_some_and(|h| h.len() > MAX_ZONES) || s.selection().len() > MAX_ZONES {
        return Err(bad(CMD, format!("too many objects (at most {MAX_ZONES})")));
    }
    let mut seen = std::collections::HashSet::new();
    let handles: Vec<Handle> = targets(s, p)?.into_iter().filter(|h| seen.insert(*h)).collect();
    if handles.is_empty() {
        return Err(bad(CMD, "select closed polylines or circles to tag as zones"));
    }
    // Shape checks need a scale; tagging itself does not, so an unset INSUNITS falls back to 1.
    let scale = match str_param(p, "units") {
        Some(units) => unit_scale(s, Some(units))?,
        None => unit_scale(s, None).unwrap_or(1.0),
    };
    let mut planned = Vec::with_capacity(handles.len());
    {
        let d = s.doc()?;
        for &h in &handles {
            let e = d.entity(h).ok_or_else(|| bad(CMD, format!("no object {}", h.hex())))?;
            if d.space_of(h) != Some(Space::Model) {
                return Err(bad(CMD, format!("{} is not in model space", h.hex())));
            }
            if crate::cmd::curves::is_locked(s, h) {
                return Err(bad(CMD, format!("{} is on a locked layer", h.hex())));
            }
            let current = existing_zone(h, xdata::read(&e.common))?;
            let ring = zone_ring_of(e).ok_or_else(|| bad(CMD, not_a_zone(h, &e.kind)))?;
            ring_from_input(&ring, scale, CHECK_TOL_MM).map_err(|err| bad(CMD, format!("{}: {err}", h.hex())))?;
            planned.push((h, zone_id(h, &ring), current));
        }
    }
    let tag_for = |id: &str| UbotTag::Zone { zone_id: id.to_string(), zone_type, clearance_mm, alt_product_id: alt_product_id.clone() };
    let zones: Vec<Value> = planned.iter().map(|(h, id, _)| json!({"handle": h.hex(), "zoneId": id})).collect();
    if planned.iter().all(|(_, id, current)| current.as_ref() == Some(&tag_for(id))) {
        s.echo(format!("{} zone(s) already tagged as {}; nothing to change.", planned.len(), zone_type_name(zone_type)));
        return Ok(json!({ "zones": zones }));
    }
    commit(s, |d| {
        for (h, id, _) in &planned {
            set_tag(d, *h, &tag_for(id))?;
        }
        Ok(Value::Null)
    })?;
    s.echo(format!("{} zone(s) tagged as {}.", planned.len(), zone_type_name(zone_type)));
    Ok(json!({ "zones": zones }))
}

/// The object's zone record, if any. Any other U-BOT record, readable or not, is refused: overwriting it loses data.
fn existing_zone(h: Handle, record: Option<std::result::Result<UbotTag, String>>) -> Result<Option<UbotTag>> {
    match record {
        None => Ok(None),
        Some(Ok(zone @ UbotTag::Zone { .. })) => Ok(Some(zone)),
        Some(Ok(UbotTag::Layout { .. })) => {
            Err(bad(CMD, format!("{} carries a U-BOT layout record (a slab boundary); it cannot become a zone", h.hex())))
        }
        Some(Ok(UbotTag::Instance { .. })) => Err(bad(CMD, format!("{} carries a U-BOT box record; it cannot become a zone", h.hex()))),
        Some(Err(why)) => Err(bad(CMD, format!("{} carries unreadable U-BOT data ({why}); it is not overwritten", h.hex()))),
    }
}

fn not_a_zone(h: Handle, kind: &EntityKind) -> String {
    match kind {
        EntityKind::LwPolyline(_) => format!("{} is an open polyline; a zone must be closed", h.hex()),
        EntityKind::Ellipse(_) | EntityKind::Hatch(_) | EntityKind::Spline(_) => {
            format!("{}: {} zones are not supported yet; trace it with a closed polyline (BOUNDARY)", h.hex(), kind.type_name())
        }
        _ => format!("{} ({}) is not a closed polyline or circle", h.hex(), kind.type_name()),
    }
}

#[derive(Default)]
struct ZoneM {
    selection: SelectPhase,
    zone_type: Option<ZoneType>,
}

impl ZoneM {
    fn apply(&self, s: &mut Session, zone_type: ZoneType, alt: Option<&str>) -> Result<Step> {
        let handles: Vec<String> = self.selection.picked.iter().map(|h| h.hex()).collect();
        let mut p = json!({ "handles": handles, "type": zone_type_name(zone_type) });
        if let Some(alt) = alt {
            p["altProduct"] = json!(alt);
        }
        run(s, &p)?;
        Ok(Step::Done)
    }

    fn type_prompt() -> Prompt {
        Prompt::new("Zone type", Accept::TEXT).kw(&TYPE_KEYWORDS).default("Exclusion")
    }
}

impl Interactive for ZoneM {
    fn name(&self) -> &'static str {
        "UBZONE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if s.selection().len() > MAX_ZONES {
            return Err(bad(CMD, format!("too many objects (at most {MAX_ZONES})")));
        }
        self.selection = SelectPhase::begin(s);
        Ok(Step::Continue)
    }
    fn prompt(&self, s: &Session) -> Prompt {
        if !self.selection.done {
            return if self.selection.removing { self.selection.prompt() } else { Prompt::new("Select closed polylines or circles", Accept::SELECT) };
        }
        match self.zone_type {
            None => Self::type_prompt(),
            Some(_) => {
                let first = s.ubot.catalog.products.first().map(|p| p.id.clone());
                let prompt = Prompt::new("Alternative product id", Accept::TEXT);
                match first {
                    Some(id) => prompt.default(id),
                    None => prompt,
                }
            }
        }
    }
    fn input(&mut self, s: &mut Session, input: Input) -> Result<Step> {
        if matches!(input, Input::Cancel) {
            return Ok(Step::Cancel);
        }
        if !self.selection.done {
            if let Input::Pick(handles) = &input
                && self.selection.picked.len().saturating_add(handles.len()) > MAX_ZONES
            {
                return Err(bad(CMD, format!("too many objects (at most {MAX_ZONES})")));
            }
            return match self.selection.feed(s, &input)? {
                SelOutcome::Empty => Ok(Step::Done),
                SelOutcome::More | SelOutcome::Done(_) => Ok(Step::Continue),
            };
        }
        match self.zone_type {
            None => {
                let keyword = match &input {
                    Input::Enter => Some("Exclusion".to_string()),
                    Input::Text(t) | Input::Keyword(t) => {
                        Self::type_prompt().match_keyword(t).or_else(|| parse_zone_type(t).map(|z| keyword_of(z).to_string()))
                    }
                    _ => None,
                };
                let Some(zone_type) = keyword.as_deref().and_then(zone_type_of_keyword) else {
                    s.echo("Choose one of the listed zone types.");
                    return Ok(Step::Continue);
                };
                if zone_type == ZoneType::AltProduct {
                    self.zone_type = Some(zone_type);
                    return Ok(Step::Continue);
                }
                self.apply(s, zone_type, None)
            }
            Some(zone_type) => {
                let id = match &input {
                    Input::Text(t) | Input::Keyword(t) if !t.trim().is_empty() => t.trim().to_string(),
                    Input::Enter => match s.ubot.catalog.products.first() {
                        Some(p) => p.id.clone(),
                        None => return Ok(Step::Cancel),
                    },
                    _ => return Ok(Step::Continue),
                };
                if s.ubot.catalog.product(&id).is_none() {
                    let shown: String = id.chars().take(64).collect();
                    s.echo(format!("Unknown product '{shown}' (see UBLIB)."));
                    return Ok(Step::Continue);
                }
                self.apply(s, zone_type, Some(&id))
            }
        }
    }
}

fn keyword_of(z: ZoneType) -> &'static str {
    match z {
        ZoneType::Exclusion => "Exclusion",
        ZoneType::Solid => "Solid",
        ZoneType::Column => "Column",
        ZoneType::Wall => "Wall",
        ZoneType::Opening => "Opening",
        ZoneType::AltProduct => "AltProduct",
    }
}

fn zone_type_of_keyword(k: &str) -> Option<ZoneType> {
    [ZoneType::Exclusion, ZoneType::Solid, ZoneType::Column, ZoneType::Wall, ZoneType::Opening, ZoneType::AltProduct]
        .into_iter()
        .find(|z| keyword_of(*z) == k)
}
