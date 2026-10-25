//! UBCOUNT, UBLEGEND and UBCHECK: box counts through nested blocks and MINSERT cells (optionally inside a slab
//! boundary), the quantity TABLE, and the drawing audit.

use std::sync::Arc;

use cadcraft_engine::Input;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::ubot::xdata::{self, UbotTag};
use cadcraft_engine::doc::{Block, Common, Drawing, Entity, EntityKind, Handle, Insert, Line, LwPolyline, Space};
use cadcraft_engine::geom::{PolyVertex, Vec2, Vec3};
use cadcraft_ubot::{ReplaceChange, ZoneType};
use serde_json::{Value, json};

fn poly(points: &[[f64; 2]], closed: bool) -> EntityKind {
    let vertices = points.iter().map(|p| PolyVertex { p: Vec2::new(p[0], p[1]), bulge: 0.0, start_width: 0.0, end_width: 0.0 }).collect();
    EntityKind::LwPolyline(LwPolyline { vertices, closed, const_width: 0.0, elevation: 0.0, plinegen: false })
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> EntityKind {
    poly(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)
}

fn square(x: f64, y: f64, size: f64) -> EntityKind {
    rect(x, y, x + size, y + size)
}

fn reference(block: &str, at: [f64; 2], rotation_deg: f64, scale: [f64; 2], grid: (u32, u32, f64, f64)) -> EntityKind {
    let (cols, rows, col_spacing, row_spacing) = grid;
    EntityKind::Insert(Insert {
        block: block.into(),
        insert: Vec3::new(at[0], at[1], 0.0),
        scale: Vec3::new(scale[0], scale[1], 1.0),
        rotation: rotation_deg.to_radians(),
        attribs: Vec::new(),
        cols,
        rows,
        col_spacing,
        row_spacing,
    })
}

fn one(block: &str, at: [f64; 2]) -> EntityKind {
    reference(block, at, 0.0, [1.0, 1.0], (1, 1, 0.0, 0.0))
}

fn box_tag(product: &str, id: &str) -> Common {
    let mut c = Common::default();
    let tag = UbotTag::Instance { instance_id: format!("{id}:0:0"), product_id: product.into(), layout_id: id.into(), count_multiplier: 1 };
    xdata::write(&mut c, &tag).unwrap();
    c
}

fn zone_tag(zone_type: ZoneType, alt: Option<&str>) -> Common {
    let mut c = Common::default();
    let tag = UbotTag::Zone { zone_id: "0123456789abcdef".into(), zone_type, clearance_mm: None, alt_product_id: alt.map(str::to_string) };
    xdata::write(&mut c, &tag).unwrap();
    c
}

fn add_block(d: &mut Drawing, name: &str, items: Vec<(Common, EntityKind)>) {
    let mut b = Block::new(name);
    for (common, kind) in items {
        let handle = d.new_handle();
        b.entities.push(Entity { handle, common, kind });
    }
    d.blocks.insert(name.into(), Arc::new(b));
}

fn session(d: Drawing) -> Session {
    let mut s = Session::empty();
    s.open_drawing(d, "ubot.dxf", None);
    s
}

/// A drawing in `insunits` with a square slab boundary from (0, 0).
fn slab(insunits: i64, size: f64) -> (Session, Handle) {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", insunits);
    let b = d.add(&Space::Model, Common::default(), square(0.0, 0.0, size)).unwrap();
    (session(d), b)
}

fn fill(s: &mut Session, b: Handle, extra: Value) -> Value {
    let mut p = json!({ "boundary": b.hex(), "product": "ubot-h13-7", "mode": "commit" });
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("ubfill", &p).unwrap()
}

/// Block `name` drawn as given, mapped to ubot-h13-7 with `anchor` (UB-1 style blocks of an LPC template).
fn mapped_block(s: &mut Session, name: &str, kind: EntityKind, anchor: &str) {
    add_block(s.doc_mut().unwrap(), name, vec![(Common::default(), kind)]);
    let overlay = json!({
        "schema": "ubot.catalog/1", "catalogId": "lpc-project", "version": "1", "products": [], "rulepacks": [],
        "mappings": [{ "productId": "ubot-h13-7", "cadEngine": "any", "blockName": name, "unitsMmPerBlockUnit": 1.0, "anchor": anchor, "rotationOffsetDeg": 0.0 }]
    });
    s.execute("ublib", &json!({ "action": "load", "text": overlay.to_string() })).unwrap();
}

fn undo_len(s: &Session) -> usize {
    s.state().unwrap().undo.len()
}

fn doc(s: &Session) -> Arc<Drawing> {
    s.state().unwrap().doc.clone()
}

fn add(s: &mut Session, common: Common, kind: EntityKind) -> Handle {
    s.doc_mut().unwrap().add(&Space::Model, common, kind).unwrap()
}

fn issues_of<'a>(r: &'a Value, code: &str) -> Vec<&'a Value> {
    r["issues"].as_array().unwrap().iter().filter(|i| i["code"] == code).collect()
}

