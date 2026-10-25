//! UBREPLACE and TUBAB: swap U-BOT box blocks keeping centre, rotation, layer, attributes and the instance tag.

use std::sync::Arc;

use cadcraft_engine::Input;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::ubot::read::{count_items, existing_instances};
use cadcraft_engine::cmd::ubot::xdata::{self, UbotTag};
use cadcraft_engine::doc::color::Color;
use cadcraft_engine::doc::{Attrib, Block, Common, Drawing, Entity, EntityKind, Handle, Insert, Line, LwPolyline, Space, Text};
use cadcraft_engine::geom::{PolyVertex, Vec2, Vec3};
use cadcraft_ubot::{Catalog, CountSource};
use serde_json::{Value, json};

const LAYOUT: &str = "0123456789abcdef";

fn text(value: &str) -> Text {
    Text {
        insert: Vec3::ZERO,
        align_pt: None,
        height: 50.0,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: Default::default(),
        valign: Default::default(),
    }
}

fn attdef(tag: &str) -> EntityKind {
    EntityKind::AttDef(Attrib {
        tag: tag.into(),
        text: text("?"),
        invisible: false,
        constant: false,
        prompt: tag.into(),
        props: Default::default(),
        multiline: None,
        modes: Default::default(),
    })
}

fn square(x: f64, y: f64, w: f64, l: f64) -> EntityKind {
    let vertices = [[x, y], [x + w, y], [x + w, y + l], [x, y + l]]
        .iter()
        .map(|p| PolyVertex { p: Vec2::new(p[0], p[1]), bulge: 0.0, start_width: 0.0, end_width: 0.0 })
        .collect();
    EntityKind::LwPolyline(LwPolyline { vertices, closed: true, const_width: 0.0, elevation: 0.0, plinegen: false })
}

/// A block whose geometry is a `w`×`l` square at `origin` (lower-left corner), with optional attribute definitions.
fn add_block(d: &mut Drawing, name: &str, origin: [f64; 2], size: [f64; 2], attdefs: &[&str]) {
    let mut b = Block::new(name);
    let h = d.new_handle();
    b.entities.push(Entity { handle: h, common: Common::default(), kind: square(origin[0], origin[1], size[0], size[1]) });
    for tag in attdefs {
        let h = d.new_handle();
        b.entities.push(Entity { handle: h, common: Common::default(), kind: attdef(tag) });
    }
    d.blocks.insert(name.into(), Arc::new(b));
}

/// UB-1..UB-3 are LPC legacy blocks: top-centre insertion point, 520 × 520.
fn legacy_block(d: &mut Drawing, name: &str, attdefs: &[&str]) {
    add_block(d, name, [-260.0, -520.0], [520.0, 520.0], attdefs);
}

fn generated_block(d: &mut Drawing, name: &str, attdefs: &[&str]) {
    add_block(d, name, [-260.0, -260.0], [520.0, 520.0], attdefs);
}

fn instance(product: &str, i: u32, count_multiplier: i16) -> UbotTag {
    UbotTag::Instance { instance_id: format!("{LAYOUT}:{i}:0"), product_id: product.into(), layout_id: LAYOUT.into(), count_multiplier }
}

struct BoxSpec<'a> {
    block: &'a str,
    at: [f64; 2],
    rotation_deg: f64,
    scale: f64,
    layer: &'a str,
    tag: Option<UbotTag>,
    attribs: &'a [(&'a str, &'a str)],
}

impl Default for BoxSpec<'_> {
    fn default() -> Self {
        BoxSpec { block: "UBOT_H13_7", at: [0.0, 0.0], rotation_deg: 0.0, scale: 1.0, layer: "0", tag: None, attribs: &[] }
    }
}

fn add_box(d: &mut Drawing, b: BoxSpec) -> Handle {
    let mut common = Common { layer: b.layer.into(), ..Common::default() };
    if let Some(t) = &b.tag {
        xdata::write(&mut common, t).unwrap();
    }
    let attribs = b
        .attribs
        .iter()
        .map(|(tag, value)| Attrib {
            tag: (*tag).into(),
            text: text(value),
            invisible: false,
            constant: false,
            prompt: String::new(),
            props: Default::default(),
            multiline: None,
            modes: Default::default(),
        })
        .collect();
    let ins = Insert {
        block: b.block.into(),
        insert: Vec3::new(b.at[0], b.at[1], 0.0),
        scale: Vec3::new(b.scale, b.scale, b.scale),
        rotation: b.rotation_deg.to_radians(),
        attribs,
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
    };
    d.add(&Space::Model, common, EntityKind::Insert(ins)).unwrap()
}

