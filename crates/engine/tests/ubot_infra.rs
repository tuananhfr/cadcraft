//! U-BOT engine plumbing: the LPC_UBOT xdata record, drawing readers, unit scale, UBLIB and UBZONE.

use std::sync::Arc;

use cadcraft_engine::Input;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::cmd::ubot::read::{count_items, existing_instances, invalid_tags, zones};
use cadcraft_engine::cmd::ubot::units::unit_scale;
use cadcraft_engine::cmd::ubot::xdata::{self, APP, UbotTag};
use cadcraft_engine::doc::{Block, Circle, Common, Drawing, Entity, EntityKind, Handle, Insert, Line, LwPolyline, RawTag, Space};
use cadcraft_engine::geom::{PolyVertex, Vec2, Vec3};
use cadcraft_ubot::{Catalog, CountSource, LayoutRecord, ZoneType};
use serde_json::{Value, json};

fn tag(code: i32, value: &str) -> RawTag {
    RawTag { code, value: value.into() }
}

fn instance_tag() -> UbotTag {
    UbotTag::Instance {
        instance_id: "0123456789abcdef:3:4".into(),
        product_id: "ubot-h13-7".into(),
        layout_id: "0123456789abcdef".into(),
        count_multiplier: 3,
    }
}

fn layout_tag() -> UbotTag {
    UbotTag::Layout {
        record: LayoutRecord {
            layout_id: "0123456789abcdef".into(),
            revision: 3,
            product_id: "ubot-h13-7".into(),
            rulepack: "lpc-draft-1@1".into(),
            origin: [1234.5678, -98.765],
            angle_deg: 12.5,
            pitch_x_mm: 660.0,
            pitch_y_mm: 640.25,
            unit_scale: 10.0,
            geometry_hash: "ab".repeat(32),
        },
        boundary_handle: "2F".into(),
    }
}

fn zone_tag(clearance_mm: Option<f64>, alt: Option<&str>) -> UbotTag {
    UbotTag::Zone {
        zone_id: "fedcba9876543210".into(),
        zone_type: if alt.is_some() { ZoneType::AltProduct } else { ZoneType::Column },
        clearance_mm,
        alt_product_id: alt.map(str::to_string),
    }
}

fn poly(points: &[[f64; 2]], closed: bool) -> EntityKind {
    let vertices = points.iter().map(|p| PolyVertex { p: Vec2::new(p[0], p[1]), bulge: 0.0, start_width: 0.0, end_width: 0.0 }).collect();
    EntityKind::LwPolyline(LwPolyline { vertices, closed, const_width: 0.0, elevation: 0.0, plinegen: false })
}

fn square(x: f64, y: f64, size: f64) -> EntityKind {
    poly(&[[x, y], [x + size, y], [x + size, y + size], [x, y + size]], true)
}

fn insert(block: &str, at: [f64; 2], cols: u32, rows: u32) -> EntityKind {
    EntityKind::Insert(Insert {
        block: block.into(),
        insert: Vec3::new(at[0], at[1], 0.0),
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs: Vec::new(),
        cols,
        rows,
        col_spacing: 700.0,
        row_spacing: 700.0,
    })
}

fn tagged(kind: EntityKind, t: Option<&UbotTag>) -> (Common, EntityKind) {
    let mut c = Common::default();
    if let Some(t) = t {
        xdata::write(&mut c, t).unwrap();
    }
    (c, kind)
}

fn add_block(d: &mut Drawing, name: &str, items: Vec<(Common, EntityKind)>) {
    let mut b = Block::new(name);
    for (common, kind) in items {
        let handle = d.new_handle();
        b.entities.push(Entity { handle, common, kind });
    }
    d.blocks.insert(name.into(), Arc::new(b));
}

fn metric_session(d: Drawing) -> Session {
    let mut s = Session::empty();
    s.open_drawing(d, "ubot.dxf", None);
    s
}

fn undo_len(s: &Session) -> usize {
    s.state().unwrap().undo.len()
}

fn read_tag(s: &Session, h: Handle) -> Option<Result<UbotTag, String>> {
    xdata::read(&s.doc().unwrap().entity(h).unwrap().common)
}