fn count(s: &mut Session, p: Value) -> Value {
    s.execute("ubcount", &p).unwrap()
}

fn tables(d: &Drawing) -> Vec<(Handle, cadcraft_engine::doc::Table)> {
    d.model
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::Table(t) => Some((e.handle, t.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn ubcount_after_fill_matches() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    let half = add(&mut s, Common::default(), rect(0.0, 0.0, 3300.0, 6600.0));
    let (undo, before) = (undo_len(&s), doc(&s));

    let r = count(&mut s, json!({}));
    assert_eq!(r["rows"], json!([{ "productId": "ubot-h13-7", "name": "UBOT H13+7", "tagged": 100, "legacy": 0, "total": 100 }]));
    assert_eq!(r["total"], 100);
    assert!(r.get("boundary").is_none());
    assert!(s.log.iter().any(|l| l == "UBOT H13+7: 100 (tagged 100, legacy 0)"), "{:?}", s.log);

    let r = count(&mut s, json!({ "boundary": b.hex() }));
    assert_eq!((r["total"].as_u64(), r["boundary"].as_str()), (Some(100), Some(b.hex().as_str())));
    // Columns 0..4 have their centres at x = 330..2970, inside the left half.
    let r = count(&mut s, json!({ "boundary": half.hex() }));
    assert_eq!(r["rows"][0]["tagged"], 50);
    assert_eq!(r["total"], 50);

    assert_eq!(undo_len(&s), undo, "UBCOUNT is not an undo step");
    assert!(Arc::ptr_eq(&before, &doc(&s)));
}

#[test]
fn ubcount_nested_cluster_and_minsert_not_double_counted() {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    add_block(&mut d, "UBOT_H13_7", vec![(Common::default(), square(-260.0, -260.0, 520.0))]);
    let boxes =
        [[0.0, 0.0], [700.0, 0.0], [0.0, 700.0], [700.0, 700.0]].iter().map(|p| (box_tag("ubot-h13-7", "aaaa"), one("UBOT_H13_7", *p))).collect();
    add_block(&mut d, "CLUSTER", boxes);
    add_block(&mut d, "CLUSTER2", vec![(Common::default(), one("CLUSTER", [0.0, 0.0])), (Common::default(), one("CLUSTER", [2000.0, 0.0]))]);
    add_block(&mut d, "CLUSTER3", vec![(Common::default(), reference("CLUSTER2", [0.0, 0.0], 0.0, [1.0, 1.0], (2, 2, 5000.0, 5000.0)))]);
    for kind in [
        one("CLUSTER2", [0.0, 0.0]),
        reference("CLUSTER", [10000.0, 0.0], 0.0, [1.0, 1.0], (3, 2, 1500.0, 1500.0)),
        reference("CLUSTER2", [0.0, 10000.0], 0.0, [1.0, 1.0], (2, 1, 5000.0, 0.0)),
        one("CLUSTER3", [20000.0, 20000.0]),
    ] {
        d.add(&Space::Model, Common::default(), kind).unwrap();
    }
    let outline = d.add(&Space::Model, Common::default(), square(-1000.0, -1000.0, 40000.0)).unwrap();
    let mut s = session(d);
    // 8 (cluster of two clusters) + 4 × 3 × 2 (MINSERT) + 8 × 2 (MINSERT of the nested cluster) + 8 × 4 (three levels).
    let expected = 8 + 24 + 16 + 32;
    for p in [json!({}), json!({ "boundary": outline.hex() })] {
        let r = count(&mut s, p);
        assert_eq!(r["rows"], json!([{ "productId": "ubot-h13-7", "name": "UBOT H13+7", "tagged": expected, "legacy": 0, "total": expected }]));
        assert_eq!(r["total"], expected);
    }
}

#[test]
fn ubcount_legacy_ub_blocks() {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    add_block(&mut d, "UB-1", vec![(Common::default(), square(-260.0, -520.0, 520.0))]);
    for x in [0.0, 1000.0, 2000.0] {
        d.add(&Space::Model, Common::default(), one("UB-1", [x, 0.0])).unwrap();
    }
    d.add(&Space::Model, Common::default(), reference("UB-1", [0.0, 3000.0], 0.0, [1.0, 1.0], (2, 1, 700.0, 0.0))).unwrap();
    let mut s = session(d);
    let r = count(&mut s, json!({}));
    assert_eq!(r["rows"], json!([{ "productId": null, "name": "UB-1", "tagged": 0, "legacy": 5, "total": 5 }]));
    assert!(s.log.iter().any(|l| l == "UB-1: 5 (tagged 0, legacy 5)"), "{:?}", s.log);

    let overlay = json!({
        "schema": "ubot.catalog/1", "catalogId": "lpc-project", "version": "1", "products": [], "rulepacks": [],
        "mappings": [{ "productId": "ubot-h10-7", "cadEngine": "any", "blockName": "UB-1", "unitsMmPerBlockUnit": 1.0, "anchor": "top_center", "rotationOffsetDeg": 0.0 }]
    });
    s.execute("ublib", &json!({ "action": "load", "text": overlay.to_string() })).unwrap();
    let r = count(&mut s, json!({}));
    assert_eq!(r["rows"], json!([{ "productId": "ubot-h10-7", "name": "UBOT H10+7", "tagged": 0, "legacy": 5, "total": 5 }]));
}

#[test]
fn ublegend_inserts_table_one_undo() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    let undo = undo_len(&s);
    let r = s.execute("ublegend", &json!({ "at": [8000.0, 6600.0] })).unwrap();
    assert_eq!((r["rows"].as_u64(), r["total"].as_u64()), (Some(1), Some(100)));
    assert_eq!(undo_len(&s), undo + 1, "one undo step");
    let d = doc(&s);
    let found = tables(&d);
    assert_eq!(found.len(), 1);
    let (h, t) = &found[0];
    assert_eq!(r["handle"], h.hex());
    let text: Vec<Vec<&str>> = t.cells.iter().map(|row| row.iter().map(|c| c.text.as_str()).collect()).collect();
    assert_eq!(text, vec![vec!["THỐNG KÊ HỘP UBOT", ""], vec!["Loại hộp", "Số lượng (hộp)"], vec!["UBOT H13+7", "100"], vec!["Tổng", "100"]]);
    assert!(t.title && t.header);
    assert_eq!(t.text_height, 250.0, "250 mm in a mm drawing");
    assert_eq!((t.insert.x, t.insert.y), (8000.0, 6600.0));
    assert_eq!(t.cells[0][0].merged, Some((1, 2)), "the title spans both columns");
    assert!(t.col_widths.iter().all(|w| *w > 250.0), "{:?}", t.col_widths);

    s.execute("u", &json!({})).unwrap();
    assert!(tables(s.doc().unwrap()).is_empty(), "undo removes the table");

    // Interactive: one prompt for the insertion point; a cm drawing gets 25 units of text.
    let (mut s, b) = slab(5, 660.0);
    fill(&mut s, b, json!({}));
    let undo = undo_len(&s);
    s.start("ublegend").unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Insertion point");
    s.input(Input::Point(Vec2::new(800.0, 0.0))).unwrap();
    assert!(s.running.is_none());
    assert_eq!(undo_len(&s), undo + 1);
    let found = tables(s.doc().unwrap());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.text_height, 25.0);
    assert_eq!(found[0].1.cells[3][1].text, "100");
}

