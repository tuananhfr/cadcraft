//! UBFILL / UBOT: lays U-BOT boxes on a slab with a preview and one-step commit.

use std::sync::Arc;

use cadcraft_engine::Input;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::ubot::read::{existing_instances, ring_of, tags, zones};
use cadcraft_engine::cmd::ubot::xdata::{self, UbotTag};
use cadcraft_engine::doc::{Block, Common, Drawing, Entity, EntityKind, Handle, Insert, LwPolyline, RawTag, Space};
use cadcraft_engine::geom::{PolyVertex, Vec2, Vec3};
use cadcraft_ubot::{CheckInstance, CheckRequest, LayoutInput, LayoutRecord};
use serde_json::{Value, json};

fn poly(points: &[[f64; 2]]) -> EntityKind {
    let vertices = points.iter().map(|p| PolyVertex { p: Vec2::new(p[0], p[1]), bulge: 0.0, start_width: 0.0, end_width: 0.0 }).collect();
    EntityKind::LwPolyline(LwPolyline { vertices, closed: true, const_width: 0.0, elevation: 0.0, plinegen: false })
}

fn square(x: f64, y: f64, size: f64) -> EntityKind {
    poly(&[[x, y], [x + size, y], [x + size, y + size], [x, y + size]])
}

fn insert(block: &str, at: [f64; 2], cols: u32, rows: u32, spacing: f64) -> EntityKind {
    EntityKind::Insert(Insert {
        block: block.into(),
        insert: Vec3::new(at[0], at[1], 0.0),
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs: Vec::new(),
        cols,
        rows,
        col_spacing: spacing,
        row_spacing: spacing,
    })
}

/// A drawing in `insunits` with a square slab boundary from (0, 0); returns the session and the boundary handle.
fn slab(insunits: i64, size: f64) -> (Session, Handle) {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", insunits);
    let b = d.add(&Space::Model, Common::default(), square(0.0, 0.0, size)).unwrap();
    let mut s = Session::empty();
    s.open_drawing(d, "slab.dxf", None);
    (s, b)
}

fn undo_len(s: &Session) -> usize {
    s.state().unwrap().undo.len()
}

fn doc(s: &Session) -> Arc<Drawing> {
    s.state().unwrap().doc.clone()
}

fn commit(s: &mut Session, b: Handle, extra: Value) -> Value {
    let mut p = json!({ "boundary": b.hex(), "product": "ubot-h13-7", "mode": "commit" });
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("ubfill", &p).unwrap()
}

fn preview(s: &mut Session, b: Handle, extra: Value) -> Value {
    let mut p = json!({ "boundary": b.hex(), "product": "ubot-h13-7" });
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("ubfill", &p).unwrap()
}

/// Box INSERTs in model space with their instance tags.
fn boxes(d: &Drawing) -> Vec<(Handle, Insert, Common)> {
    d.model
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::Insert(i) => Some((e.handle, i.clone(), e.common.clone())),
            _ => None,
        })
        .collect()
}

fn instance_of(c: &Common) -> (String, String, String, i16) {
    match xdata::read(c) {
        Some(Ok(UbotTag::Instance { instance_id, product_id, layout_id, count_multiplier })) => {
            (instance_id, product_id, layout_id, count_multiplier)
        }
        other => panic!("not an instance tag: {other:?}"),
    }
}

fn layout_of(d: &Drawing, b: Handle) -> (LayoutRecord, String) {
    match xdata::read(&d.entity(b).unwrap().common) {
        Some(Ok(UbotTag::Layout { record, boundary_handle })) => (record, boundary_handle),
        other => panic!("not a layout tag: {other:?}"),
    }
}