fn drawing(insunits: i64) -> Drawing {
    let mut d = Drawing::new_metric();
    d.header.set_i64("INSUNITS", insunits);
    d
}

fn session(d: Drawing) -> Session {
    let mut s = Session::empty();
    s.open_drawing(d, "replace.dxf", None);
    s
}

fn undo_len(s: &Session) -> usize {
    s.state().unwrap().undo.len()
}

fn doc(s: &Session) -> Arc<Drawing> {
    s.state().unwrap().doc.clone()
}

fn insert_of(s: &Session, h: Handle) -> Insert {
    match &s.doc().unwrap().entity(h).unwrap().kind {
        EntityKind::Insert(i) => i.clone(),
        other => panic!("not an insert: {other:?}"),
    }
}

fn common_of(s: &Session, h: Handle) -> Common {
    s.doc().unwrap().entity(h).unwrap().common.clone()
}

fn tag_of(s: &Session, h: Handle) -> Option<UbotTag> {
    xdata::read(&common_of(s, h)).map(|t| t.unwrap())
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn assert_xy(v: Vec3, want: [f64; 2]) {
    assert!(close(v.x, want[0]) && close(v.y, want[1]), "{v:?} != {want:?}");
}

fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, &p).unwrap_err().to_string()
}

/// Overlays the session catalog with these products and mappings (UBLIB load).
fn overlay(s: &mut Session, products: Value, mappings: Value) {
    let text =
        json!({ "schema": "ubot.catalog/1", "catalogId": "test", "version": "1", "products": products, "mappings": mappings, "rulepacks": [] });
    s.execute("ublib", &json!({ "action": "load", "text": text.to_string() })).unwrap();
}

fn product(id: &str, sku: &str, size: f64) -> Value {
    json!({ "id": id, "sku": sku, "name": id, "widthMm": size, "lengthMm": size, "version": "1", "approvalStatus": "draft" })
}

fn mapping(block: &str, product: Option<&str>, anchor: &str) -> Value {
    json!({ "productId": product, "cadEngine": "any", "blockName": block, "unitsMmPerBlockUnit": 1.0, "anchor": anchor, "rotationOffsetDeg": 0.0 })
}