#[test]
fn xdata_write_then_read_each_kind() {
    for t in [instance_tag(), layout_tag(), zone_tag(None, None), zone_tag(Some(150.0), None), zone_tag(None, Some("ubot-h10-7"))] {
        let mut c = Common::default();
        xdata::write(&mut c, &t).unwrap();
        assert_eq!(c.xdata.first(), Some(&tag(1001, APP)));
        assert_eq!(c.xdata.get(1), Some(&tag(1000, "ubot/1")), "version is always the first field");
        assert_eq!(xdata::read(&c), Some(Ok(t.clone())));
        // Writing again replaces the record rather than adding a second one.
        xdata::write(&mut c, &t).unwrap();
        assert_eq!(c.xdata.iter().filter(|r| r.code == 1001).count(), 1);
    }
    // Spec §4.1 field order, read by position.
    let mut c = Common::default();
    xdata::write(&mut c, &zone_tag(None, None)).unwrap();
    let codes: Vec<(i32, &str)> = c.xdata.iter().map(|r| (r.code, r.value.as_str())).collect();
    assert_eq!(codes, [(1001, APP), (1000, "ubot/1"), (1000, "Z"), (1000, "fedcba9876543210"), (1000, "column"), (1040, "-1"), (1000, "")]);
    let mut c = Common::default();
    xdata::write(&mut c, &layout_tag()).unwrap();
    let hash = "ab".repeat(32);
    let codes: Vec<(i32, &str)> = c.xdata.iter().map(|r| (r.code, r.value.as_str())).collect();
    assert_eq!(
        codes,
        [
            (1001, APP),
            (1000, "ubot/1"),
            (1000, "L"),
            (1000, "0123456789abcdef"),
            (1071, "3"),
            (1000, "ubot-h13-7"),
            (1000, "lpc-draft-1@1"),
            (1040, "1234.5678"),
            (1040, "-98.765"),
            (1040, "12.5"),
            (1040, "660"),
            (1040, "640.25"),
            (1040, "10"),
            (1000, hash.as_str()),
            (1000, "2F"),
        ]
    );
    let mut c = Common::default();
    xdata::write(&mut c, &instance_tag()).unwrap();
    let codes: Vec<(i32, &str)> = c.xdata.iter().map(|r| (r.code, r.value.as_str())).collect();
    assert_eq!(
        codes,
        [(1001, APP), (1000, "ubot/1"), (1000, "I"), (1000, "0123456789abcdef:3:4"), (1000, "ubot-h13-7"), (1000, "0123456789abcdef"), (1070, "3")]
    );
}

#[test]
fn xdata_write_keeps_other_apps() {
    let mut c = Common {
        xdata: vec![
            tag(1001, "ACME"),
            tag(1000, "a"),
            tag(1001, APP),
            tag(1000, "ubot/1"),
            tag(1000, "Z"),
            tag(1000, "old"),
            tag(1000, "wall"),
            tag(1040, "5"),
            tag(1000, ""),
            tag(1001, "PE_URL"),
            tag(1000, "https://example.com"),
        ],
        ..Common::default()
    };
    xdata::write(&mut c, &instance_tag()).unwrap();
    assert_eq!(c.xdata.get(..2), Some(&[tag(1001, "ACME"), tag(1000, "a")][..]));
    assert_eq!(c.xdata.get(2), Some(&tag(1001, APP)));
    assert_eq!(c.xdata.get(c.xdata.len() - 2..), Some(&[tag(1001, "PE_URL"), tag(1000, "https://example.com")][..]));
    assert_eq!(xdata::read(&c), Some(Ok(instance_tag())));

    // No record yet: appended after the others.
    let mut c = Common { xdata: vec![tag(1001, "ACME"), tag(1000, "a")], ..Common::default() };
    xdata::write(&mut c, &zone_tag(Some(0.0), None)).unwrap();
    assert_eq!(c.xdata.get(..3), Some(&[tag(1001, "ACME"), tag(1000, "a"), tag(1001, APP)][..]));
    xdata::clear(&mut c);
    assert_eq!(c.xdata, vec![tag(1001, "ACME"), tag(1000, "a")]);
    assert_eq!(xdata::read(&c), None);
}

#[test]
fn xdata_wrong_field_count_is_invalid() {
    let record = |fields: Vec<RawTag>| Common { xdata: std::iter::once(tag(1001, APP)).chain(fields).collect(), ..Common::default() };
    let good = || vec![tag(1000, "ubot/1"), tag(1000, "I"), tag(1000, "L:0:0"), tag(1000, "ubot-h13-7"), tag(1000, "L"), tag(1070, "1")];
    assert!(matches!(xdata::read(&record(good())), Some(Ok(UbotTag::Instance { .. }))));

    let mut short = good();
    short.pop();
    let mut long = good();
    long.push(tag(1000, "extra"));
    let mut wrong_type = good();
    wrong_type[5] = tag(1040, "1");
    let mut bad_number = good();
    bad_number[5] = tag(1070, "many");
    let mut zero = good();
    zero[5] = tag(1070, "0");
    let mut version = good();
    version[0] = tag(1000, "ubot/2");
    let mut kind = good();
    kind[1] = tag(1000, "Q");
    let zone_negative = vec![tag(1000, "ubot/1"), tag(1000, "Z"), tag(1000, "z1"), tag(1000, "solid"), tag(1040, "-5"), tag(1000, "")];
    let zone_nan = vec![tag(1000, "ubot/1"), tag(1000, "Z"), tag(1000, "z1"), tag(1000, "solid"), tag(1040, "NaN"), tag(1000, "")];
    let zone_type = vec![tag(1000, "ubot/1"), tag(1000, "Z"), tag(1000, "z1"), tag(1000, "pool"), tag(1040, "0"), tag(1000, "")];
    for (what, fields) in [
        ("short", short),
        ("long", long),
        ("wrong type", wrong_type),
        ("bad number", bad_number),
        ("zero multiplier", zero),
        ("version", version),
        ("kind", kind),
        ("negative clearance", zone_negative),
        ("NaN clearance", zone_nan),
        ("zone type", zone_type),
        ("empty", vec![]),
    ] {
        assert!(matches!(xdata::read(&record(fields)), Some(Err(_))), "{what}");
    }
    let other = Common { xdata: vec![tag(1001, "ACME"), tag(1000, "ubot/1")], ..Common::default() };
    assert_eq!(xdata::read(&other), None);
}