fn problem_cells(r: &Value, reason: &str) -> Vec<[i64; 2]> {
    r["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["reasons"].as_array().unwrap().iter().any(|x| x == reason))
        .map(|p| [p["cell"][0].as_i64().unwrap(), p["cell"][1].as_i64().unwrap()])
        .collect()
}

fn is_lower_hex16(id: &str) -> bool {
    id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn copied_floor_gets_new_layout_and_leaves_original() {
    let (mut s, b) = slab(4, 6600.0);
    let first = commit(&mut s, b, json!({}));
    let old_id = first["layoutId"].as_str().unwrap().to_string();
    assert_eq!(first["summary"]["added"], 100);
    let originals: Vec<(Handle, Insert, Common)> = boxes(s.doc().unwrap());
    let mut handles: Vec<String> = vec![b.hex()];
    handles.extend(originals.iter().map(|(h, _, _)| h.hex()));
    let copied = s.execute("copy", &json!({ "handles": handles, "delta": [10000.0, 0.0] })).unwrap();
    let copy_b = Handle::parse_hex(copied["handles"][0].as_str().unwrap()).unwrap();
    assert_ne!(copy_b, b);
    assert!(ring_of(s.doc().unwrap().entity(copy_b).unwrap()).is_some(), "the first copy is the boundary");
    let (_, stored) = layout_of(s.doc().unwrap(), copy_b);
    assert_eq!(stored, b.hex(), "the copy still carries the original's boundary handle");

    let r = commit(&mut s, copy_b, json!({}));
    let new_id = r["layoutId"].as_str().unwrap().to_string();
    assert!(is_lower_hex16(&new_id), "{new_id}");
    assert_ne!(new_id, old_id);
    assert_eq!(r["previousLayoutId"], old_id.as_str());
    assert_eq!(r["summary"]["updated"], 100, "{}", r["summary"]);
    assert_eq!(r["summary"]["added"], 0);
    assert_eq!(r["summary"]["removed"], 0);
    let d = s.doc().unwrap();
    let (record, owner) = layout_of(d, copy_b);
    assert_eq!((record.layout_id.as_str(), record.revision, owner.as_str()), (new_id.as_str(), 1, copy_b.hex().as_str()));
    let (record, owner) = layout_of(d, b);
    assert_eq!((record.layout_id.as_str(), record.revision, owner.as_str()), (old_id.as_str(), 1, b.hex().as_str()), "original untouched");
    for (h, ins, common) in boxes(d) {
        let (instance_id, _, layout_id, _) = instance_of(&common);
        if ins.insert.x > 10000.0 {
            assert_eq!(layout_id, new_id);
            assert!(instance_id.starts_with(&format!("{new_id}:")), "{instance_id}");
        } else {
            let (_, ins0, common0) = originals.iter().find(|(h0, _, _)| *h0 == h).unwrap();
            assert_eq!((&ins, &common), (ins0, common0), "original box {} changed", h.hex());
        }
    }

    let issues = core_check(&s, 2, 200);
    assert!(issues.is_empty(), "{issues:?}");
}

/// UBCHECK (Task 8) is not there yet: runs the core check on what the drawing holds (mm drawing, holes as tagged).
fn core_check(s: &Session, layout_count: usize, box_count: usize) -> Vec<cadcraft_ubot::Issue> {
    let d = s.doc().unwrap();
    let catalog = &s.ubot.catalog;
    let unit_scale = 1.0;
    let all_zones: Vec<_> = zones(d).into_iter().map(|(_, z)| z).collect();
    let layouts: Vec<LayoutInput> = tags(d)
        .into_iter()
        .filter_map(|(h, t)| match t {
            Ok(UbotTag::Layout { record, boundary_handle }) => Some(LayoutInput {
                layout_id: record.layout_id.clone(),
                boundary_handle,
                boundary: ring_of(d.entity(h).unwrap()).unwrap(),
                holes: Vec::new(),
                stored_geometry_hash: Some(record.geometry_hash.clone()),
                layout: Some(record),
                edge_clearance_mm: None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(layouts.len(), layout_count);
    let instances: Vec<CheckInstance> = existing_instances(d, catalog)
        .into_iter()
        .map(|base| CheckInstance { legacy: base.instance_id.is_none(), base, xdata_valid: true })
        .collect();
    assert_eq!(instances.len(), box_count);
    cadcraft_ubot::check(&CheckRequest { unit_scale, instances, layouts, zones: all_zones }, catalog).unwrap()
}

#[test]
fn check_after_a_fill_with_a_hole_sees_no_stale_layout() {
    let (mut s, b) = slab(4, 6600.0);
    let hole = s.doc_mut().unwrap().add(&Space::Model, Common::default(), square(2900.0, 2900.0, 800.0)).unwrap();
    commit(&mut s, b, json!({ "holes": [hole.hex()] }));
    let issues = core_check(&s, 1, 96);
    assert!(issues.iter().all(|i| i.code != cadcraft_ubot::IssueCode::StaleLayout), "{issues:?}");
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn stored_grid_with_a_bad_unit_scale_is_not_reused() {
    for bad in [0.0, -1.0] {
        let (mut s, b) = slab(4, 6600.0);
        commit(&mut s, b, json!({ "angle": 15, "origin": [400.0, 6200.0], "pitchX": 700 }));
        let (mut record, owner) = layout_of(s.doc().unwrap(), b);
        record.unit_scale = bad;
        s.doc_mut().unwrap().modify_entity(b, |e| xdata::write(&mut e.common, &UbotTag::Layout { record, boundary_handle: owner }).unwrap()).unwrap();
        let r = preview(&mut s, b, json!({}));
        assert_eq!(r["layout"]["origin"], json!([330.0, 6270.0]), "unit scale {bad}");
        assert_eq!((r["layout"]["angleDeg"].as_f64(), r["layout"]["pitchXMm"].as_f64()), (Some(0.0), Some(660.0)));
        assert_eq!(r["reset"], true);
    }
}

#[test]
fn preview_does_not_write() {
    let (mut s, b) = slab(4, 6600.0);
    let before = doc(&s);
    let undo = undo_len(&s);
    let r = preview(&mut s, b, json!({}));
    assert_eq!(r["mode"], "preview");
    assert_eq!(r["summary"]["added"], 100);
    assert_eq!(r["add"].as_array().unwrap().len(), 100);
    assert!(r.get("written").is_none());
    assert!(Arc::ptr_eq(&before, &doc(&s)), "preview must not touch the drawing");
    assert_eq!(undo_len(&s), undo);
    assert!(xdata::read(&s.doc().unwrap().entity(b).unwrap().common).is_none(), "no layout tag yet");
}

#[test]
fn commit_writes_100_boxes_one_undo() {
    let (mut s, b) = slab(4, 6600.0);
    let before = doc(&s);
    let undo = undo_len(&s);
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["mode"], "commit");
    assert_eq!(r["written"], json!({ "added": 100, "updated": 0, "removed": 0 }));
    assert_eq!(boxes(s.doc().unwrap()).len(), 100);
    assert_eq!(undo_len(&s), undo + 1, "one undo step");
    s.execute("u", &json!({})).unwrap();
    assert_eq!(*doc(&s), *before, "undo restores the drawing exactly");
}

#[test]
fn rerun_commit_is_noop() {
    let (mut s, b) = slab(4, 6600.0);
    commit(&mut s, b, json!({}));
    let before = doc(&s);
    let undo = undo_len(&s);
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["summary"]["added"], 0);
    assert_eq!(r["summary"]["updated"], 0);
    assert_eq!(r["summary"]["removed"], 0);
    assert_eq!(r["summary"]["kept"], 100);
    assert!(r.get("written").is_none(), "{r}");
    assert!(Arc::ptr_eq(&before, &doc(&s)), "nothing written");
    assert_eq!(undo_len(&s), undo, "no undo step");
}

#[test]
fn expect_hash_conflict() {
    let (mut s, b) = slab(4, 6600.0);
    let p = preview(&mut s, b, json!({}));
    let hash = p["inputHash"].as_str().unwrap().to_string();
    let before = doc(&s);
    let p = json!({ "boundary": b.hex(), "product": "ubot-h13-7", "mode": "commit", "expectHash": "0".repeat(64) });
    let err = s.execute("ubfill", &p).unwrap_err().to_string();
    assert!(err.contains("CONFLICT: inputs changed since preview"), "{err}");
    assert!(Arc::ptr_eq(&before, &doc(&s)));
    // A different rib is a different input.
    let p = json!({ "boundary": b.hex(), "product": "ubot-h13-7", "rib": 120, "mode": "commit", "expectHash": hash });
    assert!(s.execute("ubfill", &p).unwrap_err().to_string().contains("CONFLICT"));
    let p = json!({ "boundary": b.hex(), "product": "ubot-h13-7", "mode": "commit", "expectHash": hash });
    let r = s.execute("ubfill", &p).unwrap();
    assert_eq!(r["inputHash"], hash.as_str());
    assert_eq!(r["written"]["added"], 100);
}

/// A clock that moves on every read, so a fresh layout id differs between any two generations.
fn ticking_clock() -> u64 {
    static NOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_000);
    NOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[test]
fn fresh_layout_id_is_kept_from_preview_to_commit() {
    cadcraft_engine::cmd::ubplot::set_clock(ticking_clock);
    let (mut s, b) = slab(4, 6600.0);
    let p = preview(&mut s, b, json!({}));
    let again = preview(&mut s, b, json!({}));
    assert_eq!(p["layoutId"], again["layoutId"], "same boundary and geometry, same id");
    let p2 = json!({ "boundary": b.hex(), "product": "ubot-h13-7", "mode": "commit", "expectHash": p["inputHash"] });
    let r = s.execute("ubfill", &p2).unwrap();
    assert_eq!(r["layoutId"], p["layoutId"]);
    assert_eq!(r["add"][0]["instanceId"], p["add"][0]["instanceId"]);
    // After undo the boundary is untagged again and the next fill makes a new id.
    s.execute("u", &json!({})).unwrap();
    let after = preview(&mut s, b, json!({}));
    assert_ne!(after["layoutId"], p["layoutId"]);
}

#[test]
fn boxes_carry_instance_xdata_and_layer_ubot() {
    let (mut s, b) = slab(4, 6600.0);
    let r = commit(&mut s, b, json!({}));
    let layout_id = r["layoutId"].as_str().unwrap();
    let d = s.doc().unwrap();
    let layer = d.layer("UBOT").expect("layer UBOT");
    assert_eq!(layer.color, cadcraft_color::Color::from_aci(55));
    assert!(d.block("UBOT_H13_7").is_some());
    let placed = boxes(d);
    assert_eq!(placed.len(), 100);
    let mut ids = std::collections::HashSet::new();
    for (_, ins, common) in &placed {
        assert_eq!(common.layer, "UBOT");
        assert_eq!(ins.block, "UBOT_H13_7");
        assert_eq!(ins.scale, Vec3::new(1.0, 1.0, 1.0));
        let (instance_id, product_id, lid, multiplier) = instance_of(common);
        assert_eq!((product_id.as_str(), lid.as_str(), multiplier), ("ubot-h13-7", layout_id, 1));
        assert!(instance_id.starts_with(&format!("{layout_id}:")));
        assert!(ids.insert(instance_id));
    }
    // Rows top-down, then left to right: the first box is cell (0, 0) at the top left.
    let (_, first, common) = &placed[0];
    assert_eq!(instance_of(common).0, format!("{layout_id}:0:0"));
    assert_eq!((first.insert.x, first.insert.y), (330.0, 6270.0));
    assert_eq!(instance_of(&placed[1].2).0, format!("{layout_id}:1:0"));

    // A centimetre drawing: the mm block is inserted at 1/10.
    let (mut s, b) = slab(5, 660.0);
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["written"]["added"], 100);
    let d = s.doc().unwrap();
    let block = d.block("UBOT_H13_7").unwrap();
    let EntityKind::LwPolyline(outline) = &block.entities.iter().next().unwrap().kind else { panic!("outline") };
    assert_eq!(outline.vertices[1].p, Vec2::new(260.0, -260.0), "block drawn in mm");
    for (_, ins, _) in boxes(d) {
        assert!((ins.scale.x - 0.1).abs() < 1e-12 && ins.scale.x == ins.scale.y && ins.scale.y == ins.scale.z, "{:?}", ins.scale);
    }
}

#[test]
fn uses_existing_ub1_block_with_top_center_anchor() {
    let (mut s, b) = slab(4, 6600.0);
    {
        let d = s.doc_mut().unwrap();
        let mut block = Block::new("UB-1");
        let handle = d.new_handle();
        block.entities.push(Entity { handle, common: Common::default(), kind: square(-260.0, -520.0, 520.0) });
        d.blocks.insert("UB-1".into(), Arc::new(block));
    }
    let overlay = json!({
        "schema": "ubot.catalog/1", "catalogId": "lpc-project", "version": "1", "products": [], "rulepacks": [],
        "mappings": [{ "productId": "ubot-h13-7", "cadEngine": "any", "blockName": "UB-1", "unitsMmPerBlockUnit": 1.0, "anchor": "top_center", "rotationOffsetDeg": 0.0 }]
    });
    s.execute("ublib", &json!({ "action": "load", "text": overlay.to_string() })).unwrap();
    let r = commit(&mut s, b, json!({}));
    let first = &r["add"][0];
    assert_eq!(first["blockName"], "UB-1");
    assert_eq!(first["center"], json!([330.0, 6270.0]));
    assert_eq!(first["insert"], json!([330.0, 6530.0]), "top centre of the box");
    let d = s.doc().unwrap();
    assert!(d.block("UBOT_H13_7").is_none(), "the mapped block is used, no new block");
    let placed = boxes(d);
    assert_eq!(placed.len(), 100);
    assert!(placed.iter().all(|(_, i, _)| i.block == "UB-1" && i.scale == Vec3::new(1.0, 1.0, 1.0)));
    assert_eq!((placed[0].1.insert.x, placed[0].1.insert.y), (330.0, 6530.0));
    // The re-run reads the anchor back and changes nothing.
    let before = doc(&s);
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["summary"]["kept"], 100);
    assert!(Arc::ptr_eq(&before, &doc(&s)));

    // A mapping whose block the drawing lacks falls back to the generated block.
    let (mut s, b) = slab(4, 6600.0);
    let overlay = overlay.to_string().replace("UB-1", "UB-9");
    s.execute("ublib", &json!({ "action": "load", "text": overlay })).unwrap();
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["add"][0]["blockName"], "UBOT_H13_7");
    assert_eq!(r["add"][0]["insert"], json!([330.0, 6270.0]));
}

#[test]
fn boundary_gets_layout_tag_revision_increments() {
    let (mut s, b) = slab(4, 6600.0);
    let r = commit(&mut s, b, json!({}));
    let layout_id = r["layoutId"].as_str().unwrap().to_string();
    assert!(is_lower_hex16(&layout_id), "{layout_id}");
    let (record, owner) = layout_of(s.doc().unwrap(), b);
    assert_eq!(owner, b.hex());
    assert_eq!(record.layout_id, layout_id);
    assert_eq!(record.revision, 1);
    assert_eq!(record.product_id, "ubot-h13-7");
    assert_eq!(record.rulepack, "lpc-draft-1@1");
    assert_eq!((record.pitch_x_mm, record.pitch_y_mm, record.unit_scale, record.angle_deg), (660.0, 660.0, 1.0, 0.0));
    assert_eq!(record.origin, [330.0, 6270.0]);
    assert_eq!(record.geometry_hash.len(), 64);
    assert_eq!(r["layout"]["revision"], 1);

    // Same grid, other product: the boxes are updated in place and the revision goes up.
    let r = commit(&mut s, b, json!({ "product": "ubot-h10-7" }));
    assert_eq!(r["layoutId"], layout_id.as_str());
    assert_eq!(r["written"], json!({ "added": 0, "updated": 100, "removed": 0 }));
    let (record, _) = layout_of(s.doc().unwrap(), b);
    assert_eq!((record.revision, record.product_id.as_str()), (2, "ubot-h10-7"));
    assert!(boxes(s.doc().unwrap()).iter().all(|(_, i, c)| i.block == "UBOT_H10_7" && instance_of(c).1 == "ubot-h10-7"));
    // A no-op re-run keeps the revision.
    commit(&mut s, b, json!({ "product": "ubot-h10-7" }));
    assert_eq!(layout_of(s.doc().unwrap(), b).0.revision, 2);
}

#[test]
fn zone_inside_bbox_is_picked_up_by_default() {
    let (mut s, b) = slab(4, 6600.0);
    // Covers cell (0, 0) only: its footprint spans x 70..590, y 6010..6530.
    let z = s.doc_mut().unwrap().add(&Space::Model, Common::default(), square(200.0, 6200.0, 100.0)).unwrap();
    s.execute("ubzone", &json!({ "handles": [z.hex()], "type": "column" })).unwrap();
    let r = preview(&mut s, b, json!({}));
    assert_eq!(r["summary"]["added"], 99, "{}", r["summary"]);
    assert_eq!(problem_cells(&r, "IN_ZONE"), vec![[0, 0]]);
    // `zones` is not a filter: passing an empty list still uses the tagged zone, with a warning.
    let r = preview(&mut s, b, json!({ "zones": [] }));
    assert_eq!(r["summary"]["added"], 99);
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with("ZONES_PARAM_IGNORED")), "{}", r["warnings"]);
    // Selecting the tagged zone as a hole changes nothing; an untagged closed polyline is a hole.
    let hole = s.doc_mut().unwrap().add(&Space::Model, Common::default(), square(3000.0, 3000.0, 700.0)).unwrap();
    let with_zone = preview(&mut s, b, json!({ "holes": [z.hex()] }));
    assert_eq!(with_zone["summary"]["added"], 99);
    let with_hole = preview(&mut s, b, json!({ "holes": [hole.hex()] }));
    // The hole goes to the core as an opening zone (ruling R15): inside reads IN_HOLE, as a plain hole did.
    assert_eq!(with_hole["summary"]["added"], 95);
    assert_eq!(problem_cells(&with_hole, "IN_HOLE"), vec![[4, 4], [5, 4], [4, 5], [5, 5]], "{}", with_hole["problems"]);
    assert_eq!(problem_cells(&with_hole, "IN_ZONE"), vec![[0, 0]]);
}