fn rot(v: [f64; 2], deg: f64) -> [f64; 2] {
    let (s, c) = deg.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

// ---------------- UBREPLACE ----------------

#[test]
fn ubreplace_keeps_center_rotation_layer_attribs() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &["MARK"]);
    // The target block already exists with the same attribute, so ATTSYNC-style matching keeps the value.
    generated_block(&mut d, "UBOT_H10_7", &["MARK", "NOTE"]);
    legacy_block(&mut d, "UB-1", &[]);
    d.ensure_layer("SLAB-A");
    let tagged = add_box(
        &mut d,
        BoxSpec {
            at: [1000.0, 2000.0],
            rotation_deg: 30.0,
            layer: "SLAB-A",
            tag: Some(instance("ubot-h13-7", 1, 2)),
            attribs: &[("MARK", "A1")],
            ..BoxSpec::default()
        },
    );
    d.modify_entity(tagged, |e| e.common.color = Color::from_aci(1)).unwrap();
    // A legacy UB-1 (top centre) whose box centre is (5000, 5000).
    let legacy = add_box(&mut d, BoxSpec { block: "UB-1", at: [5000.0, 5260.0], ..BoxSpec::default() });
    let mut s = session(d);
    let before = doc(&s);
    let handles = json!([tagged.hex(), legacy.hex()]);

    let preview = s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h10-7" })).unwrap();
    assert!(Arc::ptr_eq(&doc(&s), &before), "preview writes nothing");
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["summary"], json!({ "targets": 2, "replaced": 2, "unchanged": 0, "refused": 0 }));
    assert_eq!(preview["changes"].as_array().unwrap().len(), 2);
    let hash = preview["inputHash"].as_str().unwrap().to_string();
    assert_eq!(hash.len(), 64);
    assert!(preview["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with("LEGACY_BLOCK")), "{}", preview["warnings"]);

    let r = s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h10-7", "mode": "commit", "expectHash": hash })).unwrap();
    assert_eq!(r["written"], json!({ "replaced": 2 }));
    let ins = insert_of(&s, tagged);
    assert_eq!(ins.block, "UBOT_H10_7");
    assert_xy(ins.insert, [1000.0, 2000.0]);
    assert!(close(ins.rotation, 30f64.to_radians()));
    assert_eq!(ins.scale, Vec3::new(1.0, 1.0, 1.0));
    let common = common_of(&s, tagged);
    assert_eq!(common.layer, "SLAB-A");
    assert_eq!(common.color, Color::from_aci(1));
    let marks: Vec<(String, String)> = ins.attribs.iter().map(|a| (a.tag.clone(), a.text.value.clone())).collect();
    assert_eq!(marks, [("MARK".to_string(), "A1".to_string()), ("NOTE".to_string(), "?".to_string())], "values kept by tag, new tags get defaults");
    assert_eq!(
        tag_of(&s, tagged),
        Some(UbotTag::Instance {
            instance_id: format!("{LAYOUT}:1:0"),
            product_id: "ubot-h10-7".into(),
            layout_id: LAYOUT.into(),
            count_multiplier: 2
        })
    );
    // The legacy box keeps its centre on the centred block and stays untagged (its block now names the product).
    let ins = insert_of(&s, legacy);
    assert_eq!(ins.block, "UBOT_H10_7");
    assert_xy(ins.insert, [5000.0, 5000.0]);
    assert_eq!(tag_of(&s, legacy), None);
    let found = existing_instances(s.doc().unwrap(), &s.ubot.catalog);
    let legacy_now = found.iter().find(|e| e.handle == legacy.hex()).unwrap();
    assert_eq!(legacy_now.product_id.as_deref(), Some("ubot-h10-7"));
}

#[test]
fn ubreplace_draws_the_new_box_at_true_size() {
    // R19/R13: the new INSERT scale is the core's (1/unit_scale for UBOT_ blocks); a missing block is generated.
    let mut d = drawing(5);
    generated_block(&mut d, "UBOT_H13_7", &["MARK"]);
    let cm = add_box(&mut d, BoxSpec { at: [100.0, 200.0], rotation_deg: 45.0, scale: 0.1, attribs: &[("MARK", "Z")], ..BoxSpec::default() });
    let big = add_box(&mut d, BoxSpec { at: [500.0, 500.0], scale: 0.2, ..BoxSpec::default() });
    let mut s = session(d);
    s.execute("ubreplace", &json!({ "handles": [cm.hex(), big.hex()], "to": "ubot-h20-7", "mode": "commit" })).unwrap();
    let block = s.doc().unwrap().block("UBOT_H20_7").cloned().expect("generated block");
    assert_eq!(block.units, 4);
    for h in [cm, big] {
        let ins = insert_of(&s, h);
        assert_eq!(ins.block, "UBOT_H20_7");
        assert!(close(ins.scale.x, 0.1) && close(ins.scale.y, 0.1), "{:?}", ins.scale);
        assert!(ins.attribs.is_empty(), "the generated block defines no attributes");
    }
    assert_xy(insert_of(&s, cm).insert, [100.0, 200.0]);
    assert_xy(insert_of(&s, big).insert, [500.0, 500.0]);
    assert!(close(insert_of(&s, cm).rotation, 45f64.to_radians()));
}

#[test]
fn ubreplace_refuses_size_change() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    let a = add_box(&mut d, BoxSpec { tag: Some(instance("ubot-h13-7", 0, 1)), ..BoxSpec::default() });
    let b = add_box(&mut d, BoxSpec { at: [700.0, 0.0], tag: Some(instance("ubot-h13-7", 1, 1)), ..BoxSpec::default() });
    let mut s = session(d);
    overlay(&mut s, json!([product("ubot-big", "BIG", 600.0)]), json!([]));
    let before = doc(&s);
    let undo = undo_len(&s);
    let p = json!({ "handles": [a.hex(), b.hex()], "to": "ubot-big" });
    let preview = s.execute("ubreplace", &p).unwrap();
    assert_eq!(preview["summary"], json!({ "targets": 2, "replaced": 0, "unchanged": 0, "refused": 2 }));
    let refused = preview["refused"].as_array().unwrap();
    assert_eq!(refused[0]["handle"], a.hex());
    assert!(refused[0]["message"].as_str().unwrap().contains("size differs"), "{}", refused[0]);
    let mut commit = p.clone();
    commit["mode"] = json!("commit");
    let e = err(&mut s, "ubreplace", commit.clone());
    assert!(e.contains("refused") && e.contains("size differs") && e.contains("allowResize"), "{e}");
    assert!(Arc::ptr_eq(&doc(&s), &before));
    assert_eq!(undo_len(&s), undo);

    commit["allowResize"] = json!(true);
    let r = s.execute("ubreplace", &commit).unwrap();
    assert_eq!(r["written"], json!({ "replaced": 2 }));
    assert_eq!(insert_of(&s, a).block, "UBOT_BIG");
    assert_eq!(undo_len(&s), undo + 1);
}

