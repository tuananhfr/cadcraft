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