#[test]
fn selected_hole_becomes_an_opening_zone_on_commit() {
    let (mut s, b) = slab(4, 6600.0);
    // Covers cells (4, 4), (5, 4), (4, 5) and (5, 5) near the slab centre.
    let hole = s.doc_mut().unwrap().add(&Space::Model, Common::default(), square(2900.0, 2900.0, 800.0)).unwrap();
    let p = preview(&mut s, b, json!({ "holes": [hole.hex()] }));
    assert_eq!(problem_cells(&p, "IN_HOLE"), vec![[4, 4], [5, 4], [4, 5], [5, 5]], "{}", p["problems"]);
    assert!(xdata::read(&s.doc().unwrap().entity(hole).unwrap().common).is_none(), "preview tags nothing");
    let r = commit(&mut s, b, json!({ "holes": [hole.hex()], "expectHash": p["inputHash"] }));
    assert_eq!(r["written"]["added"], 96);
    let ring = ring_of(s.doc().unwrap().entity(hole).unwrap()).unwrap();
    let expected = UbotTag::Zone {
        zone_id: cadcraft_engine::cmd::ubot::zone_id(hole, &ring),
        zone_type: cadcraft_ubot::ZoneType::Opening,
        clearance_mm: Some(0.0),
        alt_product_id: None,
    };
    assert_eq!(xdata::read(&s.doc().unwrap().entity(hole).unwrap().common), Some(Ok(expected)), "the hole is kept as an opening zone");
    assert_eq!(zones(s.doc().unwrap()).len(), 1);
    // The opening reads back as a zone: the same geometry, so nothing to do without selecting it again.
    let before = doc(&s);
    let undo = undo_len(&s);
    let r = commit(&mut s, b, json!({}));
    assert!(r.get("written").is_none(), "{}", r["summary"]);
    let r = commit(&mut s, b, json!({ "holes": [hole.hex()] }));
    assert!(r.get("written").is_none(), "a tagged zone selected again is a no-op");
    assert!(Arc::ptr_eq(&before, &doc(&s)));
    assert_eq!(undo_len(&s), undo);

    // A hole that carries other U-BOT data is refused.
    let (mut s, b) = slab(4, 6600.0);
    let mut tagged = Common::default();
    let tag = UbotTag::Instance {
        instance_id: "aaaaaaaaaaaaaaaa:0:0".into(),
        product_id: "ubot-h13-7".into(),
        layout_id: "aaaaaaaaaaaaaaaa".into(),
        count_multiplier: 1,
    };
    xdata::write(&mut tagged, &tag).unwrap();
    let odd = s.doc_mut().unwrap().add(&Space::Model, tagged, square(2900.0, 2900.0, 800.0)).unwrap();
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7", "holes": [odd.hex()] })).unwrap_err().to_string();
    assert!(err.contains(&format!("hole {}", odd.hex())), "{err}");
    // S1: a NaN hole is named.
    let nan = s.doc_mut().unwrap().add(&Space::Model, Common::default(), poly(&[[10.0, 10.0], [f64::NAN, 10.0], [20.0, 20.0]])).unwrap();
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7", "holes": [nan.hex()] })).unwrap_err().to_string();
    assert!(err.contains(&format!("hole {}", nan.hex())), "{err}");
    // A hole on a locked layer is refused before anything is written.
    let (mut s, b) = slab(4, 6600.0);
    let d = s.doc_mut().unwrap();
    d.ensure_layer("LOCKED");
    d.layer_mut("LOCKED").unwrap().locked = true;
    let locked = d.add(&Space::Model, Common { layer: "LOCKED".into(), ..Common::default() }, square(2900.0, 2900.0, 800.0)).unwrap();
    let before = doc(&s);
    let err = s
        .execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7", "holes": [locked.hex()], "mode": "commit" }))
        .unwrap_err()
        .to_string();
    assert!(err.contains(&locked.hex()) && err.contains("locked"), "{err}");
    assert!(Arc::ptr_eq(&before, &doc(&s)));
}

