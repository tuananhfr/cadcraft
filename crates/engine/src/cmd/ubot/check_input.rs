//! UBCHECK inputs from model space: layouts on tagged boundaries, every tagged zone and every box. Each object is
//! looked at first, so one the core could not use becomes an issue on its handle instead of failing the check (R10).

use std::collections::{HashMap, HashSet};

use cadcraft_doc::{Drawing, Entity, Handle, entity_bounds};
use cadcraft_geom::Vec2;
use cadcraft_ubot::{
    Catalog, CheckInstance, CheckRequest, ExistingInstance, Issue, IssueCode, LayoutInput, LayoutRecord, MAX_CLEARANCE_MM, RingInput, Severity,
    ZoneInput, ZoneType, center_from_insert_xy, relative_scale, ring_from_input,
};

use super::read::{existing_instances, invalid_tags, ring_of, zones};
use super::xdata::{self, UbotTag, zone_type_name};

/// Arc tolerance of the core check.
const RING_TOL_MM: f64 = 0.5;
/// Boundary vertices of one layout the core accepts (spec §6).
const MAX_BOUNDARY_VERTICES: usize = 20_000;
/// Box positions past this (mm) leave no room for the footprint maths.
const MAX_COORD_MM: f64 = 1e15;
/// Relative box scales past this make the footprint overflow.
const MAX_RELATIVE_SCALE: f64 = 1e9;
/// Stored pitches past this (mm) put cell centres out of range.
const MAX_PITCH_MM: f64 = 1e9;
/// File values quoted in messages are cut to this many characters.
const MAX_QUOTED: usize = 64;

pub(super) struct Gathered {
    pub request: CheckRequest,
    /// About objects left out of the request, and zone data the core does not judge.
    pub issues: Vec<Issue>,
}

fn issue(code: IssueCode, handle: String, at: [f64; 2], message: String) -> Issue {
    Issue { code, severity: Severity::Error, handle: Some(handle), at, message }
}

fn shown(v: &str) -> String {
    v.chars().take(MAX_QUOTED).collect()
}

/// A point to zoom to: the object's extents centre, else the origin.
fn spot(d: &Drawing, e: &Entity) -> [f64; 2] {
    let b = entity_bounds(d, e, 0);
    let c = b.center();
    if !b.is_empty() && c.is_finite() { [c.x, c.y] } else { [0.0, 0.0] }
}

fn first_vertex(ring: &RingInput) -> [f64; 2] {
    ring.vertices.first().filter(|v| v[0].is_finite() && v[1].is_finite()).map_or([0.0, 0.0], |v| [v[0], v[1]])
}

pub(super) fn gather(d: &Drawing, catalog: &Catalog, unit_scale: f64) -> Gathered {
    let mut issues = Vec::new();
    let unreadable = invalid_tags(d);
    let unreadable_hex: HashMap<String, &str> = unreadable.iter().map(|(h, why)| (h.hex(), why.as_str())).collect();

    let mut instances = Vec::new();
    // Boxes already reported, by the core or here: one XDATA_INVALID per object.
    let mut boxes: HashSet<String> = HashSet::new();
    for base in existing_instances(d, catalog) {
        boxes.insert(base.handle.clone());
        if let Some(why) = unplaceable(&base, catalog, unit_scale) {
            let at = if base.insert.iter().all(|c| c.is_finite()) { base.insert } else { [0.0, 0.0] };
            let mut message = format!("the box cannot be checked: {why}");
            if let Some(data) = unreadable_hex.get(&base.handle) {
                message.push_str(&format!("; its U-BOT data cannot be used either: {data}"));
            }
            issues.push(issue(IssueCode::XdataInvalid, base.handle.clone(), at, message));
            continue;
        }
        // Legacy means no LPC_UBOT record at all; a box whose record cannot be read is reported as that alone.
        let has_record = Handle::parse_hex(&base.handle).and_then(|h| d.entity(h)).is_some_and(|e| xdata::read(&e.common).is_some());
        instances.push(CheckInstance {
            xdata_valid: !unreadable_hex.contains_key(&base.handle),
            legacy: base.instance_id.is_none() && !has_record,
            base,
        });
    }
    for (h, why) in &unreadable {
        if boxes.contains(&h.hex()) {
            continue;
        }
        let at = d.entity(*h).map_or([0.0, 0.0], |e| spot(d, e));
        issues.push(issue(IssueCode::XdataInvalid, h.hex(), at, format!("the U-BOT data cannot be used: {why}")));
    }

    let mut layouts = Vec::new();
    for e in d.model.iter() {
        let Some(Ok(UbotTag::Layout { record, .. })) = xdata::read(&e.common) else { continue };
        // A layout tag on anything but a closed polyline is among the unreadable tags above.
        let Some(ring) = ring_of(e) else { continue };
        let at = first_vertex(&ring);
        let id = shown(&record.layout_id);
        if ring.vertices.len() > MAX_BOUNDARY_VERTICES {
            let n = ring.vertices.len();
            issues.push(issue(
                IssueCode::XdataInvalid,
                e.handle.hex(),
                at,
                format!("the slab boundary of layout {id} has {n} vertices, the limit is {MAX_BOUNDARY_VERTICES}"),
            ));
            continue;
        }
        if let Err(why) = ring_from_input(&ring, unit_scale, RING_TOL_MM) {
            issues.push(issue(IssueCode::XdataInvalid, e.handle.hex(), at, format!("the slab boundary of layout {id} cannot be checked: {why}")));
            continue;
        }
        let stored_geometry_hash = Some(record.geometry_hash.clone());
        let layout_id = record.layout_id.clone();
        let layout = if grid_usable(&record) {
            Some(record)
        } else {
            issues.push(issue(
                IssueCode::XdataInvalid,
                e.handle.hex(),
                at,
                format!("layout {id} stores a grid that cannot be used, so moved boxes are not checked"),
            ));
            None
        };
        layouts.push(LayoutInput {
            layout_id,
            boundary_handle: e.handle.hex(),
            boundary: ring,
            holes: Vec::new(),
            stored_geometry_hash,
            layout,
            edge_clearance_mm: None,
        });
    }

    // Every zone goes to the core unfiltered: it keeps those near a slab or a box (ruling R17).
    let mut zone_inputs = Vec::new();
    for (h, z) in zones(d) {
        let at = first_vertex(&z.ring);
        if let Some((code, message)) = alt_product_problem(&z, catalog) {
            issues.push(issue(code, h.hex(), at, message));
        }
        match zone_problem(&z, unit_scale) {
            Some(why) => issues.push(issue(IssueCode::XdataInvalid, h.hex(), at, format!("zone {} cannot be checked: {why}", shown(&z.id)))),
            None => zone_inputs.push(z),
        }
    }

    Gathered { request: CheckRequest { unit_scale, instances, layouts, zones: zone_inputs }, issues }
}