#[test]
fn unit_scale_from_insunits_and_param() {
    let mut s = metric_session(Drawing::new_metric());
    for (insunits, mm) in [(1, 25.4), (4, 1.0), (5, 10.0), (6, 1000.0)] {
        s.doc_mut().unwrap().header.set_i64("INSUNITS", insunits);
        assert_eq!(unit_scale(&s, None).unwrap(), mm, "INSUNITS {insunits}");
    }
    for insunits in [0, 2, 14] {
        s.doc_mut().unwrap().header.set_i64("INSUNITS", insunits);
        let err = unit_scale(&s, None).unwrap_err().to_string();
        assert!(err.contains("units") && err.contains("mm, cm, m or in"), "{err}");
    }
    for (units, mm) in [("mm", 1.0), ("cm", 10.0), ("m", 1000.0), ("in", 25.4), (" M ", 1000.0)] {
        assert_eq!(unit_scale(&s, Some(units)).unwrap(), mm, "{units}");
    }
    // An explicit parameter wins over INSUNITS.
    s.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
    assert_eq!(unit_scale(&s, Some("m")).unwrap(), 1000.0);
    assert!(unit_scale(&s, Some("ft")).is_err());
}

const OVERLAY: &str = r#"{
  "schema": "ubot.catalog/1", "catalogId": "project-x", "version": "7",
  "products": [
    { "id": "ubot-x-600", "sku": "X600", "name": "UBOT X600", "widthMm": 600, "lengthMm": 600, "version": "1", "approvalStatus": "approved" },
    { "id": "ubot-h13-7", "sku": "H13+7", "name": "UBOT H13+7 (project)", "widthMm": 520, "lengthMm": 520, "version": "2", "approvalStatus": "approved" }
  ],
  "mappings": [], "rulepacks": []
}"#;