#[test]
fn hand_inserted_generated_block_blocks_its_cell() {
    let (mut s, b) = slab(4, 6600.0);
    s.doc_mut().unwrap().add(&Space::Model, Common::default(), insert("UBOT_H13_7", [990.0, 6270.0], 1, 1, 0.0)).unwrap();
    let r = preview(&mut s, b, json!({}));
    assert_eq!(problem_cells(&r, "OVERLAPS_EXISTING"), vec![[1, 0]], "{}", r["problems"]);
    assert_eq!(r["summary"]["added"], 99);
}

fn tag_zone(s: &mut Session, h: Handle, id: &str, zone_type: cadcraft_ubot::ZoneType) {
    let tag = UbotTag::Zone { zone_id: id.into(), zone_type, clearance_mm: None, alt_product_id: None };
    s.doc_mut().unwrap().modify_entity(h, |e| xdata::write(&mut e.common, &tag).unwrap()).unwrap();
}

#[test]
fn broken_zone_shape_in_reach_is_an_error_far_away_it_is_not() {
    let (mut s, b) = slab(4, 6600.0);
    let mut open = square(1000.0, 1000.0, 300.0);
    if let EntityKind::LwPolyline(pl) = &mut open {
        pl.closed = false;
    }
    let z = s.doc_mut().unwrap().add(&Space::Model, Common::default(), open).unwrap();
    tag_zone(&mut s, z, "00112233445566cc", cadcraft_ubot::ZoneType::Column);
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap_err().to_string();
    assert!(err.contains(&format!("zone {}", z.hex())), "{err}");
    // On another floor, far beyond any clearance, it is not this slab's business.
    s.doc_mut()
        .unwrap()
        .modify_entity(z, |e| {
            if let EntityKind::LwPolyline(pl) = &mut e.kind {
                for v in &mut pl.vertices {
                    v.p.x += 100_000.0;
                }
            }
        })
        .unwrap();
    assert_eq!(preview(&mut s, b, json!({}))["summary"]["added"], 100);
}

#[test]
fn zones_on_other_floors_do_not_block_but_nan_zones_do() {
    let (mut s, b) = slab(4, 6600.0);
    let clean = preview(&mut s, b, json!({}));
    let far = 100_000.0;
    let bow = poly(&[[far, 0.0], [far + 1000.0, 1000.0], [far + 1000.0, 0.0], [far, 1000.0]]);
    let bow = s.doc_mut().unwrap().add(&Space::Model, Common::default(), bow).unwrap();
    let dense: Vec<[f64; 2]> =
        (0..20_000).map(|k| f64::from(k) * std::f64::consts::TAU / 20_000.0).map(|a| [far + 5000.0 + 1000.0 * a.cos(), 1000.0 * a.sin()]).collect();
    let many = s.doc_mut().unwrap().add(&Space::Model, Common::default(), poly(&dense)).unwrap();
    tag_zone(&mut s, bow, "00000000000000b1", cadcraft_ubot::ZoneType::Exclusion);
    tag_zone(&mut s, many, "00000000000000b2", cadcraft_ubot::ZoneType::Exclusion);
    let r = preview(&mut s, b, json!({}));
    assert_eq!(r["summary"]["added"], 100);
    assert_eq!(r["layout"]["geometryHash"], clean["layout"]["geometryHash"]);
    let nan = s.doc_mut().unwrap().add(&Space::Model, Common::default(), poly(&[[1e9, 0.0], [f64::NAN, 0.0], [1e9, 10.0]])).unwrap();
    tag_zone(&mut s, nan, "00000000000000b3", cadcraft_ubot::ZoneType::Exclusion);
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap_err().to_string();
    assert!(err.contains(&format!("zone {}", nan.hex())), "{err}");
}