/// Why the core could not place this box, if it could not.
fn unplaceable(b: &ExistingInstance, catalog: &Catalog, unit_scale: f64) -> Option<&'static str> {
    let scale_y = b.scale_y.unwrap_or(b.scale);
    if !(b.insert.iter().all(|c| c.is_finite()) && b.rotation_deg.is_finite()) {
        return Some("the insert point and rotation must be finite numbers");
    }
    if !(b.scale.is_finite() && b.scale != 0.0 && scale_y.is_finite() && scale_y != 0.0) {
        return Some("the scale must be finite and non-zero");
    }
    let mapping = catalog.mapping_for_block(&b.block_name);
    let rel = [relative_scale(mapping, b.scale, unit_scale), relative_scale(mapping, scale_y, unit_scale)];
    if !rel.iter().all(|r| r.abs() <= MAX_RELATIVE_SCALE) {
        return Some("the scale is out of range");
    }
    let product = match b.product_id.as_deref() {
        Some(id) => catalog.product(id),
        None => mapping.and_then(|m| m.product_id.as_deref()).and_then(|id| catalog.product(id)),
    };
    let insert_mm = Vec2::new(b.insert[0], b.insert[1]) * unit_scale;
    let c = center_from_insert_xy(mapping, product.map_or(0.0, |p| p.length_mm), insert_mm, b.rotation_deg, rel);
    if !(c.x.abs() <= MAX_COORD_MM && c.y.abs() <= MAX_COORD_MM) {
        return Some("the box is too far from the origin");
    }
    None
}

/// The stored grid can place cells: finite values, positive unit scale and pitches in range.
fn grid_usable(r: &LayoutRecord) -> bool {
    let finite = [r.origin[0], r.origin[1], r.angle_deg, r.pitch_x_mm, r.pitch_y_mm, r.unit_scale].iter().all(|v| v.is_finite());
    let pitch_ok = [r.pitch_x_mm, r.pitch_y_mm].iter().all(|p| *p > 0.0 && *p <= MAX_PITCH_MM);
    finite
        && pitch_ok
        && r.unit_scale > 0.0
        && (r.origin[0] * r.unit_scale).abs() <= MAX_COORD_MM
        && (r.origin[1] * r.unit_scale).abs() <= MAX_COORD_MM
}

/// What would make the core refuse this zone.
fn zone_problem(z: &ZoneInput, unit_scale: f64) -> Option<String> {
    if let Some(c) = z.clearance_mm
        && !(c.is_finite() && (0.0..=MAX_CLEARANCE_MM).contains(&c))
    {
        return Some(format!("clearance {c} mm is not between 0 and {MAX_CLEARANCE_MM} mm"));
    }
    ring_from_input(&z.ring, unit_scale, RING_TOL_MM).err().map(|e| e.to_string())
}

/// An alternative product only on, and always on, an alt_product zone, and one the catalog knows (carried from T6).
fn alt_product_problem(z: &ZoneInput, catalog: &Catalog) -> Option<(IssueCode, String)> {
    match (z.zone_type == ZoneType::AltProduct, z.alt_product_id.as_deref()) {
        (true, None) => Some((IssueCode::XdataInvalid, "an alt_product zone needs an alternative product".into())),
        (false, Some(id)) => Some((
            IssueCode::XdataInvalid,
            format!("a {} zone carries alternative product {}; only alt_product zones name one", zone_type_name(z.zone_type), shown(id)),
        )),
        (true, Some(id)) if catalog.product(id).is_none() => {
            Some((IssueCode::UnknownProduct, format!("alternative product {} is not in the catalog", shown(id))))
        }
        _ => None,
    }
}