#[test]
fn ubreplace_one_undo() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    let hs: Vec<Handle> = (0..3)
        .map(|i| add_box(&mut d, BoxSpec { at: [700.0 * f64::from(i), 0.0], tag: Some(instance("ubot-h13-7", i, 1)), ..BoxSpec::default() }))
        .collect();
    let mut s = session(d);
    let handles: Vec<String> = hs.iter().map(|h| h.hex()).collect();
    let before = doc(&s);
    let undo = undo_len(&s);
    let r = s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h10-7", "mode": "commit" })).unwrap();
    assert_eq!(r["written"], json!({ "replaced": 3 }));
    assert_eq!(undo_len(&s), undo + 1, "one undo step for all boxes");
    s.execute("u", &json!({})).unwrap();
    assert_eq!(*doc(&s), *before, "undo restores every box");

    // The same swap twice: the second run changes nothing and adds no undo step.
    s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h10-7", "mode": "commit" })).unwrap();
    let after = doc(&s);
    let undo = undo_len(&s);
    let r = s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h10-7", "mode": "commit" })).unwrap();
    assert_eq!(r["summary"]["unchanged"], 3);
    assert!(r.get("written").is_none(), "{r}");
    assert!(s.log.iter().any(|l| l == "UBREPLACE: nothing to change."));
    assert!(Arc::ptr_eq(&doc(&s), &after));
    assert_eq!(undo_len(&s), undo);

    // A drawing change between preview and commit is a conflict.
    let preview = s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h20-7" })).unwrap();
    s.execute("move", &json!({ "handles": [handles[0]], "delta": [10.0, 0.0] })).unwrap();
    let moved = doc(&s);
    let e = err(&mut s, "ubreplace", json!({ "handles": handles, "to": "ubot-h20-7", "mode": "commit", "expectHash": preview["inputHash"] }));
    assert!(e.contains("CONFLICT"), "{e}");
    assert!(Arc::ptr_eq(&doc(&s), &moved));
    let preview = s.execute("ubreplace", &json!({ "handles": handles, "to": "ubot-h20-7", "expectHash": "x" })).unwrap();
    assert!(preview["warnings"].as_array().unwrap().iter().any(|w| w == "expectHash ignored in preview"));
}

#[test]
fn ubreplace_interactive_select_replace_apply() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    let a = add_box(&mut d, BoxSpec { tag: Some(instance("ubot-h13-7", 0, 1)), ..BoxSpec::default() });
    let line = d.add(&Space::Model, Common::default(), EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(1.0, 0.0, 0.0) })).unwrap();
    let mut s = session(d);
    let before = doc(&s);
    let undo = undo_len(&s);
    s.start("ubreplace").unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Select boxes");
    s.input(Input::Pick(vec![a, line])).unwrap();
    assert!(s.log.iter().any(|l| l == "1 object(s) ignored (not U-BOT boxes)"), "{:?}", s.log.iter().rev().take(3).collect::<Vec<_>>());
    s.input(Input::Enter).unwrap();
    let p = s.current_prompt().unwrap();
    assert_eq!(p.message, "Replace with");
    assert!(p.keywords.contains(&"ubot-h10-7".to_string()), "{:?}", p.keywords);
    s.input(Input::Text("?".into())).unwrap();
    assert!(s.log.iter().any(|l| l.contains("ubot-h20-7")), "? lists the products");
    s.input(Input::Text("nope".into())).unwrap();
    assert_eq!(s.current_prompt().unwrap().message, "Replace with", "an unknown product asks again");
    s.input(Input::Keyword("ubot-h10-7".into())).unwrap();
    assert!(
        s.log.iter().any(|l| l == "UBREPLACE: 1 boxes to replace, 0 unchanged, 0 refused"),
        "{:?}",
        s.log.iter().rev().take(3).collect::<Vec<_>>()
    );
    assert_eq!(s.current_prompt().unwrap().display(), "Apply? [Yes/No] <Yes>:");
    s.input(Input::Cancel).unwrap();
    assert!(s.running.is_none());
    assert_eq!(*doc(&s), *before);
    assert_eq!(undo_len(&s), undo);

    s.set_selection(Vec::new());
    s.start("ubreplace").unwrap();
    s.input(Input::Pick(vec![a])).unwrap();
    s.input(Input::Enter).unwrap();
    s.input(Input::Keyword("ubot-h10-7".into())).unwrap();
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(insert_of(&s, a).block, "UBOT_H10_7");
}

// ---------------- TUBAB ----------------