#[test]
fn fresh_ids_of_two_boundaries_do_not_evict_each_other() {
    cadcraft_engine::cmd::ubplot::set_clock(ticking_clock);
    let (mut s, a) = slab(4, 6600.0);
    let b = s.doc_mut().unwrap().add(&Space::Model, Common::default(), square(20_000.0, 0.0, 6600.0)).unwrap();
    let pa = preview(&mut s, a, json!({}));
    let pb = preview(&mut s, b, json!({}));
    assert_ne!(pa["layoutId"], pb["layoutId"]);
    let ra = commit(&mut s, a, json!({ "expectHash": pa["inputHash"] }));
    assert_eq!(ra["layoutId"], pa["layoutId"]);
    let rb = commit(&mut s, b, json!({ "expectHash": pb["inputHash"] }));
    assert_eq!(rb["layoutId"], pb["layoutId"], "committing A keeps B's remembered id");
}

#[test]
fn failed_commit_writes_nothing() {
    let (mut s, b) = slab(4, 6600.0);
    commit(&mut s, b, json!({}));
    s.doc_mut().unwrap().layer_mut("UBOT").unwrap().locked = true;
    let before = doc(&s);
    let undo = undo_len(&s);
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h10-7", "mode": "commit" })).unwrap_err().to_string();
    assert!(err.contains("locked layer"), "{err}");
    assert!(Arc::ptr_eq(&before, &doc(&s)));
    assert_eq!(undo_len(&s), undo);
}

fn ub_block(s: &mut Session, name: &str, kind: EntityKind, mm_per_unit: f64, anchor: &str) {
    let d = s.doc_mut().unwrap();
    let mut block = Block::new(name);
    let handle = d.new_handle();
    block.entities.push(Entity { handle, common: Common::default(), kind });
    d.blocks.insert(name.into(), Arc::new(block));
    let overlay = json!({
        "schema": "ubot.catalog/1", "catalogId": "lpc-project", "version": "1", "products": [], "rulepacks": [],
        "mappings": [{ "productId": "ubot-h13-7", "cadEngine": "any", "blockName": name, "unitsMmPerBlockUnit": mm_per_unit, "anchor": anchor, "rotationOffsetDeg": 0.0 }]
    });
    s.execute("ublib", &json!({ "action": "load", "text": overlay.to_string() })).unwrap();
}

#[test]
fn top_center_anchor_follows_the_grid_angle() {
    let (mut s, b) = slab(4, 6600.0);
    ub_block(&mut s, "UB-1", square(-260.0, -520.0, 520.0), 1.0, "top_center");
    let r = commit(&mut s, b, json!({ "angle": 30 }));
    let first = &r["add"][0];
    let (cx, cy) = (first["center"][0].as_f64().unwrap(), first["center"][1].as_f64().unwrap());
    let a = 30f64.to_radians();
    let expected = [cx - 260.0 * a.sin(), cy + 260.0 * a.cos()];
    let got = [first["insert"][0].as_f64().unwrap(), first["insert"][1].as_f64().unwrap()];
    assert!((got[0] - expected[0]).abs() < 1e-9 && (got[1] - expected[1]).abs() < 1e-9, "{got:?} vs {expected:?}");
    let placed = boxes(s.doc().unwrap());
    let ins = &placed.iter().find(|(_, _, c)| instance_of(c).0 == first["instanceId"].as_str().unwrap()).unwrap().1;
    assert_eq!([ins.insert.x, ins.insert.y], got);
    assert!((ins.rotation - a).abs() < 1e-12, "{}", ins.rotation);
    let before = doc(&s);
    commit(&mut s, b, json!({ "angle": 30 }));
    assert!(Arc::ptr_eq(&before, &doc(&s)), "rotated anchored boxes read back in place");
}

#[test]
fn mapped_block_units_set_the_insert_scale() {
    let (mut s, b) = slab(4, 6600.0);
    // Drawn in cm: 52 block units are the 520 mm box.
    ub_block(&mut s, "UB-2", square(-26.0, -26.0, 52.0), 10.0, "center");
    commit(&mut s, b, json!({}));
    let placed = boxes(s.doc().unwrap());
    assert_eq!(placed.len(), 100);
    assert!(placed.iter().all(|(_, i, _)| i.block == "UB-2" && i.scale == Vec3::new(10.0, 10.0, 10.0)), "{:?}", placed[0].1.scale);
    let before = doc(&s);
    commit(&mut s, b, json!({}));
    assert!(Arc::ptr_eq(&before, &doc(&s)));
}

fn interactive_enter_through(s: &mut Session, b: Handle) {
    s.start("ubfill").unwrap();
    s.input(Input::Pick(vec![b])).unwrap();
    for _ in 0..4 {
        s.input(Input::Enter).unwrap();
    }
    assert!(s.running.is_none(), "{:?}", s.current_prompt());
}

#[test]
fn interactive_rerun_is_a_noop() {
    let (mut s, b) = slab(4, 6600.0);
    interactive_enter_through(&mut s, b);
    assert_eq!(boxes(s.doc().unwrap()).len(), 100);
    let before = doc(&s);
    let undo = undo_len(&s);
    interactive_enter_through(&mut s, b);
    assert!(Arc::ptr_eq(&before, &doc(&s)));
    assert_eq!(undo_len(&s), undo);

    // A custom grid made through JSON survives an interactive re-run that takes every default.
    let (mut s, b) = slab(4, 6600.0);
    commit(&mut s, b, json!({ "angle": 15, "origin": [400.0, 6200.0], "pitchX": 700, "pitchY": 680, "product": "ubot-h20-7" }));
    let before = doc(&s);
    let undo = undo_len(&s);
    s.start("ubfill").unwrap();
    s.input(Input::Pick(vec![b])).unwrap();
    s.input(Input::Enter).unwrap();
    assert_eq!(s.current_prompt().unwrap().default.as_deref(), Some("ubot-h20-7"), "the stored product is the default");
    s.input(Input::Enter).unwrap();
    assert_eq!(s.current_prompt().unwrap().default.as_deref(), Some("180"), "rib default from the stored pitch");
    s.input(Input::Enter).unwrap();
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert!(Arc::ptr_eq(&before, &doc(&s)), "no-op");
    assert_eq!(undo_len(&s), undo);
    // JSON with origin, angle and pitch omitted keeps them too.
    let r = preview(&mut s, b, json!({ "product": "ubot-h20-7" }));
    assert_eq!((r["summary"]["kept"].as_u64(), r["reset"].as_bool()), (Some(boxes(s.doc().unwrap()).len() as u64), Some(false)), "{}", r["summary"]);
    // Another product or rib lets the core lay a new grid.
    let r = preview(&mut s, b, json!({ "product": "ubot-h20-7", "rib": 140 }));
    assert_eq!((r["layout"]["pitchXMm"].as_f64(), r["layout"]["angleDeg"].as_f64()), (Some(660.0), Some(15.0)));
    assert_eq!(r["reset"], true);
}