#[test]
fn ublib_list_load_reset() {
    let mut s = metric_session(Drawing::new_metric());
    let ids = |r: &Value| -> Vec<String> { r["products"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect() };
    let r = s.execute("ublib", &json!({})).unwrap();
    assert_eq!(ids(&r), ["ubot-h13-7", "ubot-h10-7", "ubot-h20-7", "ubot-h17-7"]);
    assert_eq!(r["source"], "builtin");
    assert_eq!(r["rulepacks"][0]["id"], "lpc-draft-1");
    assert_eq!(r["catalog"]["catalogId"], "lpc-builtin");

    let before = undo_len(&s);
    let r = s.execute("ublib", &json!({ "action": "load", "text": OVERLAY })).unwrap();
    assert_eq!(ids(&r), ["ubot-h13-7", "ubot-h10-7", "ubot-h20-7", "ubot-h17-7", "ubot-x-600"]);
    assert_eq!(r["source"], "text");
    assert_eq!(r["products"][0]["name"], "UBOT H13+7 (project)", "same id is replaced in place");
    assert_eq!(undo_len(&s), before, "UBLIB is not an undo step");
    assert!(s.ubot.catalog.product("ubot-x-600").is_some());

    // A broken catalog is refused and the loaded one stays.
    assert!(s.execute("ublib", &json!({ "action": "load", "text": "{ not json" })).is_err());
    assert!(s.execute("ublib", &json!({ "action": "load" })).is_err());
    assert!(s.execute("ublib", &json!({ "action": "explode" })).is_err());
    assert_eq!(ids(&s.execute("ublib", &json!({ "action": "list" })).unwrap()).len(), 5);

    let path = std::env::temp_dir().join(format!("cadcraft-ublib-{}.json", std::process::id()));
    std::fs::write(&path, OVERLAY).unwrap();
    let r = s.execute("ublib", &json!({ "action": "load", "path": path.to_string_lossy() }));
    let _ = std::fs::remove_file(&path);
    let r = r.unwrap();
    assert_eq!(r["source"], path.to_string_lossy().as_ref());
    assert_eq!(ids(&r).len(), 5);

    let r = s.execute("ublib", &json!({ "action": "reset" })).unwrap();
    assert_eq!(ids(&r).len(), 4);
    assert_eq!(r["source"], "builtin");
}

/// A metric drawing with a closed square, a circle, an open polyline and a line; returns their handles.
fn zone_drawing() -> (Session, [Handle; 4]) {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    let sq = d.add(&Space::Model, Common::default(), square(0.0, 0.0, 1000.0)).unwrap();
    let ci = d.add(&Space::Model, Common::default(), EntityKind::Circle(Circle { center: Vec3::new(3000.0, 500.0, 0.0), radius: 250.0 })).unwrap();
    let open = d.add(&Space::Model, Common::default(), poly(&[[0.0, 2000.0], [1000.0, 2000.0], [1000.0, 3000.0]], false)).unwrap();
    let line = d.add(&Space::Model, Common::default(), EntityKind::Line(Line { a: Vec3::new(0.0, 0.0, 0.0), b: Vec3::new(1.0, 1.0, 0.0) })).unwrap();
    (metric_session(d), [sq, ci, open, line])
}

#[test]
fn ubzone_tags_polyline_and_one_undo() {
    let (mut s, [sq, ci, _, _]) = zone_drawing();
    let before = undo_len(&s);
    let r = s.execute("ubzone", &json!({ "handles": [sq.hex(), ci.hex()], "type": "column", "clearanceMm": 50 })).unwrap();
    assert_eq!(undo_len(&s), before + 1, "one undo step for all zones");
    let tagged = r["zones"].as_array().unwrap();
    assert_eq!(tagged.len(), 2);
    for (z, h) in tagged.iter().zip([sq, ci]) {
        assert_eq!(z["handle"], h.hex());
        let id = z["zoneId"].as_str().unwrap();
        assert!(id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{id}");
        assert_eq!(
            read_tag(&s, h),
            Some(Ok(UbotTag::Zone { zone_id: id.into(), zone_type: ZoneType::Column, clearance_mm: Some(50.0), alt_product_id: None }))
        );
    }
    assert_ne!(tagged[0]["zoneId"], tagged[1]["zoneId"]);
    let found = zones(s.doc().unwrap());
    assert_eq!(found.len(), 2);
    let circle = &found.iter().find(|(h, _)| *h == ci).unwrap().1;
    assert_eq!(circle.ring.vertices, vec![[2750.0, 500.0, 1.0], [3250.0, 500.0, 1.0]], "a circle is two half arcs");
    assert!(cadcraft_ubot::ring_from_input(&circle.ring, 1.0, 0.5).is_ok());
    assert!(invalid_tags(s.doc().unwrap()).is_empty());

    // Retagging the same zone keeps its id and changes the type.
    s.execute("ubzone", &json!({ "handles": [sq.hex()], "type": "alt_product", "altProduct": "ubot-h10-7" })).unwrap();
    assert!(
        matches!(read_tag(&s, sq), Some(Ok(UbotTag::Zone { zone_type: ZoneType::AltProduct, clearance_mm: None, alt_product_id: Some(ref a), .. })) if a == "ubot-h10-7")
    );
    s.execute("u", &json!({})).unwrap();
    s.execute("u", &json!({})).unwrap();
    assert_eq!(read_tag(&s, sq), None, "undo takes the tags away");
    assert_eq!(read_tag(&s, ci), None);

    // Interactive: pick, type keyword, alternative product; one undo step.
    let before = undo_len(&s);
    s.start("ubzone").unwrap();
    s.input(Input::Pick(vec![sq])).unwrap();
    s.input(Input::Enter).unwrap();
    s.input(Input::Text("ap".into())).unwrap();
    s.input(Input::Text("ubot-h20-7".into())).unwrap();
    assert!(s.running.is_none());
    assert_eq!(undo_len(&s), before + 1);
    assert!(matches!(read_tag(&s, sq), Some(Ok(UbotTag::Zone { alt_product_id: Some(ref a), .. })) if a == "ubot-h20-7"));

    // Esc at the zone type prompt leaves the drawing as it was.
    s.set_selection(Vec::new());
    let doc = s.state().unwrap().doc.clone();
    let before = undo_len(&s);
    s.start("ubzone").unwrap();
    s.input(Input::Pick(vec![ci])).unwrap();
    s.input(Input::Enter).unwrap();
    assert!(s.current_prompt().unwrap().keywords.contains(&"AltProduct".to_string()));
    s.cancel();
    assert!(s.running.is_none());
    assert_eq!(*doc, *s.doc().unwrap());
    assert_eq!(undo_len(&s), before);
    assert_eq!(read_tag(&s, ci), None);
}

#[test]
fn ubzone_rejects_open_polyline() {
    let (mut s, [sq, ci, open, line]) = zone_drawing();
    let doc = s.state().unwrap().doc.clone();
    let before = undo_len(&s);
    let err = s.execute("ubzone", &json!({ "handles": [sq.hex(), open.hex()], "type": "exclusion" })).unwrap_err().to_string();
    assert!(err.contains("open polyline"), "{err}");
    let err = s.execute("ubzone", &json!({ "handles": [line.hex()], "type": "exclusion" })).unwrap_err().to_string();
    assert!(err.contains("not a closed polyline or circle"), "{err}");
    assert!(s.execute("ubzone", &json!({ "handles": [sq.hex()], "type": "pool" })).is_err());
    assert!(s.execute("ubzone", &json!({ "handles": [sq.hex()] })).is_err());
    assert!(s.execute("ubzone", &json!({ "handles": [sq.hex()], "type": "alt_product" })).is_err(), "alt_product needs a product");
    assert!(s.execute("ubzone", &json!({ "handles": [sq.hex()], "type": "alt_product", "altProduct": "nope" })).is_err());
    assert!(s.execute("ubzone", &json!({ "handles": [sq.hex()], "type": "wall", "altProduct": "ubot-h10-7" })).is_err());
    assert!(s.execute("ubzone", &json!({ "handles": [ci.hex()], "type": "wall", "clearanceMm": -1 })).is_err());
    assert!(s.execute("ubzone", &json!({ "handles": [ci.hex()], "type": "wall", "clearanceMm": 1e9 })).is_err());
    assert!(s.execute("ubzone", &json!({ "handles": [], "type": "wall" })).is_err());
    assert!(Arc::ptr_eq(&doc, &s.state().unwrap().doc), "nothing was written");
    assert_eq!(undo_len(&s), before);
}

#[test]
fn ubzone_refuses_layout_box_and_malformed_records() {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    let (c, k) = tagged(square(0.0, 0.0, 6600.0), Some(&layout_tag()));
    let boundary = d.add(&Space::Model, c, k).unwrap();
    let (c, k) = tagged(square(8000.0, 0.0, 520.0), Some(&instance_tag()));
    let boxed = d.add(&Space::Model, c, k).unwrap();
    let broken = Common { xdata: vec![tag(1001, APP), tag(1000, "ubot/1"), tag(1000, "Q")], ..Common::default() };
    let malformed = d.add(&Space::Model, broken.clone(), square(9000.0, 0.0, 400.0)).unwrap();
    let plain = d.add(&Space::Model, Common::default(), square(10000.0, 0.0, 400.0)).unwrap();
    let mut s = metric_session(d);
    let doc = s.state().unwrap().doc.clone();
    let before = undo_len(&s);

    let err = s.execute("ubzone", &json!({ "handles": [plain.hex(), boundary.hex()], "type": "exclusion" })).unwrap_err().to_string();
    assert!(err.contains(&boundary.hex()) && err.contains("layout"), "{err}");
    let err = s.execute("ubzone", &json!({ "handles": [boxed.hex()], "type": "column" })).unwrap_err().to_string();
    assert!(err.contains(&boxed.hex()) && err.contains("box"), "{err}");
    let err = s.execute("ubzone", &json!({ "handles": [malformed.hex()], "type": "column" })).unwrap_err().to_string();
    assert!(err.contains(&malformed.hex()) && err.contains("unreadable"), "{err}");
    assert!(Arc::ptr_eq(&doc, &s.state().unwrap().doc), "nothing was written");
    assert_eq!(undo_len(&s), before);
    assert_eq!(read_tag(&s, boundary), Some(Ok(layout_tag())));
    assert_eq!(s.doc().unwrap().entity(malformed).unwrap().common.xdata, broken.xdata);
    assert_eq!(read_tag(&s, plain), None);

    // An identical re-tag is a no-op: no undo step, same drawing; a changed one writes again.
    let first = s.execute("ubzone", &json!({ "handles": [plain.hex()], "type": "column", "clearanceMm": 12.5 })).unwrap();
    assert_eq!(undo_len(&s), before + 1);
    let doc = s.state().unwrap().doc.clone();
    let again = s.execute("ubzone", &json!({ "handles": [plain.hex()], "type": "column", "clearanceMm": 12.5 })).unwrap();
    assert_eq!(again, first, "the same zones are reported");
    assert!(Arc::ptr_eq(&doc, &s.state().unwrap().doc), "an identical re-tag writes nothing");
    assert_eq!(undo_len(&s), before + 1);
    s.execute("ubzone", &json!({ "handles": [plain.hex()], "type": "wall", "clearanceMm": 12.5 })).unwrap();
    assert_eq!(undo_len(&s), before + 2);
    assert!(matches!(read_tag(&s, plain), Some(Ok(UbotTag::Zone { zone_type: ZoneType::Wall, .. }))));
}

#[test]
fn ubot_xdata_survives_dxf_round_trip() {
    set_io(IoHooks {
        read: |bytes, _| cadcraft_io::read_dxf(bytes).map_err(|e| e.to_string()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: None,
    });
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 4);
    add_block(&mut d, "UBOT_H13_7", vec![(Common::default(), square(-260.0, -260.0, 520.0))]);
    let mut box_common = Common { xdata: vec![tag(1001, "ACME"), tag(1000, "keep me"), tag(1070, "7")], ..Common::default() };
    xdata::write(&mut box_common, &instance_tag()).unwrap();
    let boxed = d.add(&Space::Model, box_common, insert("UBOT_H13_7", [500.0, 500.0], 1, 1)).unwrap();
    let (c, k) = tagged(square(0.0, 0.0, 6600.0), Some(&layout_tag()));
    let boundary = d.add(&Space::Model, c, k).unwrap();
    let zone = d.add(&Space::Model, Common::default(), square(2000.0, 2000.0, 400.0)).unwrap();
    let circle =
        d.add(&Space::Model, Common::default(), EntityKind::Circle(Circle { center: Vec3::new(4000.0, 4000.0, 0.0), radius: 200.0 })).unwrap();
    let mut s = metric_session(d);
    s.execute("ubzone", &json!({ "handles": [zone.hex()], "type": "alt_product", "altProduct": "ubot-h10-7", "clearanceMm": 12.75 })).unwrap();
    s.execute("ubzone", &json!({ "handles": [circle.hex()], "type": "column" })).unwrap();
    let want: Vec<(Handle, UbotTag)> = [boxed, boundary, zone, circle].iter().map(|h| (*h, read_tag(&s, *h).unwrap().unwrap())).collect();
    let acme: Vec<RawTag> = s.doc().unwrap().entity(boxed).unwrap().common.xdata.iter().take(3).cloned().collect();

    let data = s.execute("document.bytes", &json!({ "format": "dxf" })).unwrap()["data"].as_str().unwrap().to_string();
    let text = String::from_utf8(cadcraft_engine::cmd::file::base64_decode(&data).unwrap()).unwrap();
    let groups: Vec<&str> = text.lines().map(str::trim).collect();
    // Group pairs of each APPID record: its name (group 2) must be LPC_UBOT for one of them.
    let pairs: Vec<(&str, &str)> = groups.chunks(2).filter_map(|p| Some((*p.first()?, *p.get(1)?))).collect();
    let appid = pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| **p == ("0", "APPID"))
        .any(|(i, _)| pairs.iter().skip(i + 1).take_while(|p| p.0 != "0").any(|p| *p == ("2", APP)));
    assert!(appid, "the APPID table registers LPC_UBOT");
    assert!(groups.windows(2).any(|w| w == ["1070", "3"]), "count multiplier is saved");
    s.execute("open", &json!({ "data": data, "name": "roundtrip.dxf" })).unwrap();
    let back = s.doc().unwrap();
    for (h, t) in &want {
        let e = back.entity(*h).unwrap_or_else(|| panic!("{} kept its handle", h.hex()));
        assert_eq!(xdata::read(&e.common), Some(Ok(t.clone())), "{}", h.hex());
    }
    let kept: Vec<(i32, String)> = back.entity(boxed).unwrap().common.xdata.iter().take(3).map(|t| (t.code, t.value.trim().to_string())).collect();
    assert_eq!(kept, acme.iter().map(|t| (t.code, t.value.clone())).collect::<Vec<_>>(), "other applications' data stays first");
    assert_eq!(zones(back).len(), 2);
    let inst = existing_instances(back, &Catalog::builtin());
    assert_eq!(inst.len(), 1);
    assert_eq!(inst[0].handle, boxed.hex());
    assert_eq!(inst[0].instance_id.as_deref(), Some("0123456789abcdef:3:4"));
    assert_eq!(inst[0].insert, [500.0, 500.0]);
    assert!(matches!(xdata::read(&back.entity(boxed).unwrap().common), Some(Ok(UbotTag::Instance { count_multiplier: 3, .. }))));
}

#[test]
fn count_items_nested_cluster_and_minsert() {
    let catalog = Catalog::builtin();
    let boxed =
        |product: &str| UbotTag::Instance { instance_id: "x:0:0".into(), product_id: product.into(), layout_id: "x".into(), count_multiplier: 1 };
    let mut d = Drawing::new_metric();
    add_block(&mut d, "UBOT_H13_7", vec![(Common::default(), square(-260.0, -260.0, 520.0))]);
    add_block(&mut d, "UB-1", vec![(Common::default(), square(-260.0, -520.0, 520.0))]);
    add_block(&mut d, "DOOR", vec![(Common::default(), square(0.0, 0.0, 900.0))]);
    // A 2×2 cluster of tagged boxes, and a cluster of two such clusters.
    add_block(
        &mut d,
        "CLUSTER",
        [[0.0, 0.0], [700.0, 0.0], [0.0, 700.0], [700.0, 700.0]]
            .iter()
            .map(|p| tagged(insert("UBOT_H13_7", *p, 1, 1), Some(&boxed("ubot-h13-7"))))
            .collect(),
    );
    add_block(&mut d, "CLUSTER2", vec![tagged(insert("CLUSTER", [0.0, 0.0], 1, 1), None), tagged(insert("CLUSTER", [2000.0, 0.0], 1, 1), None)]);
    // Hand-drawn legacy boxes inside a cluster, and two blocks that reference each other.
    add_block(&mut d, "LEGACY3", (0..3).map(|i| tagged(insert("UB-1", [700.0 * f64::from(i), 0.0], 1, 1), None)).collect());
    add_block(&mut d, "LOOP_A", vec![tagged(insert("LOOP_B", [0.0, 0.0], 1, 1), None), tagged(insert("UB-1", [0.0, 0.0], 1, 1), None)]);
    add_block(&mut d, "LOOP_B", vec![tagged(insert("LOOP_A", [0.0, 0.0], 1, 1), None)]);
    // A cluster holding a tagged MINSERT row of three boxes.
    add_block(&mut d, "ROW3", vec![tagged(insert("UBOT_H13_7", [0.0, 0.0], 3, 1), Some(&boxed("ubot-h20-7")))]);
    let five = UbotTag::Instance { instance_id: "x:9:9".into(), product_id: "ubot-h17-7".into(), layout_id: "x".into(), count_multiplier: 5 };
    let model = vec![
        tagged(insert("CLUSTER", [0.0, 0.0], 1, 1), None),                             // 4
        tagged(insert("CLUSTER", [5000.0, 0.0], 1, 1), None),                          // 4
        tagged(insert("CLUSTER2", [0.0, 5000.0], 1, 1), None),                         // 8
        tagged(insert("UBOT_H13_7", [0.0, 9000.0], 2, 3), Some(&boxed("ubot-h10-7"))), // MINSERT 2×3 = 6
        tagged(insert("CLUSTER", [9000.0, 0.0], 2, 1), None),                          // MINSERT of a cluster: 8
        tagged(insert("UB-1", [0.0, -2000.0], 1, 1), None),                            // legacy 1
        tagged(insert("LEGACY3", [0.0, -4000.0], 1, 2), None),                         // legacy 3 × 2 = 6
        tagged(insert("DOOR", [0.0, -6000.0], 1, 1), None),                            // not a box
        tagged(insert("LOOP_A", [0.0, -8000.0], 1, 1), None),                          // legacy 1 inside a cycle
        tagged(insert("MISSING", [0.0, -9000.0], 1, 1), None),
        tagged(insert("ROW3", [0.0, -10000.0], 2, 1), None), // MINSERT 3×1 inside a cluster inserted 2×1: 6
        tagged(insert("UBOT_H13_7", [0.0, -12000.0], 1, 2), Some(&five)), // count multiplier 5 × MINSERT 1×2: 10                         // no such block
    ];
    for (common, kind) in model {
        d.add(&Space::Model, common, kind).unwrap();
    }
    let items = count_items(&d, &catalog);
    assert!(items.iter().all(|i| !i.block_name.starts_with("CLUSTER") && !i.block_name.starts_with("LEGACY") && i.block_name != "DOOR"), "{items:?}");
    let sum = |source: CountSource, product: Option<&str>| -> u64 {
        items.iter().filter(|i| i.source == source && i.product_id.as_deref() == product).map(|i| i.multiplier).sum()
    };
    assert_eq!(sum(CountSource::Tagged, Some("ubot-h13-7")), 4 + 4 + 8 + 8);
    assert_eq!(sum(CountSource::Tagged, Some("ubot-h10-7")), 6);
    assert_eq!(sum(CountSource::Legacy, None), 1 + 6 + 1);
    assert_eq!(sum(CountSource::Tagged, Some("ubot-h20-7")), 6);
    assert_eq!(sum(CountSource::Tagged, Some("ubot-h17-7")), 10);
    let counts = cadcraft_ubot::count(&items, &catalog);
    assert_eq!(counts.total, 24 + 6 + 8 + 6 + 10);

    // Only model-space box references are existing instances: the tagged MINSERT and the bare UB-1.
    let existing = existing_instances(&d, &catalog);
    assert_eq!(existing.iter().map(|e| e.block_name.as_str()).collect::<Vec<_>>(), ["UBOT_H13_7", "UB-1", "UBOT_H13_7"]);
    assert_eq!(existing[1].product_id, None, "legacy boxes leave the product to the mapping");
}

#[test]
fn write_helpers_make_layer_block_and_commit_atomically() {
    use cadcraft_engine::cmd::ubot::write::{LAYER, commit, ensure_layer_ubot, ensure_product_block};
    let catalog = Catalog::builtin();
    let product = catalog.product("ubot-h13-7").unwrap();
    let mut d = Drawing::new_metric();
    ensure_layer_ubot(&mut d);
    assert_eq!(d.layer(LAYER).unwrap().color, cadcraft_engine::doc::color::Color::from_aci(55));
    let name = Catalog::default_block_name(product);
    assert_eq!(name, "UBOT_H13_7");
    // Box blocks are drawn in mm (one block unit = 1 mm) whatever the drawing units, origin at the centre.
    ensure_product_block(&mut d, product, &name).unwrap();
    let b = d.block(&name).unwrap().clone();
    let EntityKind::LwPolyline(pl) = &b.entities.iter().next().unwrap().kind else { panic!("outline") };
    assert!(pl.closed);
    assert_eq!(pl.vertices.iter().map(|v| [v.p.x, v.p.y]).collect::<Vec<_>>(), [[-260.0, -260.0], [260.0, -260.0], [260.0, 260.0], [-260.0, 260.0]]);
    ensure_product_block(&mut d, product, &name).unwrap();
    assert!(Arc::ptr_eq(&b, d.block(&name).unwrap()), "an existing block is kept");
    assert!(ensure_product_block(&mut d, product, "  ").is_err());

    let mut s = metric_session(d);
    let doc = s.state().unwrap().doc.clone();
    let r = commit(&mut s, |d| {
        ensure_layer_ubot(d);
        d.add(&Space::Model, Common::default(), square(0.0, 0.0, 10.0))?;
        Err(cadcraft_engine::EngineError::Other("stop".into()))
    });
    assert!(r.is_err());
    assert!(Arc::ptr_eq(&doc, &s.state().unwrap().doc), "a failed edit publishes nothing");
    let r = commit(&mut s, |d| {
        d.add(&Space::Model, Common::default(), square(0.0, 0.0, 10.0))?;
        Ok(json!({ "added": 1 }))
    })
    .unwrap();
    assert_eq!(r["added"], 1);
    assert_eq!(s.doc().unwrap().model.len(), doc.model.len() + 1);
}

#[test]
fn box_block_is_mm_and_scaled_by_the_inverse_unit_scale() {
    use cadcraft_engine::cmd::ubot::write::ensure_product_block;
    let catalog = Catalog::builtin();
    let product = catalog.product("ubot-h13-7").unwrap();
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", 5);
    ensure_product_block(&mut d, product, "UBOT_H13_7").unwrap();
    let s = metric_session(d);
    let mm_per_unit = unit_scale(&s, None).unwrap();
    assert_eq!(mm_per_unit, 10.0);
    let d = s.doc().unwrap();
    let block = d.block("UBOT_H13_7").unwrap();
    let EntityKind::LwPolyline(pl) = &block.entities.iter().next().unwrap().kind else { panic!("outline") };
    let xs: Vec<f64> = pl.vertices.iter().map(|v| v.p.x).collect();
    assert_eq!(xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min), 520.0, "block square is 520 units (mm)");
    // An insert at scale 1/unit_scale covers the true 520 mm = 52 cm of the drawing.
    let EntityKind::Insert(mut ins) = insert("UBOT_H13_7", [1000.0, 1000.0], 1, 1) else { unreachable!() };
    ins.scale = Vec3::new(1.0 / mm_per_unit, 1.0 / mm_per_unit, 1.0);
    let m = ins.transform(block.base.xy());
    let world: Vec<Vec2> = pl.vertices.iter().map(|v| m.apply(v.p)).collect();
    let (lo, hi) = world.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    assert!((hi - lo - 52.0).abs() < 1e-9, "{lo}..{hi}");
    assert!((world[0].x - 974.0).abs() < 1e-9 && (world[0].y - 974.0).abs() < 1e-9, "{:?}", world[0]);
}