#[test]
fn tubab_filters_by_from_and_keeps_transform() {
    let mut d = drawing(4);
    legacy_block(&mut d, "UB-1", &["MARK"]);
    legacy_block(&mut d, "UB-2", &["MARK"]);
    legacy_block(&mut d, "UB-3", &[]);
    d.ensure_layer("OTHER");
    // What the old VBA TUBxy lost: rotation, scale, layer and attributes.
    let ub1 = add_box(
        &mut d,
        BoxSpec {
            block: "UB-1",
            at: [1000.0, 2000.0],
            rotation_deg: 30.0,
            scale: 2.0,
            layer: "OTHER",
            attribs: &[("MARK", "X")],
            ..BoxSpec::default()
        },
    );
    let ub3 = add_box(&mut d, BoxSpec { block: "UB-3", at: [3000.0, 0.0], ..BoxSpec::default() });
    let line = d.add(&Space::Model, Common::default(), EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(1.0, 0.0, 0.0) })).unwrap();
    let mut s = session(d);
    let ub3_before = s.doc().unwrap().entity(ub3).unwrap().clone();
    let undo = undo_len(&s);
    s.set_selection(vec![ub1, ub3, line]);
    let r = s.execute("tubab", &json!({ "from": "ub-1", "to": "UB-2", "mode": "commit" })).unwrap();
    assert_eq!(r["summary"], json!({ "selected": 3, "matched": 1, "replaced": 1, "unchanged": 0 }));
    assert_eq!(r["written"], json!({ "replaced": 1 }));
    assert_eq!(undo_len(&s), undo + 1);
    let ins = insert_of(&s, ub1);
    assert_eq!(ins.block, "UB-2");
    assert_eq!(ins.insert, Vec3::new(1000.0, 2000.0, 0.0), "same anchors: the insert point is kept exactly");
    assert_eq!(ins.rotation, 30f64.to_radians());
    assert_eq!(ins.scale, Vec3::new(2.0, 2.0, 2.0));
    assert_eq!(common_of(&s, ub1).layer, "OTHER");
    assert_eq!(ins.attribs.iter().map(|a| (a.tag.as_str(), a.text.value.as_str())).collect::<Vec<_>>(), [("MARK", "X")]);
    assert_eq!(**s.doc().unwrap().entity(ub3).unwrap(), *ub3_before, "other blocks are left alone");
    assert_eq!(tag_of(&s, ub1), None, "an untagged box stays untagged");
}

#[test]
fn tubab_updates_product_id_when_mapped() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    legacy_block(&mut d, "UB-2", &[]);
    add_block(&mut d, "DOOR", [0.0, 0.0], [900.0, 50.0], &[]);
    let boxed = add_box(&mut d, BoxSpec { at: [1000.0, 1000.0], rotation_deg: 90.0, tag: Some(instance("ubot-h13-7", 4, 3)), ..BoxSpec::default() });
    let mut s = session(d);
    overlay(&mut s, json!([]), json!([mapping("UB-2", Some("ubot-h10-7"), "top_center")]));
    // R20: a tagged box may not become a block that is no U-BOT box.
    let before = doc(&s);
    let e = err(&mut s, "tubab", json!({ "handles": [boxed.hex()], "from": "UBOT_H13_7", "to": "DOOR", "mode": "commit" }));
    assert!(e.contains("target block is not a known U-BOT box"), "{e}");
    assert!(Arc::ptr_eq(&doc(&s), &before));

    s.execute("tubab", &json!({ "handles": [boxed.hex()], "from": "UBOT_H13_7", "to": "UB-2", "mode": "commit" })).unwrap();
    assert_eq!(
        tag_of(&s, boxed),
        Some(UbotTag::Instance {
            instance_id: format!("{LAYOUT}:4:0"),
            product_id: "ubot-h10-7".into(),
            layout_id: LAYOUT.into(),
            count_multiplier: 3
        }),
        "product follows the block; instance and layout ids stay"
    );
    // Centre (1000, 1000) kept: the top-centre insert sits half a box up the rotated block's y axis.
    let ins = insert_of(&s, boxed);
    let off = rot([0.0, 260.0], 90.0);
    assert_xy(ins.insert, [1000.0 + off[0], 1000.0 + off[1]]);
    assert_eq!(ins.rotation, 90f64.to_radians());
}