#[test]
fn ublegend_with_nothing_to_count_errors() {
    let (mut s, b) = slab(4, 6600.0);
    add(&mut s, Common::default(), EntityKind::Line(Line { a: Vec3::new(0.0, 0.0, 0.0), b: Vec3::new(10.0, 0.0, 0.0) }));
    let (undo, before) = (undo_len(&s), doc(&s));
    let e = s.execute("ublegend", &json!({ "at": [0.0, 0.0] })).unwrap_err();
    assert!(e.to_string().contains("no U-BOT boxes to count"), "{e}");
    // At the prompt the point is refused with the same message and the command asks again (AutoCAD re-prompts).
    s.start("ublegend").unwrap();
    s.input(Input::Point(Vec2::new(0.0, 0.0))).unwrap();
    assert!(s.log.iter().any(|l| l.contains("no U-BOT boxes to count")));
    assert!(s.running.is_some());
    s.cancel();
    assert!(s.running.is_none());
    assert_eq!(undo_len(&s), undo);
    assert!(Arc::ptr_eq(&before, &doc(&s)));
    fill(&mut s, b, json!({}));
    let far = add(&mut s, Common::default(), square(50000.0, 0.0, 1000.0));
    let (undo2, before2) = (undo_len(&s), doc(&s));
    let e = s.execute("ublegend", &json!({ "at": [0.0, 0.0], "boundary": far.hex() })).unwrap_err();
    assert!(e.to_string().contains("no U-BOT boxes to count"), "{e}");
    assert_eq!(undo_len(&s), undo2);
    assert!(Arc::ptr_eq(&before2, &doc(&s)));
    assert!(undo2 > undo && !Arc::ptr_eq(&before, &before2));
}