#[test]
fn copied_floor_uses_its_own_grid_origin() {
    let (mut s, b) = slab(4, 6600.0);
    commit(&mut s, b, json!({ "origin": [400.0, 6200.0] }));
    let handles: Vec<String> = std::iter::once(b.hex()).chain(boxes(s.doc().unwrap()).iter().map(|(h, _, _)| h.hex())).collect();
    let copied = s.execute("copy", &json!({ "handles": handles, "delta": [10000.0, 0.0] })).unwrap();
    let copy_b = Handle::parse_hex(copied["handles"][0].as_str().unwrap()).unwrap();
    let r = preview(&mut s, copy_b, json!({}));
    assert_eq!(r["layout"]["origin"], json!([10330.0, 6270.0]), "the default origin of the copy, not the original's stored one");
}

#[test]
fn interactive_refuses_a_zone_as_boundary_and_keeps_the_selection() {
    let (mut s, b) = slab(4, 6600.0);
    let z = s.doc_mut().unwrap().add(&Space::Model, Common::default(), square(200.0, 200.0, 100.0)).unwrap();
    s.execute("ubzone", &json!({ "handles": [z.hex()], "type": "column" })).unwrap();
    s.set_selection(vec![z]);
    s.start("ubfill").unwrap();
    s.input(Input::Pick(vec![z])).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Select slab boundary", "a zone is not a boundary");
    s.input(Input::Pick(vec![b])).unwrap();
    s.input(Input::Pick(vec![z])).unwrap();
    assert_eq!(s.selection(), vec![z]);
    s.input(Input::Cancel).unwrap();
    assert!(s.running.is_none());
    assert_eq!(s.selection(), vec![z], "Esc leaves the selection as it was");
}

#[test]
fn expect_hash_in_preview_is_reported() {
    let (mut s, b) = slab(4, 6600.0);
    let r = preview(&mut s, b, json!({ "expectHash": "abc" }));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w == "expectHash ignored in preview"), "{}", r["warnings"]);
}

#[test]
fn block_change_drops_attributes() {
    let (mut s, b) = slab(4, 6600.0);
    ub_block(&mut s, "UB-1", square(-260.0, -520.0, 520.0), 1.0, "top_center");
    commit(&mut s, b, json!({}));
    let attrib: cadcraft_engine::doc::Attrib =
        serde_json::from_value(json!({ "tag": "MARK", "text": { "insert": { "x": 0.0, "y": 0.0, "z": 0.0 }, "height": 100.0, "value": "A" } }))
            .unwrap();
    let first = boxes(s.doc().unwrap())[0].0;
    s.doc_mut()
        .unwrap()
        .modify_entity(first, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.attribs.push(attrib.clone());
            }
        })
        .unwrap();
    s.execute("ublib", &json!({ "action": "reset" })).unwrap();
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["written"]["updated"], 100, "{}", r["summary"]);
    let EntityKind::Insert(ins) = s.doc().unwrap().entity(first).unwrap().kind.clone() else { panic!("insert") };
    assert_eq!(ins.block, "UBOT_H13_7");
    assert!(ins.attribs.is_empty(), "attributes of the old block are dropped");
}

fn start_to_apply(s: &mut Session, b: Handle) {
    s.start("ubot").unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Select slab boundary");
    s.input(Input::Pick(vec![b])).unwrap();
    let p = s.current_prompt().unwrap();
    assert_eq!((p.message.as_str(), p.default.as_deref()), ("Select holes and zones", Some("none")));
    s.input(Input::Enter).unwrap();
    let p = s.current_prompt().unwrap();
    assert_eq!((p.message.as_str(), p.default.as_deref()), ("Box type", Some("ubot-h13-7")));
    assert!(p.keywords.contains(&"ubot-h10-7".to_string()));
    s.input(Input::Text("?".into())).unwrap();
    assert!(s.log.iter().any(|l| l.contains("ubot-h20-7")), "? lists the products");
    s.input(Input::Enter).unwrap();
    let p = s.current_prompt().unwrap();
    assert_eq!((p.message.as_str(), p.default.as_deref(), p.keywords.clone()), ("Rib", Some("140"), vec!["120".to_string(), "140".to_string()]));
    s.input(Input::Enter).unwrap();
    let p = s.current_prompt().unwrap();
    assert_eq!(p.display(), "Apply? [Yes/No/List] <Yes>:");
}

#[test]
fn interactive_cancel_at_apply_writes_nothing() {
    let (mut s, b) = slab(4, 6600.0);
    // One cell outside the slab so List has something to print.
    s.doc_mut().unwrap().modify_entity(b, |e| e.kind = poly(&[[0.0, 0.0], [6600.0, 0.0], [6600.0, 6600.0], [300.0, 6600.0], [0.0, 6300.0]])).unwrap();
    let before = doc(&s);
    let undo = undo_len(&s);
    start_to_apply(&mut s, b);
    assert!(s.log.iter().any(|l| l.starts_with("UBFILL: +99 boxes")), "summary printed: {:?}", s.log.iter().rev().take(5).collect::<Vec<_>>());
    assert_eq!(s.preview(Vec2::new(0.0, 0.0)).len(), 99, "footprints of the boxes to add");
    let marks = s.marks();
    assert_eq!(marks.len(), 1);
    assert!(marks[0].warn);
    assert_eq!(marks[0].label, "OUTSIDE_BOUNDARY");
    assert_eq!(marks[0].outline.len(), 4);
    s.input(Input::Text("List".into())).unwrap();
    assert!(s.log.iter().any(|l| l == "cell 0,0 at 330,6270: OUTSIDE_BOUNDARY"), "{:?}", s.log.iter().rev().take(3).collect::<Vec<_>>());
    assert_eq!(s.current_prompt().unwrap().message, "Apply?", "List asks again");
    s.input(Input::Cancel).unwrap();
    assert!(s.running.is_none());
    assert_eq!(*doc(&s), *before);
    assert_eq!(undo_len(&s), undo);

    // No also ends without writing.
    start_to_apply(&mut s, b);
    s.input(Input::Keyword("No".into())).unwrap();
    assert!(s.running.is_none());
    assert_eq!(*doc(&s), *before);
    assert_eq!(undo_len(&s), undo);
}

#[test]
fn interactive_yes_commits() {
    let (mut s, b) = slab(4, 6600.0);
    let undo = undo_len(&s);
    start_to_apply(&mut s, b);
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(boxes(s.doc().unwrap()).len(), 100);
    assert_eq!(layout_of(s.doc().unwrap(), b).0.revision, 1);

    // Rib 120 by keyword, product by id, Yes typed.
    let (mut s, b) = slab(4, 6600.0);
    s.start("ubfill").unwrap();
    s.input(Input::Pick(vec![b])).unwrap();
    s.input(Input::Enter).unwrap();
    s.input(Input::Keyword("ubot-h20-7".into())).unwrap();
    s.input(Input::Keyword("120".into())).unwrap();
    s.input(Input::Keyword("Yes".into())).unwrap();
    assert!(s.running.is_none());
    let (record, _) = layout_of(s.doc().unwrap(), b);
    assert_eq!((record.product_id.as_str(), record.pitch_x_mm), ("ubot-h20-7", 640.0));
}