#[test]
fn tubab_converts_anchor_to_keep_the_box_centre() {
    // R19: UB-1 (top centre) to the centred UBOT_ block keeps the box centre, scale kept raw.
    let mut d = drawing(4);
    legacy_block(&mut d, "UB-1", &[]);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    let at = [2000.0, 3000.0];
    let h = add_box(&mut d, BoxSpec { block: "UB-1", at, rotation_deg: 30.0, scale: 1.5, ..BoxSpec::default() });
    let mut s = session(d);
    let preview = s.execute("tubab", &json!({ "handles": [h.hex()], "from": "UB-1", "to": "UBOT_H13_7" })).unwrap();
    assert_eq!(preview["mode"], "preview");
    let r = s
        .execute("tubab", &json!({ "handles": [h.hex()], "from": "UB-1", "to": "UBOT_H13_7", "mode": "commit", "expectHash": preview["inputHash"] }))
        .unwrap();
    assert_eq!(r["written"]["replaced"], 1);
    let ins = insert_of(&s, h);
    let off = rot([0.0, 260.0 * 1.5], 30.0);
    assert_xy(ins.insert, [at[0] - off[0], at[1] - off[1]]);
    assert_eq!(ins.scale, Vec3::new(1.5, 1.5, 1.5));
    assert!(close(ins.rotation, 30f64.to_radians()));
    assert_eq!(tag_of(&s, h), None);
}

#[test]
fn tubab_refuses_a_size_change() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    add_block(&mut d, "BIG", [-300.0, -300.0], [600.0, 600.0], &[]);
    let h = add_box(&mut d, BoxSpec { tag: Some(instance("ubot-h13-7", 0, 1)), ..BoxSpec::default() });
    let mut s = session(d);
    overlay(&mut s, json!([product("ubot-big", "BIG", 600.0)]), json!([mapping("BIG", Some("ubot-big"), "center")]));
    let before = doc(&s);
    let e = err(&mut s, "tubab", json!({ "handles": [h.hex()], "from": "UBOT_H13_7", "to": "BIG" }));
    assert!(e.contains("size differs") && e.contains(&h.hex()), "{e}");
    assert!(Arc::ptr_eq(&doc(&s), &before));
}

#[test]
fn tubab_missing_to_block_errors() {
    let mut d = drawing(4);
    legacy_block(&mut d, "UB-1", &[]);
    let h = add_box(&mut d, BoxSpec { block: "UB-1", ..BoxSpec::default() });
    let mut s = session(d);
    let before = doc(&s);
    let undo = undo_len(&s);
    let e = err(&mut s, "tubab", json!({ "handles": [h.hex()], "from": "UB-1", "to": "UB-9", "mode": "commit" }));
    assert!(e.contains("no block `UB-9`"), "{e}");
    let e = err(&mut s, "tubab", json!({ "handles": [h.hex()], "from": "UB-2", "to": "UB-1", "mode": "commit" }));
    assert!(e.contains("no references of block `UB-2`"), "{e}");
    let e = err(&mut s, "tubab", json!({ "handles": [h.hex()], "to": "UB-1" }));
    assert!(e.contains("`from`"), "{e}");
    assert!(Arc::ptr_eq(&doc(&s), &before));
    assert_eq!(undo_len(&s), undo);
}

#[test]
fn tubab_cancel_writes_nothing() {
    let mut d = drawing(4);
    legacy_block(&mut d, "UB-1", &[]);
    legacy_block(&mut d, "UB-2", &[]);
    let a = add_box(&mut d, BoxSpec { block: "UB-1", ..BoxSpec::default() });
    let b = add_box(&mut d, BoxSpec { block: "UB-1", at: [700.0, 0.0], ..BoxSpec::default() });
    let mut s = session(d);
    let before = doc(&s);
    let undo = undo_len(&s);
    let to_apply = |s: &mut Session| {
        // The previous run left its pick selected; start without a pickfirst selection.
        s.set_selection(Vec::new());
        s.start("tubab").unwrap();
        assert_eq!(s.current_prompt().unwrap().message, "Select boxes");
        s.input(Input::Pick(vec![a, b])).unwrap();
        s.input(Input::Enter).unwrap();
        assert_eq!(s.current_prompt().unwrap().display(), "Replace block <UB-1>:");
        s.input(Input::Enter).unwrap();
        assert_eq!(s.current_prompt().unwrap().message, "With block");
        s.input(Input::Text("UB-9".into())).unwrap();
        assert!(s.log.iter().any(|l| l == "No block named UB-9."), "{:?}", s.log.iter().rev().take(3).collect::<Vec<_>>());
        assert_eq!(s.current_prompt().unwrap().message, "With block", "a missing block asks again");
        s.input(Input::Text("ub-2".into())).unwrap();
        assert!(
            s.log.iter().any(|l| l == "TUBAB: 2 references of UB-1 to replace with UB-2, 0 unchanged"),
            "{:?}",
            s.log.iter().rev().take(3).collect::<Vec<_>>()
        );
        assert_eq!(s.current_prompt().unwrap().display(), "Apply? [Yes/No] <Yes>:");
    };
    to_apply(&mut s);
    s.input(Input::Cancel).unwrap();
    assert!(s.running.is_none());
    assert_eq!(*doc(&s), *before);
    assert_eq!(undo_len(&s), undo);
    to_apply(&mut s);
    s.input(Input::Keyword("No".into())).unwrap();
    assert!(s.running.is_none());
    assert_eq!(*doc(&s), *before);
    assert_eq!(undo_len(&s), undo);
    to_apply(&mut s);
    s.input(Input::Keyword("Yes".into())).unwrap();
    assert!(s.running.is_none());
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(insert_of(&s, a).block, "UB-2");
    assert_eq!(insert_of(&s, b).block, "UB-2");
}

