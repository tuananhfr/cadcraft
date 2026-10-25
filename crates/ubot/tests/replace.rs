use cadcraft_ubot::{
    Anchor, ApprovalStatus, BlockMapping, Catalog, ExistingInstance, IssueCode, Product, ReplaceRequest, UbotError, center_from_insert, replace,
};

fn target(handle: &str, product: Option<&str>, block: &str, insert: [f64; 2], rotation_deg: f64) -> ExistingInstance {
    ExistingInstance {
        handle: handle.into(),
        instance_id: None,
        layout_id: None,
        product_id: product.map(Into::into),
        block_name: block.into(),
        insert,
        rotation_deg,
        scale: 1.0,
    }
}

fn request(targets: Vec<ExistingInstance>, to: &str, allow_resize: bool) -> ReplaceRequest {
    ReplaceRequest { unit_scale: 1.0, targets, to_product_id: to.into(), allow_resize }
}

fn catalog_with_top_center() -> Catalog {
    let mut c = Catalog::builtin();
    c.mappings.push(BlockMapping {
        product_id: Some("ubot-h13-7".into()),
        cad_engine: "any".into(),
        block_name: "UBOT_H13_7".into(),
        units_mm_per_block_unit: 1.0,
        anchor: Anchor::TopCenter,
        rotation_offset_deg: 0.0,
    });
    c
}

#[test]
fn same_size_keeps_center_and_rotation() {
    let c = Catalog::builtin();
    let r = replace(&request(vec![target("1", Some("ubot-h13-7"), "UBOT_H13_7", [1000.0, 2000.0], 30.0)], "ubot-h10-7", false), &c).unwrap();
    assert!(r.refused.is_empty() && r.warnings.is_empty());
    let ch = &r.changes[0];
    assert_eq!((ch.handle.as_str(), ch.product_id.as_str(), ch.block_name.as_str()), ("1", "ubot-h10-7", "UBOT_H10_7"));
    assert_eq!(ch.insert, [1000.0, 2000.0]);
    assert_eq!(ch.rotation_deg, 30.0);
}

#[test]
fn top_center_to_center_anchor_moves_insert() {
    let c = Catalog::builtin();
    // Legacy UB-1 is anchored at the top centre: the centre of a 520 mm box sits 260 mm below the insert.
    let r = replace(&request(vec![target("1", Some("ubot-h13-7"), "UB-1", [1000.0, 2000.0], 0.0)], "ubot-h13-7", false), &c).unwrap();
    assert_eq!(r.changes[0].insert, [1000.0, 1740.0]);
    // Rotated 90 degrees the top-centre offset points along -x.
    let r = replace(&request(vec![target("1", Some("ubot-h13-7"), "UB-1", [1000.0, 2000.0], 90.0)], "ubot-h13-7", false), &c).unwrap();
    assert!((r.changes[0].insert[0] - 1260.0).abs() < 1e-9 && (r.changes[0].insert[1] - 2000.0).abs() < 1e-9, "{:?}", r.changes[0].insert);
    // And back: centre to top-centre moves the insert up, and the centre survives the round trip.
    let tc = catalog_with_top_center();
    let r = replace(&request(vec![target("2", Some("ubot-h10-7"), "UBOT_H10_7", [500.0, 500.0], 0.0)], "ubot-h13-7", false), &tc).unwrap();
    assert_eq!(r.changes[0].insert, [500.0, 760.0]);
    let m = tc.mapping_for_product("ubot-h13-7");
    let back = center_from_insert(m, 520.0, cadcraft_geom::Vec2::new(500.0, 760.0), 0.0, 1.0);
    assert_eq!((back.x, back.y), (500.0, 500.0));
}

#[test]
fn different_size_refused_without_allow_resize() {
    let mut c = Catalog::builtin();
    let mut big = c.products[0].clone();
    big.id = "ubot-big".into();
    big.sku = "BIG".into();
    big.width_mm = 600.0;
    big.length_mm = 600.0;
    big.approval_status = ApprovalStatus::Draft;
    c.products.push(Product { ..big });
    let t = target("1", Some("ubot-h13-7"), "UBOT_H13_7", [0.0, 0.0], 0.0);
    let r = replace(&request(vec![t.clone()], "ubot-big", false), &c).unwrap();
    assert!(r.changes.is_empty());
    assert_eq!(r.refused.len(), 1);
    assert_eq!(r.refused[0].code, IssueCode::Clearance);
    assert!(r.refused[0].message.contains("size differs"));
    assert_eq!(r.refused[0].handle.as_deref(), Some("1"));
    let r = replace(&request(vec![t], "ubot-big", true), &c).unwrap();
    assert_eq!((r.changes.len(), r.refused.len()), (1, 0));
    assert_eq!(r.changes[0].block_name, "UBOT_BIG");
}

#[test]
fn legacy_target_is_assumed_to_be_the_new_size_with_a_warning() {
    let c = Catalog::builtin();
    let r = replace(&request(vec![target("1", None, "UB-2", [0.0, 0.0], 0.0)], "ubot-h13-7", false), &c).unwrap();
    assert_eq!(r.changes.len(), 1);
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.warnings[0].code, IssueCode::LegacyBlock);
}