#[test]
fn ubcheck_clean_after_fill_is_empty() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    let (undo, before) = (undo_len(&s), doc(&s));
    let r = s.execute("ubcheck", &json!({})).unwrap();
    assert_eq!(r["issues"], json!([]));
    assert_eq!(r["counts"], json!({}));
    assert_eq!(r["total"], 0);
    assert!(s.log.iter().any(|l| l == "UBCHECK: no issues."), "{:?}", s.log);
    assert_eq!(undo_len(&s), undo, "UBCHECK is not an undo step");
    assert!(Arc::ptr_eq(&before, &doc(&s)));
}

#[test]
fn ubcheck_duplicate_after_copy() {
    let (mut s, b) = slab(4, 6600.0);
    let r = fill(&mut s, b, json!({}));
    let id = r["add"][0]["instanceId"].as_str().unwrap().to_string();
    let original = s
        .doc()
        .unwrap()
        .model
        .iter()
        .find(|e| matches!(xdata::read(&e.common), Some(Ok(UbotTag::Instance { ref instance_id, .. })) if *instance_id == id))
        .unwrap()
        .handle;
    let copied = s.execute("copy", &json!({ "handles": [original.hex()], "delta": [0.0, -10000.0] })).unwrap();
    let copy = copied["handles"][0].as_str().unwrap().to_string();
    let r = s.execute("ubcheck", &json!({})).unwrap();
    let dups = issues_of(&r, "DUPLICATE_ID");
    assert_eq!(dups.len(), 1, "{r}");
    assert_eq!(dups[0]["handle"], copy.as_str());
    assert_eq!(dups[0]["severity"], "error");
    assert_eq!(r["counts"]["DUPLICATE_ID"], 1);
    let line = format!("DUPLICATE_ID {copy} at 330,-3730: instance id {id} is used by another box");
    assert!(s.log.contains(&line), "{line} not in {:?}", s.log);
}