#[test]
fn insunits_unknown_requires_units() {
    let (mut s, b) = slab(0, 6600.0);
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap_err().to_string();
    assert!(err.contains("units"), "{err}");
    let r = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7", "units": "mm" })).unwrap();
    assert_eq!(r["summary"]["added"], 100);
    // Read as metres the slab is 6.6 km wide: far over the cell limit.
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7", "units": "m" })).unwrap_err().to_string();
    assert!(err.contains("limit"), "{err}");
}

#[test]
fn insunits_unknown_interactive_asks_for_units() {
    let (mut s, b) = slab(0, 6600.0);
    s.start("ubfill").unwrap();
    s.input(Input::Pick(vec![b])).unwrap();
    s.input(Input::Enter).unwrap();
    let p = s.current_prompt().unwrap();
    assert_eq!(p.message, "Drawing units");
    s.input(Input::Text("furlong".into())).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Drawing units", "asks again");
    s.input(Input::Keyword("MM".into())).unwrap();
    s.input(Input::Enter).unwrap();
    s.input(Input::Enter).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Apply?");
    s.input(Input::Enter).unwrap();
    assert_eq!(boxes(s.doc().unwrap()).len(), 100);
}

#[test]
fn grid_change_relays_boxes_without_manual_moved() {
    let (mut s, b) = slab(4, 6600.0);
    let r = commit(&mut s, b, json!({ "pitchX": 700 }));
    assert_eq!(r["written"]["added"], 90);
    // Omitted, the stored pitch holds (ruling R16).
    let r = commit(&mut s, b, json!({}));
    assert!(r.get("written").is_none(), "{}", r["summary"]);
    // Another rib gives the default pitch: the stored grid differs, so boxes follow the new grid.
    let r = commit(&mut s, b, json!({ "rib": 140 }));
    assert_eq!(r["reset"], true);
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w == "GRID_CHANGED"), "{}", r["warnings"]);
    assert!(problem_cells(&r, "MANUAL_MOVED").is_empty(), "{}", r["problems"]);
    // Column 0 sits at x = 330 on both grids, so its 10 boxes stay.
    assert_eq!(r["written"], json!({ "added": 10, "updated": 80, "removed": 0 }));
    assert_eq!(r["summary"]["kept"], 10);
    let r = commit(&mut s, b, json!({}));
    assert_eq!((r["summary"]["kept"].as_u64(), r["reset"].as_bool()), (Some(100), Some(false)));
    assert!(r.get("written").is_none());

    // An explicit reset: false wins: the boxes stay where they are and read as moved.
    let r = preview(&mut s, b, json!({ "pitchX": 700, "reset": false }));
    assert_eq!(r["reset"], false);
    assert!(!problem_cells(&r, "MANUAL_MOVED").is_empty());
}

#[test]
fn bad_zone_names_its_handle() {
    let (mut s, b) = slab(4, 6600.0);
    // A self-intersecting bow tie, tagged directly (UBZONE refuses it).
    let z = s
        .doc_mut()
        .unwrap()
        .add(&Space::Model, Common::default(), poly(&[[1000.0, 1000.0], [2000.0, 2000.0], [2000.0, 1000.0], [1000.0, 2000.0]]))
        .unwrap();
    let tag =
        UbotTag::Zone { zone_id: "00112233445566aa".into(), zone_type: cadcraft_ubot::ZoneType::Exclusion, clearance_mm: None, alt_product_id: None };
    s.doc_mut().unwrap().modify_entity(z, |e| xdata::write(&mut e.common, &tag).unwrap()).unwrap();
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap_err().to_string();
    assert!(err.contains(&format!("zone {}", z.hex())), "{err}");
    // An alternative product of another size is refused with the handle too.
    let tag = UbotTag::Zone {
        zone_id: "00112233445566bb".into(),
        zone_type: cadcraft_ubot::ZoneType::AltProduct,
        clearance_mm: None,
        alt_product_id: Some("nope".into()),
    };
    s.doc_mut()
        .unwrap()
        .modify_entity(z, |e| {
            e.kind = square(1000.0, 1000.0, 100.0);
            xdata::write(&mut e.common, &tag).unwrap();
        })
        .unwrap();
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap_err().to_string();
    assert!(err.contains(&format!("zone {}", z.hex())) && err.contains("nope"), "{err}");
}

#[test]
fn malformed_tag_insert_and_tagged_minsert_cells_are_obstacles() {
    let (mut s, b) = slab(4, 6600.0);
    let d = s.doc_mut().unwrap();
    let broken = Common {
        xdata: vec![
            RawTag { code: 1001, value: "LPC_UBOT".into() },
            RawTag { code: 1000, value: "ubot/1".into() },
            RawTag { code: 1000, value: "I".into() },
        ],
        ..Common::default()
    };
    d.add(&Space::Model, broken, insert("JUNK", [330.0, 6270.0], 1, 1, 0.0)).unwrap();
    let mut other = Common::default();
    let tag = UbotTag::Instance {
        instance_id: "ffffffffffffffff:0:0".into(),
        product_id: "ubot-h13-7".into(),
        layout_id: "ffffffffffffffff".into(),
        count_multiplier: 1,
    };
    xdata::write(&mut other, &tag).unwrap();
    // Cells (0, 9) and (1, 9) of the default grid (centres y = 330).
    let minsert = d.add(&Space::Model, other, insert("UBOT_H13_7", [330.0, 330.0], 2, 1, 660.0)).unwrap();
    let before = doc(&s);
    let r = preview(&mut s, b, json!({}));
    assert_eq!(problem_cells(&r, "OVERLAPS_EXISTING"), vec![[0, 0], [0, 9], [1, 9]], "{}", r["problems"]);
    assert_eq!(r["summary"]["added"], 97);
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["written"]["added"], 97);
    let d = s.doc().unwrap();
    assert_eq!(d.entity(minsert), before.entity(minsert), "obstacles are never edited");
}