// ---------------- rulings ----------------

#[test]
fn untagged_generated_block_is_a_box() {
    // R18: an untagged UBOT_<SKU> reference is a box of that product, read like a legacy one.
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    let h = add_box(&mut d, BoxSpec { block: "ubot_h13_7", ..BoxSpec::default() });
    add_block(&mut d, "DOOR", [0.0, 0.0], [900.0, 50.0], &[]);
    add_box(&mut d, BoxSpec { block: "DOOR", at: [3000.0, 0.0], ..BoxSpec::default() });
    let catalog = Catalog::builtin();
    let found = existing_instances(&d, &catalog);
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].handle.as_str(), found[0].instance_id.as_deref(), found[0].product_id.as_deref()),
        (h.hex().as_str(), None, Some("ubot-h13-7"))
    );
    let items = count_items(&d, &catalog);
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].product_id.as_deref(), items[0].source, items[0].multiplier), (Some("ubot-h13-7"), CountSource::Legacy, 1));
    assert_eq!(cadcraft_ubot::count(&items, &catalog).total, 1);

    let mut s = session(d);
    s.execute("ubreplace", &json!({ "handles": [h.hex()], "to": "ubot-h10-7", "mode": "commit" })).unwrap();
    assert_eq!(insert_of(&s, h).block, "UBOT_H10_7");
    assert_eq!(tag_of(&s, h), None);
}

#[test]
fn locked_targets_are_refused_before_writing() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    d.ensure_layer("LOCKED");
    d.layer_mut("LOCKED").unwrap().locked = true;
    let free = add_box(&mut d, BoxSpec { tag: Some(instance("ubot-h13-7", 0, 1)), ..BoxSpec::default() });
    let locked = add_box(&mut d, BoxSpec { at: [700.0, 0.0], layer: "LOCKED", tag: Some(instance("ubot-h13-7", 1, 1)), ..BoxSpec::default() });
    let mut s = session(d);
    let before = doc(&s);
    let undo = undo_len(&s);
    let handles = json!([free.hex(), locked.hex()]);
    for (cmd, p) in [
        ("ubreplace", json!({ "handles": handles, "to": "ubot-h10-7", "mode": "commit" })),
        ("ubreplace", json!({ "handles": handles, "to": "ubot-h10-7" })),
        ("tubab", json!({ "handles": handles, "from": "UBOT_H13_7", "to": "UBOT_H13_7", "mode": "commit" })),
    ] {
        let e = err(&mut s, cmd, p);
        assert!(e.contains(&format!("{} is on a locked layer", locked.hex())), "{cmd}: {e}");
        assert!(Arc::ptr_eq(&doc(&s), &before));
        assert_eq!(undo_len(&s), undo);
    }
}

