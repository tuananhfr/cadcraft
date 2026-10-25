use cadcraft_ubot::{Catalog, CountItem, CountSource, RingInput, UbotError, count, count_in};

fn item(product: Option<&str>, block: &str, multiplier: u64, source: CountSource) -> CountItem {
    CountItem { product_id: product.map(Into::into), block_name: block.into(), multiplier, source }
}

fn square(side: f64) -> RingInput {
    RingInput { vertices: vec![[0.0, 0.0, 0.0], [side, 0.0, 0.0], [side, side, 0.0], [0.0, side, 0.0]] }
}

#[test]
fn tagged_and_legacy_counted_separately() {
    let items = [
        item(Some("ubot-h13-7"), "UBOT_H13_7", 1, CountSource::Tagged),
        item(Some("ubot-h13-7"), "UBOT_H13_7", 1, CountSource::Tagged),
        item(Some("ubot-h13-7"), "OLD", 1, CountSource::Legacy),
    ];
    let c = count(&items, &Catalog::builtin());
    assert_eq!(c.rows.len(), 1);
    let row = &c.rows[0];
    assert_eq!((row.product_id.as_deref(), row.name.as_str()), (Some("ubot-h13-7"), "UBOT H13+7"));
    assert_eq!((row.tagged, row.legacy, row.total), (2, 1, 3));
    assert_eq!(c.total, 3);
}

#[test]
fn multiplier_counts() {
    let items = [item(Some("ubot-h10-7"), "UBOT_H10_7", 12, CountSource::Tagged), item(Some("ubot-h10-7"), "UBOT_H10_7", 3, CountSource::Tagged)];
    let c = count(&items, &Catalog::builtin());
    assert_eq!((c.rows[0].tagged, c.total), (15, 15));
    assert_eq!(count(&[], &Catalog::builtin()).total, 0);
}

#[test]
fn unknown_block_rows_after_catalog_rows() {
    let items = [
        item(None, "ub-2", 4, CountSource::Legacy),
        item(None, "UB-2", 1, CountSource::Legacy),
        item(None, "UB-1", 2, CountSource::Legacy),
        item(Some("ubot-h20-7"), "UBOT_H20_7", 1, CountSource::Tagged),
        item(Some("ubot-h13-7"), "UBOT_H13_7", 1, CountSource::Tagged),
        item(Some("not-in-catalog"), "ZZ", 1, CountSource::Tagged),
    ];
    let c = count(&items, &Catalog::builtin());
    let names: Vec<_> = c.rows.iter().map(|r| r.name.as_str()).collect();
    // Catalog order (h13 before h20), then unknown blocks by name; "UB-2" spellings merge and show the smaller one.
    assert_eq!(names, ["UBOT H13+7", "UBOT H20+7", "UB-1", "UB-2", "ZZ"]);
    assert!(c.rows[2..].iter().all(|r| r.product_id.is_none()));
    assert_eq!((c.rows[3].legacy, c.rows[3].tagged), (5, 0));
    assert_eq!(c.total, 10);
}

#[test]
fn overflow_saturates() {
    let items = [
        item(Some("ubot-h13-7"), "A", u64::MAX, CountSource::Tagged),
        item(Some("ubot-h13-7"), "A", 5, CountSource::Tagged),
        item(Some("ubot-h13-7"), "A", 5, CountSource::Legacy),
    ];
    let c = count(&items, &Catalog::builtin());
    assert_eq!((c.rows[0].tagged, c.rows[0].legacy, c.rows[0].total), (u64::MAX, 5, u64::MAX));
    assert_eq!(c.total, u64::MAX);
}

#[test]
fn count_in_boundary_filters_by_center() {
    let b = |x: f64, y: f64| (item(Some("ubot-h13-7"), "UBOT_H13_7", 1, CountSource::Tagged), [x, y]);
    // Drawing units are metres (unit scale 1000): the slab is 3 m x 3 m.
    let items = [b(1.0, 1.0), b(2.9, 2.9), b(3.1, 1.0), b(-0.1, 1.0), b(f64::NAN, 1.0)];
    let ring = square(3.0);
    let c = count_in(&items, Some((&ring, 1000.0)), &Catalog::builtin()).unwrap();
    assert_eq!(c.total, 2);
    assert_eq!(count_in(&items, None, &Catalog::builtin()).unwrap().total, 5);
    let degenerate = RingInput { vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]] };
    assert!(matches!(count_in(&items, Some((&degenerate, 1.0)), &Catalog::builtin()), Err(UbotError::Geometry(_))));
    assert!(count_in(&items, Some((&ring, 0.0)), &Catalog::builtin()).is_err());
}

#[test]
fn json_shape_is_camel_case() {
    let c = count(&[item(Some("ubot-h13-7"), "X", 1, CountSource::Legacy)], &Catalog::builtin());
    let json = serde_json::to_value(&c).unwrap();
    assert_eq!(json["rows"][0]["productId"], "ubot-h13-7");
    let it: CountItem = serde_json::from_value(serde_json::json!({"blockName": "UB-1", "multiplier": 2, "source": "legacy"})).unwrap();
    assert_eq!(it.source, CountSource::Legacy);
}