#[test]
fn hostile_inputs_fail_cleanly() {
    let (mut s, b) = slab(4, 6600.0);
    let before = doc(&s);
    let bad = |s: &mut Session, p: Value| s.execute("ubfill", &p).unwrap_err().to_string();
    assert!(bad(&mut s, json!({ "product": "ubot-h13-7" })).contains("boundary"));
    assert!(bad(&mut s, json!({ "boundary": "ZZ", "product": "ubot-h13-7" })).contains("boundary"));
    assert!(bad(&mut s, json!({ "boundary": "FFFFFF", "product": "ubot-h13-7" })).contains("FFFFFF"));
    assert!(bad(&mut s, json!({ "boundary": b.hex() })).contains("product"));
    assert!(bad(&mut s, json!({ "boundary": b.hex(), "product": "nope" })).contains("nope"));
    assert!(bad(&mut s, json!({ "boundary": b.hex(), "product": "ubot-h13-7", "rib": "wide" })).contains("rib"));
    assert!(bad(&mut s, json!({ "boundary": b.hex(), "product": "ubot-h13-7", "mode": "apply" })).contains("mode"));
    assert!(bad(&mut s, json!({ "boundary": b.hex(), "product": "ubot-h13-7", "pitchX": 100 })).contains("smaller"));
    assert!(bad(&mut s, json!({ "boundary": b.hex(), "product": "ubot-h13-7", "holes": [b.hex()] })).contains(&b.hex()));
    assert!(bad(&mut s, json!({ "boundary": b.hex(), "product": "ubot-h13-7", "origin": [1, "x"] })).contains("origin"));
    // NaN in the boundary, a slab far over the cell limit, a line as boundary.
    let d = s.doc_mut().unwrap();
    let nan = d.add(&Space::Model, Common::default(), poly(&[[0.0, 0.0], [f64::NAN, 0.0], [10.0, 10.0]])).unwrap();
    let huge = d.add(&Space::Model, Common::default(), square(0.0, 0.0, 1e9)).unwrap();
    let not_ring = d.add(&Space::Model, Common::default(), insert("JUNK", [0.0, 0.0], 1, 1, 0.0)).unwrap();
    let before_bad = doc(&s);
    for h in [nan, huge, not_ring] {
        let err = bad(&mut s, json!({ "boundary": h.hex(), "product": "ubot-h13-7", "mode": "commit" }));
        assert!(err.contains(&h.hex()) || err.contains("limit"), "{err}");
    }
    assert!(Arc::ptr_eq(&before_bad, &doc(&s)), "nothing written");
    // A tagged MINSERT with far more cells than the existing-box limit is refused before it is expanded.
    let mut tagged = Common::default();
    let tag = UbotTag::Instance {
        instance_id: "eeeeeeeeeeeeeeee:0:0".into(),
        product_id: "ubot-h13-7".into(),
        layout_id: "eeeeeeeeeeeeeeee".into(),
        count_multiplier: 1,
    };
    xdata::write(&mut tagged, &tag).unwrap();
    // Exactly at the limit (100 000 cells near the slab, none over a cell) the fill still runs.
    let many = s.doc_mut().unwrap().add(&Space::Model, tagged, insert("UBOT_H13_7", [7600.0, 0.0], 400, 250, 600.0)).unwrap();
    let t = std::time::Instant::now();
    let r = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap();
    assert_eq!(r["summary"]["added"], 100);
    eprintln!("100 000 existing cells: {:?}", t.elapsed());
    s.doc_mut()
        .unwrap()
        .modify_entity(many, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                (i.cols, i.rows) = (30_000, 30_000);
            }
        })
        .unwrap();
    let t = std::time::Instant::now();
    let err = bad(&mut s, json!({ "boundary": b.hex(), "product": "ubot-h13-7" }));
    assert!(err.contains("100000"), "{err}");
    assert!(t.elapsed().as_secs() < 5);
    // Out of the slab's reach the same array is never expanded and does not count.
    s.doc_mut()
        .unwrap()
        .modify_entity(many, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.insert.x = 1e8;
            }
        })
        .unwrap();
    let t = std::time::Instant::now();
    let r = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap();
    assert_eq!(r["summary"]["added"], 100);
    assert!(t.elapsed().as_secs() < 5);
    assert_eq!(before.entity(b), s.doc().unwrap().entity(b), "boundary untouched");
    assert_eq!(undo_len(&s), 0);
}

#[test]
#[ignore = "perf: run with --release -- --ignored --nocapture"]
fn commit_10k_boxes_perf() {
    let (mut s, b) = slab(4, 66000.0);
    let t = std::time::Instant::now();
    let r = commit(&mut s, b, json!({}));
    let first = t.elapsed();
    assert_eq!(r["written"]["added"], 10_000);
    let t = std::time::Instant::now();
    let r = commit(&mut s, b, json!({}));
    let rerun = t.elapsed();
    assert_eq!(r["summary"]["kept"], 10_000);
    eprintln!("UBFILL 10 000 boxes: commit {first:?}, no-op re-run {rerun:?}");
    assert!(first.as_secs_f64() < 2.0);
}

#[test]
fn moving_a_filled_floor_keeps_every_box() {
    let (mut s, b) = slab(4, 6600.0);
    commit(&mut s, b, json!({}));
    let all: Vec<String> = s.doc().unwrap().model.iter().map(|e| e.handle.hex()).collect();
    s.execute("move", &json!({ "handles": all, "delta": [100, 0] })).unwrap();

    let r = preview(&mut s, b, json!({}));
    let moved = problem_cells(&r, "MANUAL_MOVED").len();
    assert_eq!((r["summary"]["removed"].as_u64(), r["summary"]["kept"].as_u64(), moved), (Some(0), Some(100), 100), "{}", r["summary"]);
    let before = doc(&s);
    let r = commit(&mut s, b, json!({}));
    assert_eq!(r["summary"]["removed"], 0, "{}", r["summary"]);
    assert_eq!(boxes(s.doc().unwrap()).len(), 100);
    assert_eq!(boxes(s.doc().unwrap()), boxes(&before), "every box stays where the user put it");

    // reset: true still lays the grid again: boxes whose cell is no longer valid go.
    let r = preview(&mut s, b, json!({ "reset": true }));
    assert_eq!(r["summary"], json!({ "added": 0, "invalid": 20, "kept": 0, "removed": 10, "uncertain": 0, "updated": 90, "valid": 90 }));
}

/// Final review CC-I3: the 100 000 existing-box cap counts the boxes near this slab, not every floor of the drawing.
#[test]
fn existing_box_cap_counts_only_boxes_near_the_slab() {
    let (mut s, b) = slab(4, 6600.0);
    // 101 000 hand-inserted box cells on another floor, well beyond the reach of this slab.
    s.doc_mut().unwrap().add(&Space::Model, Common::default(), insert("UBOT_H13_7", [100_000.0, 0.0], 1000, 101, 600.0)).unwrap();
    let r = preview(&mut s, b, json!({}));
    assert_eq!(r["summary"]["added"], 100, "{}", r["summary"]);

    // The same array reaching into the slab still counts every cell.
    s.doc_mut().unwrap().add(&Space::Model, Common::default(), insert("UBOT_H13_7", [-100_000.0, 3000.0], 1000, 101, 600.0)).unwrap();
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap_err().to_string();
    assert!(err.contains("101000 existing boxes"), "{err}");
}

/// Final review CC-m7: a stored revision at the 32-bit limit is refused up front with a clear message; a no-op still runs.
#[test]
fn revision_at_the_limit_is_refused_before_commit() {
    let (mut s, b) = slab(4, 6600.0);
    commit(&mut s, b, json!({}));
    let (mut record, owner) = layout_of(s.doc().unwrap(), b);
    record.revision = i32::MAX as u32;
    let tag = UbotTag::Layout { record, boundary_handle: owner };
    s.doc_mut().unwrap().modify_entity(b, |e| xdata::write(&mut e.common, &tag).unwrap()).unwrap();
    let before = doc(&s);
    let r = preview(&mut s, b, json!({}));
    assert_eq!(r["summary"]["kept"], 100, "a no-op keeps the revision");
    let err = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h10-7" })).unwrap_err().to_string();
    assert!(err.contains("revision 2147483647") && err.contains(&b.hex()), "{err}");
    assert!(Arc::ptr_eq(&before, &doc(&s)), "nothing written");
}