#[test]
fn hostile_targets_and_params_fail_cleanly() {
    let mut d = drawing(4);
    generated_block(&mut d, "UBOT_H13_7", &[]);
    add_block(&mut d, "DOOR", [0.0, 0.0], [900.0, 50.0], &[]);
    let good = add_box(&mut d, BoxSpec { tag: Some(instance("ubot-h13-7", 0, 1)), ..BoxSpec::default() });
    let door = add_box(&mut d, BoxSpec { block: "DOOR", at: [3000.0, 0.0], ..BoxSpec::default() });
    let zone = add_box(
        &mut d,
        BoxSpec {
            at: [6000.0, 0.0],
            tag: Some(UbotTag::Zone { zone_id: "z".into(), zone_type: cadcraft_ubot::ZoneType::Column, clearance_mm: None, alt_product_id: None }),
            ..BoxSpec::default()
        },
    );
    let mut malformed = Common::default();
    xdata::write(&mut malformed, &instance("ubot-h13-7", 5, 1)).unwrap();
    malformed.xdata.truncate(3);
    let broken = d
        .add(
            &Space::Model,
            malformed,
            EntityKind::Insert(Insert {
                block: "UBOT_H13_7".into(),
                insert: Vec3::new(9000.0, 0.0, 0.0),
                scale: Vec3::new(1.0, 1.0, 1.0),
                rotation: 0.0,
                attribs: Vec::new(),
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            }),
        )
        .unwrap();
    let minsert = d
        .add(
            &Space::Model,
            Common::default(),
            EntityKind::Insert(Insert {
                block: "UBOT_H13_7".into(),
                insert: Vec3::new(0.0, 9000.0, 0.0),
                scale: Vec3::new(1.0, 1.0, 1.0),
                rotation: 0.0,
                attribs: Vec::new(),
                cols: 3,
                rows: 2,
                col_spacing: 700.0,
                row_spacing: 700.0,
            }),
        )
        .unwrap();
    let nan = add_box(&mut d, BoxSpec { at: [f64::NAN, 0.0], ..BoxSpec::default() });
    let mut s = session(d);
    let before = doc(&s);
    let undo = undo_len(&s);
    let cases: Vec<(&str, Value, &str)> = vec![
        ("ubreplace", json!({ "handles": [good.hex()] }), "`to`"),
        ("ubreplace", json!({ "handles": [good.hex()], "to": "nope" }), "unknown product 'nope'"),
        ("ubreplace", json!({ "handles": [good.hex()], "to": "ubot-h10-7", "mode": "bogus" }), "unknown mode"),
        ("ubreplace", json!({ "handles": "2A", "to": "ubot-h10-7" }), "array"),
        ("ubreplace", json!({ "handles": [], "to": "ubot-h10-7" }), "no U-BOT boxes selected"),
        ("ubreplace", json!({ "handles": ["FFFFFF"], "to": "ubot-h10-7" }), "no object FFFFFF"),
        ("ubreplace", json!({ "handles": [door.hex()], "to": "ubot-h10-7" }), "is not a U-BOT box"),
        ("ubreplace", json!({ "handles": [zone.hex()], "to": "ubot-h10-7" }), "U-BOT data that is not a box"),
        ("ubreplace", json!({ "handles": [broken.hex()], "to": "ubot-h10-7" }), "malformed U-BOT data"),
        ("ubreplace", json!({ "handles": [minsert.hex()], "to": "ubot-h10-7" }), "MINSERT"),
        ("ubreplace", json!({ "handles": [nan.hex()], "to": "ubot-h10-7", "mode": "commit" }), "finite"),
        ("ubreplace", json!({ "handles": [good.hex()], "to": "ubot-h10-7", "allowResize": "yes" }), "`allowResize`"),
        ("ubreplace", json!({ "handles": [good.hex()], "to": "ubot-h10-7", "units": "furlong" }), "unknown units"),
        ("tubab", json!({ "handles": [good.hex()], "from": "UBOT_H13_7" }), "`to`"),
        ("tubab", json!({ "handles": [zone.hex()], "from": "UBOT_H13_7", "to": "UBOT_H13_7" }), "U-BOT data that is not a box"),
        ("tubab", json!({ "handles": [broken.hex()], "from": "UBOT_H13_7", "to": "DOOR" }), "malformed U-BOT data"),
        ("tubab", json!({ "handles": [minsert.hex()], "from": "UBOT_H13_7", "to": "DOOR" }), "MINSERT"),
        ("tubab", json!({ "handles": [good.hex()], "from": "UBOT_H13_7", "to": "DOOR", "mode": "nope" }), "unknown mode"),
    ];
    for (cmd, p, want) in cases {
        let e = err(&mut s, cmd, p.clone());
        assert!(e.contains(want), "{cmd} {p}: {e}");
        assert!(Arc::ptr_eq(&doc(&s), &before), "{cmd} {p}");
        assert_eq!(undo_len(&s), undo);
    }
    // Too many handles is refused before any lookup.
    let many: Vec<String> = (0..100_001).map(|i| format!("{:X}", i + 1)).collect();
    let e = err(&mut s, "ubreplace", json!({ "handles": many, "to": "ubot-h10-7" }));
    assert!(e.contains("too many"), "{e}");
    let e = err(&mut s, "tubab", json!({ "handles": many, "from": "A", "to": "B" }));
    assert!(e.contains("too many"), "{e}");
    // An insert of a block that does not exist is no box for UBREPLACE.
    let ghost = s
        .doc_mut()
        .unwrap()
        .add(
            &Space::Model,
            Common::default(),
            EntityKind::Insert(Insert {
                block: "GHOST".into(),
                insert: Vec3::ZERO,
                scale: Vec3::new(1.0, 1.0, 1.0),
                rotation: 0.0,
                attribs: Vec::new(),
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            }),
        )
        .unwrap();
    let e = err(&mut s, "ubreplace", json!({ "handles": [ghost.hex()], "to": "ubot-h10-7" }));
    assert!(e.contains("is not a U-BOT box"), "{e}");
}