#[test]
fn xdata_write_rejects_long_strings_and_revision_overflow() {
    let other = vec![tag(1001, "ACME"), tag(1000, "a")];
    let mut c = Common { xdata: other.clone(), ..Common::default() };
    xdata::write(&mut c, &instance_tag()).unwrap();
    let before = c.xdata.clone();
    let long = |n: usize| UbotTag::Instance { instance_id: "i".into(), product_id: "p".repeat(n), layout_id: "l".into(), count_multiplier: 1 };
    assert!(xdata::write(&mut c, &long(256)).is_err(), "a 256-byte string does not fit a 1000 group");
    assert_eq!(c.xdata, before, "a refused write changes nothing");
    // Bytes, not characters: 128 two-byte letters are 256 bytes.
    let wide = UbotTag::Zone { zone_id: "đ".repeat(128), zone_type: ZoneType::Wall, clearance_mm: None, alt_product_id: None };
    assert!(xdata::write(&mut c, &wide).is_err());
    xdata::write(&mut c, &long(255)).unwrap();
    assert_eq!(xdata::read(&c), Some(Ok(long(255))));

    let UbotTag::Layout { mut record, boundary_handle } = layout_tag() else { unreachable!() };
    record.revision = i32::MAX as u32 + 1;
    let too_new = UbotTag::Layout { record: record.clone(), boundary_handle: boundary_handle.clone() };
    assert!(xdata::write(&mut c, &too_new).is_err(), "1071 is a 32-bit signed group");
    record.revision = i32::MAX as u32;
    xdata::write(&mut c, &UbotTag::Layout { record, boundary_handle }).unwrap();

    // The reader refuses the same values from a file.
    let record = |fields: Vec<RawTag>| Common { xdata: std::iter::once(tag(1001, APP)).chain(fields).collect(), ..Common::default() };
    let long_id = "x".repeat(256);
    let i = vec![tag(1000, "ubot/1"), tag(1000, "I"), tag(1000, &long_id), tag(1000, "p"), tag(1000, "l"), tag(1070, "1")];
    assert!(matches!(xdata::read(&record(i)), Some(Err(_))));
    let mut c = Common::default();
    xdata::write(&mut c, &layout_tag()).unwrap();
    if let Some(rev) = c.xdata.iter_mut().find(|t| t.code == 1071) {
        rev.value = "2147483648".into();
    }
    assert!(matches!(xdata::read(&c), Some(Err(_))));
}