#[test]
fn unknown_products_and_hostile_input() {
    let c = Catalog::builtin();
    assert!(matches!(replace(&request(vec![], "nope", false), &c), Err(UbotError::UnknownProduct(_))));
    let r = replace(&request(vec![target("1", Some("nope"), "X", [0.0, 0.0], 0.0)], "ubot-h13-7", false), &c).unwrap();
    assert_eq!((r.changes.len(), r.refused.len()), (0, 1));
    assert_eq!(r.refused[0].code, IssueCode::UnknownProduct);
    assert!(replace(&request(vec![target("1", None, "X", [f64::NAN, 0.0], 0.0)], "ubot-h13-7", false), &c).is_err());
    let mut bad = request(vec![], "ubot-h13-7", false);
    bad.unit_scale = f64::INFINITY;
    assert!(replace(&bad, &c).is_err());
}

#[test]
fn json_shape_is_camel_case() {
    let req: ReplaceRequest = serde_json::from_value(serde_json::json!({
        "unitScale": 1.0, "toProductId": "ubot-h13-7",
        "targets": [{"handle": "1", "blockName": "UB-1", "insert": [0, 0], "rotationDeg": 0, "scale": 1}]
    }))
    .unwrap();
    assert!(!req.allow_resize);
}

fn mapping(product: Option<&str>, block: &str, anchor: Anchor, units: f64, offset: f64) -> BlockMapping {
    BlockMapping {
        product_id: product.map(Into::into),
        cad_engine: "any".into(),
        block_name: block.into(),
        units_mm_per_block_unit: units,
        anchor,
        rotation_offset_deg: offset,
    }
}

fn near(a: [f64; 2], b: [f64; 2]) {
    assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "{a:?} != {b:?}");
}

#[test]
fn metre_drawing_with_block_scale() {
    let c = Catalog::builtin();
    let mut t = target("1", Some("ubot-h13-7"), "UB-1", [1.0, 2.0], 0.0);
    t.scale = 0.001;
    let mut req = request(vec![t], "ubot-h13-7", false);
    req.unit_scale = 1000.0;
    let ch = &replace(&req, &c).unwrap().changes[0];
    near(ch.insert, [1.0, 1.74]);
    assert!((ch.scale - 0.001).abs() < 1e-12, "{}", ch.scale);
}

#[test]
fn block_scale_other_than_one_scales_the_anchor_offset() {
    let c = Catalog::builtin();
    let mut t = target("1", Some("ubot-h13-7"), "UB-1", [1000.0, 2000.0], 0.0);
    t.scale = 2.0;
    let ch = &replace(&request(vec![t], "ubot-h13-7", false), &c).unwrap().changes[0];
    near(ch.insert, [1000.0, 1480.0]);
    assert_eq!(ch.scale, 1.0);
}

#[test]
fn rotation_offsets_of_both_blocks_apply() {
    let mut c = Catalog::builtin();
    c.mappings.push(mapping(None, "OLDX", Anchor::Center, 1.0, 10.0));
    c.mappings.push(mapping(Some("ubot-h10-7"), "NEWX", Anchor::Center, 1.0, 90.0));
    let r = replace(&request(vec![target("1", Some("ubot-h13-7"), "OLDX", [5.0, 6.0], 40.0)], "ubot-h10-7", false), &c).unwrap();
    // Footprint angle 40 - 10 = 30, then the new block's own offset.
    assert_eq!(r.changes[0].rotation_deg, 120.0);
    assert_eq!(r.changes[0].block_name, "NEWX");
    near(r.changes[0].insert, [5.0, 6.0]);
}

#[test]
fn top_center_with_a_different_size_and_allow_resize() {
    let mut c = catalog_with_top_center();
    let mut big = c.products[0].clone();
    big.id = "ubot-big".into();
    big.sku = "BIG".into();
    big.width_mm = 600.0;
    big.length_mm = 600.0;
    c.products.push(big);
    c.mappings.push(mapping(Some("ubot-big"), "UBOT_BIG", Anchor::TopCenter, 1.0, 0.0));
    // 520 mm box, top-centre insert (500, 500): centre (500, 240); the 600 mm box puts its insert 300 above that.
    let r = replace(&request(vec![target("1", Some("ubot-h13-7"), "UBOT_H13_7", [500.0, 500.0], 0.0)], "ubot-big", true), &c).unwrap();
    near(r.changes[0].insert, [500.0, 540.0]);
}

#[test]
fn duplicate_target_handles_are_rejected() {
    let c = Catalog::builtin();
    let t = target("1", Some("ubot-h13-7"), "X", [0.0, 0.0], 0.0);
    assert!(matches!(replace(&request(vec![t.clone(), t], "ubot-h13-7", false), &c), Err(UbotError::BadInput(_))));
}

#[test]
fn mapped_legacy_block_has_a_known_size_and_no_warning() {
    let mut c = Catalog::builtin();
    c.mappings.push(mapping(Some("ubot-h13-7"), "UB-9", Anchor::Center, 1.0, 0.0));
    let mut big = c.products[0].clone();
    big.id = "ubot-big".into();
    big.sku = "BIG".into();
    big.width_mm = 600.0;
    c.products.push(big);
    let r = replace(&request(vec![target("1", None, "UB-9", [0.0, 0.0], 0.0)], "ubot-h10-7", false), &c).unwrap();
    assert!(r.warnings.is_empty() && r.changes.len() == 1);
    let r = replace(&request(vec![target("1", None, "UB-9", [0.0, 0.0], 0.0)], "ubot-big", false), &c).unwrap();
    assert_eq!((r.changes.len(), r.refused.len()), (0, 1));
}
