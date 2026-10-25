use cadcraft_ubot::{Anchor, ApprovalStatus, Catalog, UbotError, input_hash};
use serde_json::json;

#[test]
fn builtin_has_four_draft_products() {
    let c = Catalog::builtin();
    let ids: Vec<&str> = c.products.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["ubot-h13-7", "ubot-h10-7", "ubot-h20-7", "ubot-h17-7"]);
    let floors: Vec<Option<f64>> = c.products.iter().map(|p| p.slab_thickness_mm).collect();
    assert_eq!(floors, [Some(270.0), Some(240.0), Some(350.0), Some(320.0)]);
    for p in &c.products {
        assert_eq!((p.width_mm, p.length_mm), (520.0, 520.0));
        assert_eq!(p.approval_status, ApprovalStatus::Draft);
        assert!(p.source_ref.is_some());
    }
    assert_eq!(c.catalog_id, "lpc-builtin");
}

#[test]
fn builtin_rulepack_is_lpc_draft_1() {
    let c = Catalog::builtin();
    let r = c.rulepack(None).expect("rulepack");
    assert_eq!(r.id, "lpc-draft-1");
    assert_eq!(r.rib_mm_default, 140.0);
    assert_eq!(r.rib_mm_choices, [120.0, 140.0]);
    assert_eq!((r.edge_clearance_mm, r.exclusion_clearance_mm), (0.0, 0.0));
    assert_eq!(r.approval_status, ApprovalStatus::Draft);
    assert!(c.rulepack(Some("lpc-draft-1")).is_some());
    assert!(c.rulepack(Some("nope")).is_none());
}

#[test]
fn legacy_mappings_are_top_center_without_product() {
    let c = Catalog::builtin();
    for n in ["UB-1", "UB-2", "UB-3", "UB-4"] {
        let m = c.mapping_for_block(n).expect(n);
        assert_eq!(m.anchor, Anchor::TopCenter);
        assert!(m.product_id.is_none());
        assert_eq!(m.cad_engine, "any");
    }
}

#[test]
fn overlay_replaces_by_id_and_appends() {
    let base = Catalog::builtin();
    let mut other = Catalog::builtin();
    other.products.truncate(1);
    other.products[0].name = "Changed".into();
    let mut extra = other.products[0].clone();
    extra.id = "new".into();
    other.products.push(extra);
    let m = base.overlay(&other);
    assert_eq!(m.products.len(), 5);
    assert_eq!(m.products[0].name, "Changed");
    assert_eq!(m.products[4].id, "new");
    assert_eq!(m.products[1].id, "ubot-h10-7");
}

#[test]
fn from_json_rejects_wrong_schema_and_nan() {
    let good = serde_json::to_string(&Catalog::builtin()).unwrap();
    assert!(Catalog::from_json(&good).is_ok());
    let bad_schema = good.replace("ubot.catalog/1", "ubot.catalog/2");
    assert!(matches!(Catalog::from_json(&bad_schema), Err(UbotError::BadInput(_))));
    let neg = good.replacen("\"widthMm\":520.0", "\"widthMm\":-1.0", 1);
    assert_ne!(neg, good);
    assert!(matches!(Catalog::from_json(&neg), Err(UbotError::BadInput(_))));
    let zero = good.replacen("\"widthMm\":520.0", "\"widthMm\":0", 1);
    assert!(matches!(Catalog::from_json(&zero), Err(UbotError::BadInput(_))));
    let nan = good.replacen("\"widthMm\":520.0", "\"widthMm\":NaN", 1);
    assert!(matches!(Catalog::from_json(&nan), Err(UbotError::BadInput(_))));
    assert!(matches!(Catalog::from_json("{"), Err(UbotError::BadInput(_))));
}

#[test]
fn from_json_rejects_duplicate_product_ids() {
    let mut c = Catalog::builtin();
    let dup = c.products[0].clone();
    c.products.push(dup);
    let s = serde_json::to_string(&c).unwrap();
    assert!(matches!(Catalog::from_json(&s), Err(UbotError::BadInput(_))));
}

#[test]
fn mapping_lookup_is_case_insensitive() {
    let c = Catalog::builtin();
    assert!(c.mapping_for_block("ub-3").is_some());
    assert!(c.mapping_for_block("Ub-3").is_some());
    assert!(c.mapping_for_block("UB-9").is_none());
    assert!(c.mapping_for_product("ubot-h13-7").is_none());
}

#[test]
fn default_block_name_uppercases_and_replaces() {
    let c = Catalog::builtin();
    assert_eq!(Catalog::default_block_name(&c.products[0]), "UBOT_H13_7");
}

#[test]
fn input_hash_ignores_key_order_and_tiny_float_noise() {
    let a = input_hash(&json!({"a": 1.0000000001, "b": 2}));
    let b = input_hash(&json!({"b": 2, "a": 1}));
    assert_eq!(a, b);
    assert_eq!(a.len(), 64);
    assert_ne!(a, input_hash(&json!({"a": 1.001, "b": 2})));
    assert_eq!(input_hash(&json!({"a": -0.0})), input_hash(&json!({"a": 0})));
}