#[test]
fn ubcheck_stale_after_boundary_stretch() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    s.doc_mut()
        .unwrap()
        .modify_entity(b, |e| {
            if let EntityKind::LwPolyline(pl) = &mut e.kind {
                pl.vertices[2].p = Vec2::new(7000.0, 6600.0);
            }
        })
        .unwrap();
    let r = s.execute("ubcheck", &json!({})).unwrap();
    let stale = issues_of(&r, "STALE_LAYOUT");
    assert_eq!(stale.len(), 1, "{r}");
    assert_eq!(stale[0]["handle"], b.hex());
    assert_eq!(stale[0]["severity"], "warning");
    assert_eq!(r["total"], 1, "the boxes still fit: {r}");
}

/// A legacy UB-1 (top-centre anchor) mirrored with scale (1, -1) at 30°: its footprint centre is where its
/// geometry is, for the fill (as an obstacle), the check and the count (ruling R21).
#[test]
fn mirrored_ub1_keeps_its_centre_in_fill_check_and_count() {
    let (r30, up) = (30f64.to_radians(), |a: f64, d: f64| Vec2::new(-d * a.sin(), d * a.cos()));
    // Fill obstacle: an untagged mirrored UB-1 whose box sits exactly on cell (4, 4) blocks that cell only.
    let (mut s, b) = slab(4, 6600.0);
    mapped_block(&mut s, "UB-1", square(-260.0, -520.0, 520.0), "top_center");
    let centre = Vec2::new(330.0 + 4.0 * 660.0, 6270.0 - 4.0 * 660.0);
    let at = centre - up(r30, 260.0);
    add(&mut s, Common::default(), reference("UB-1", [at.x, at.y], 30.0, [1.0, -1.0], (1, 1, 0.0, 0.0)));
    let r = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7" })).unwrap();
    let blocked: Vec<Value> = r["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["reasons"].as_array().unwrap().iter().any(|x| x == "OVERLAPS_EXISTING"))
        .map(|p| p["cell"].clone())
        .collect();
    assert_eq!(blocked, vec![json!([4, 4])], "{}", r["problems"]);
    // Counted inside a boundary around its true centre, not around the point its insert would suggest.
    let around = add(&mut s, Common::default(), rect(centre.x - 100.0, centre.y - 100.0, centre.x + 100.0, centre.y + 100.0));
    assert_eq!(count(&mut s, json!({ "boundary": around.hex() }))["total"], 1);

    // Check: a box of a 30° fill mirrored about its own axis stays on its cell.
    let (mut s, b) = slab(4, 6600.0);
    mapped_block(&mut s, "UB-1", square(-260.0, -520.0, 520.0), "top_center");
    let r = fill(&mut s, b, json!({ "angle": 30 }));
    let added = r["summary"]["added"].as_u64().unwrap();
    let adds = r["add"].as_array().unwrap();
    let cell = adds[adds.len() / 2].clone();
    let c = Vec2::new(cell["center"][0].as_f64().unwrap(), cell["center"][1].as_f64().unwrap());
    let h = s
        .doc()
        .unwrap()
        .model
        .iter()
        .find(|e| matches!(xdata::read(&e.common), Some(Ok(UbotTag::Instance { ref instance_id, .. })) if *instance_id == cell["instanceId"].as_str().unwrap()))
        .unwrap()
        .handle;
    let axis = c + Vec2::new(r30.cos(), r30.sin());
    s.execute("mirror", &json!({ "handles": [h.hex()], "p1": [c.x, c.y], "p2": [axis.x, axis.y], "erase": true })).unwrap();
    let mirrored = s
        .doc()
        .unwrap()
        .model
        .iter()
        .find_map(|e| match (&e.kind, xdata::read(&e.common)) {
            (EntityKind::Insert(i), Some(Ok(UbotTag::Instance { instance_id, .. }))) if instance_id == cell["instanceId"].as_str().unwrap() => {
                Some(i.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(mirrored.scale.x * mirrored.scale.y < 0.0, "mirrored: {:?}", mirrored.scale);
    let r = s.execute("ubcheck", &json!({})).unwrap();
    assert_eq!(r["issues"], json!([]), "no false MANUAL_MOVED or OVERLAP");
    let r = s.execute("ubfill", &json!({ "boundary": b.hex(), "product": "ubot-h13-7", "angle": 30 })).unwrap();
    assert_eq!(r["summary"]["kept"].as_u64(), Some(added), "{}", r["summary"]);
    assert!(
        r["problems"].as_array().unwrap().iter().all(|p| p["reasons"].as_array().unwrap().iter().all(|x| x != "MANUAL_MOVED")),
        "{}",
        r["problems"]
    );
}

#[test]
fn ubcheck_reports_a_broken_zone_and_goes_on() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    let bow_tie =
        add(&mut s, zone_tag(ZoneType::Exclusion, None), poly(&[[1000.0, 1000.0], [2000.0, 2000.0], [2000.0, 1000.0], [1000.0, 2000.0]], true));
    let open = add(&mut s, zone_tag(ZoneType::Column, None), poly(&[[3000.0, 1000.0], [3500.0, 1000.0], [3500.0, 1500.0]], false));
    let nan = add(&mut s, zone_tag(ZoneType::Solid, None), poly(&[[f64::NAN, 0.0], [100.0, 0.0], [100.0, 100.0]], true));
    let good = add(&mut s, zone_tag(ZoneType::Exclusion, None), rect(100.0, 6050.0, 560.0, 6500.0));
    let (undo, before) = (undo_len(&s), doc(&s));
    let r = s.execute("ubcheck", &json!({})).unwrap();
    let invalid: Vec<&str> = issues_of(&r, "XDATA_INVALID").iter().map(|i| i["handle"].as_str().unwrap()).collect();
    for h in [bow_tie, open, nan] {
        assert!(invalid.contains(&h.hex().as_str()), "{} not reported: {r}", h.hex());
    }
    let in_zone = issues_of(&r, "IN_ZONE");
    assert_eq!(in_zone.len(), 1, "the good zone is still checked: {r}");
    assert!(in_zone[0]["message"].as_str().unwrap().contains("0123456789abcdef"));
    assert_eq!(issues_of(&r, "STALE_LAYOUT").len(), 1, "the good zone changed the slab: {r}");
    assert!(invalid.iter().all(|h| *h != good.hex()));
    assert_eq!(undo_len(&s), undo);
    assert!(Arc::ptr_eq(&before, &doc(&s)));
}

#[test]
fn ubcheck_flags_inconsistent_alt_zones() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    let no_alt = add(&mut s, zone_tag(ZoneType::AltProduct, None), square(30000.0, 0.0, 500.0));
    let stray_alt = add(&mut s, zone_tag(ZoneType::Exclusion, Some("ubot-h10-7")), square(31000.0, 0.0, 500.0));
    let unknown = add(&mut s, zone_tag(ZoneType::AltProduct, Some("nope")), square(32000.0, 0.0, 500.0));
    let fine = add(&mut s, zone_tag(ZoneType::AltProduct, Some("ubot-h10-7")), square(33000.0, 0.0, 500.0));
    let r = s.execute("ubcheck", &json!({})).unwrap();
    let issues = r["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 3, "{r}");
    let of = |h: Handle| issues.iter().find(|i| i["handle"] == h.hex()).map(|i| i["code"].as_str().unwrap().to_string());
    assert_eq!(of(no_alt).as_deref(), Some("XDATA_INVALID"));
    assert_eq!(of(stray_alt).as_deref(), Some("XDATA_INVALID"));
    assert_eq!(of(unknown).as_deref(), Some("UNKNOWN_PRODUCT"));
    assert_eq!(of(fine), None);
    assert_eq!(r["counts"], json!({ "UNKNOWN_PRODUCT": 1, "XDATA_INVALID": 2 }));
}

#[test]
fn ubcount_boundary_with_nested_cluster_minsert_and_rotation() {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    add_block(&mut d, "UBOT_H13_7", vec![(Common::default(), square(-260.0, -260.0, 520.0))]);
    add_block(&mut d, "UB-1", vec![(Common::default(), square(-260.0, -520.0, 520.0))]);
    add_block(
        &mut d,
        "PAIR",
        vec![(box_tag("ubot-h13-7", "aaaa"), one("UBOT_H13_7", [0.0, 0.0])), (box_tag("ubot-h13-7", "aaaa"), one("UBOT_H13_7", [1000.0, 0.0]))],
    );
    add_block(&mut d, "OUTER", vec![(Common::default(), reference("PAIR", [0.0, 0.0], 90.0, [1.0, 1.0], (1, 1, 0.0, 0.0)))]);
    let tagged = box_tag("ubot-h13-7", "bbbb");
    for (common, kind) in [
        // Boxes at (5000, 5000) and (5000, 6000).
        (Common::default(), reference("PAIR", [5000.0, 5000.0], 90.0, [1.0, 1.0], (1, 1, 0.0, 0.0))),
        // Boxes at x = 0, 1000, 3000, 4000, 6000, 7000.
        (Common::default(), reference("PAIR", [0.0, 0.0], 0.0, [1.0, 1.0], (3, 1, 3000.0, 0.0))),
        // 90° + 90°: boxes at (20000, 0) and (19000, 0).
        (Common::default(), reference("OUTER", [20000.0, 0.0], 90.0, [1.0, 1.0], (1, 1, 0.0, 0.0))),
        // A tagged MINSERT turned 90°: its columns run up, cells at y = 10000, 11000, 12000.
        (tagged, reference("UBOT_H13_7", [0.0, 10000.0], 90.0, [1.0, 1.0], (3, 1, 1000.0, 0.0))),
        // Scaled twice: boxes at (30000, 0) and (32000, 0).
        (Common::default(), reference("PAIR", [30000.0, 0.0], 0.0, [2.0, 2.0], (1, 1, 0.0, 0.0))),
        // Legacy UB-1 of unknown size: its box is the block's extents, centred 260 below the insert.
        (Common::default(), one("UB-1", [40000.0, 0.0])),
    ] {
        d.add(&Space::Model, common, kind).unwrap();
    }
    let mut zones = Vec::new();
    for (x0, y0, x1, y1) in [
        (-500.0, -500.0, 4500.0, 11500.0),
        (4500.0, 4500.0, 5500.0, 6500.0),
        (18500.0, -500.0, 19500.0, 500.0),
        (31500.0, -500.0, 32500.0, 500.0),
        (39800.0, -400.0, 40200.0, -100.0),
        (39800.0, 100.0, 40200.0, 400.0),
    ] {
        zones.push(d.add(&Space::Model, Common::default(), rect(x0, y0, x1, y1)).unwrap());
    }
    let mut s = session(d);
    assert_eq!(count(&mut s, json!({}))["total"], 16);
    let got: Vec<u64> = zones.iter().map(|z| count(&mut s, json!({ "boundary": z.hex() }))["total"].as_u64().unwrap()).collect();
    assert_eq!(got, vec![6, 2, 1, 1, 1, 0]);
    let r = count(&mut s, json!({ "boundary": zones[0].hex() }));
    assert_eq!(r["rows"], json!([{ "productId": "ubot-h13-7", "name": "UBOT H13+7", "tagged": 6, "legacy": 0, "total": 6 }]));
}

#[test]
fn untagged_generated_block_is_counted_once() {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    add_block(&mut d, "UBOT_H13_7", vec![(Common::default(), square(-260.0, -260.0, 520.0))]);
    add_block(&mut d, "ROW", vec![(Common::default(), one("UBOT_H13_7", [0.0, 0.0])), (Common::default(), one("ubot_h13_7", [700.0, 0.0]))]);
    for kind in [one("UBOT_H13_7", [0.0, 5000.0]), one("ROW", [0.0, 0.0]), one("ROW", [0.0, 2000.0])] {
        d.add(&Space::Model, Common::default(), kind).unwrap();
    }
    let all = d.add(&Space::Model, Common::default(), square(-1000.0, -1000.0, 10000.0)).unwrap();
    let mut s = session(d);
    for p in [json!({}), json!({ "boundary": all.hex() })] {
        let r = count(&mut s, p);
        assert_eq!(r["rows"], json!([{ "productId": "ubot-h13-7", "name": "UBOT H13+7", "tagged": 0, "legacy": 5, "total": 5 }]));
    }
}

#[test]
fn hostile_params_fail_cleanly() {
    let (mut s, b) = slab(4, 6600.0);
    fill(&mut s, b, json!({}));
    let line = add(&mut s, Common::default(), EntityKind::Line(Line { a: Vec3::new(0.0, 0.0, 0.0), b: Vec3::new(10.0, 0.0, 0.0) }));
    let (undo, before) = (undo_len(&s), doc(&s));
    let cases = [
        ("ubcount", json!({ "boundary": "zz" })),
        ("ubcount", json!({ "boundary": 12.5 })),
        ("ubcount", json!({ "boundary": "FFFFFF" })),
        ("ubcount", json!({ "boundary": line.hex() })),
        ("ubcount", json!({ "boundary": b.hex(), "units": "furlong" })),
        ("ublegend", json!({})),
        ("ublegend", json!({ "at": [0.0, 0.0], "textHeight": -1 })),
        ("ublegend", json!({ "at": [0.0, 0.0], "textHeight": 1e300 })),
        ("ublegend", json!({ "at": [0.0, 0.0], "boundary": line.hex() })),
        ("ubcheck", json!({ "units": "furlong" })),
    ];
    for (cmd, p) in cases {
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        assert_eq!(undo_len(&s), undo, "{cmd} {p}");
        assert!(Arc::ptr_eq(&before, &doc(&s)), "{cmd} {p}");
    }
    // Typed at the command line, UBCOUNT and UBCHECK run over model space.
    s.start("ubcount").unwrap();
    assert!(s.log.iter().any(|l| l == "UBCOUNT: 100 boxes."), "{:?}", s.log);
    s.start("ubcheck").unwrap();
    assert!(s.log.iter().any(|l| l == "UBCHECK: no issues."), "{:?}", s.log);
    // Without INSUNITS the check needs `units`.
    s.doc_mut().unwrap().header.set_i64("INSUNITS", 0);
    assert!(s.execute("ubcheck", &json!({})).is_err());
    assert_eq!(s.execute("ubcheck", &json!({ "units": "mm" })).unwrap()["total"], 0);
}

#[test]
fn replace_change_tolerates_a_missing_scale_xy() {
    let c: ReplaceChange = serde_json::from_value(json!({
        "handle": "1F", "productId": "ubot-h10-7", "blockName": "UBOT_H10_7", "insert": [0.0, 0.0], "rotationDeg": 0.0, "scale": 1.0
    }))
    .unwrap();
    assert_eq!(c.scale_xy, [0.0, 0.0]);
}

#[test]
#[ignore = "perf: cargo test --release -p cadcraft-engine --test ubot_count_check -- --ignored --nocapture"]
fn count_and_check_10k_boxes_perf() {
    let (mut s, b) = slab(4, 66000.0);
    assert_eq!(fill(&mut s, b, json!({}))["written"]["added"], 10_000);
    let t = std::time::Instant::now();
    assert_eq!(count(&mut s, json!({}))["total"], 10_000);
    let all = t.elapsed();
    let t = std::time::Instant::now();
    assert_eq!(count(&mut s, json!({ "boundary": b.hex() }))["total"], 10_000);
    let inside = t.elapsed();
    let t = std::time::Instant::now();
    assert_eq!(s.execute("ubcheck", &json!({})).unwrap()["total"], 0);
    let check = t.elapsed();
    eprintln!("10 000 boxes: UBCOUNT {all:?}, UBCOUNT with boundary {inside:?}, UBCHECK {check:?}");
    assert!(check.as_secs_f64() < 2.0);
}
