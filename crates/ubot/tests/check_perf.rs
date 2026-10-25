//! Timing guards, run with `cargo test --release -p cadcraft-ubot --test check_perf -- --ignored --nocapture`.

use std::time::Instant;

use cadcraft_ubot::{Catalog, CheckInstance, CheckRequest, ExistingInstance, FillRequest, LayoutInput, RingInput, ZoneInput, ZoneType, check, fill};

fn rect(x1: f64, y1: f64) -> RingInput {
    RingInput { vertices: vec![[0.0, 0.0, 0.0], [x1, 0.0, 0.0], [x1, y1, 0.0], [0.0, y1, 0.0]] }
}

#[test]
#[ignore = "timing"]
fn check_ten_thousand_boxes() {
    let catalog = Catalog::builtin();
    let side = 66_000.0;
    let boundary = rect(side, side);
    let zones: Vec<ZoneInput> = (0..200)
        .map(|k| {
            let (x, y) = (f64::from(k % 20) * 3000.0 + 500.0, f64::from(k / 20) * 3000.0 + 500.0);
            ZoneInput {
                id: format!("z{k}"),
                ring: RingInput { vertices: vec![[x, y, 0.0], [x + 400.0, y, 0.0], [x + 400.0, y + 400.0, 0.0], [x, y + 400.0, 0.0]] },
                zone_type: ZoneType::Column,
                clearance_mm: None,
                alt_product_id: None,
            }
        })
        .collect();
    let res = fill(
        &FillRequest {
            unit_scale: 1.0,
            layout_id: "0123456789abcdef".into(),
            previous_layout_id: None,
            reset: false,
            boundary: boundary.clone(),
            holes: vec![],
            zones: zones.clone(),
            product_id: "ubot-h13-7".into(),
            rulepack_id: None,
            rib_mm: None,
            pitch_x_mm: None,
            pitch_y_mm: None,
            angle_deg: 0.0,
            origin: None,
            edge_clearance_mm: None,
            exclusion_clearance_mm: None,
            existing: vec![],
        },
        &catalog,
    )
    .unwrap();
    let instances: Vec<CheckInstance> = res
        .add
        .iter()
        .enumerate()
        .map(|(k, p)| CheckInstance {
            base: ExistingInstance {
                handle: format!("{:X}", 0x100 + k),
                instance_id: Some(p.instance_id.clone()),
                layout_id: Some(res.layout.layout_id.clone()),
                product_id: Some(p.product_id.clone()),
                block_name: p.block_name.clone(),
                insert: p.insert,
                rotation_deg: p.rotation_deg,
                scale: 1.0,
            },
            xdata_valid: true,
            legacy: false,
        })
        .collect();
    let layout = LayoutInput {
        layout_id: res.layout.layout_id.clone(),
        boundary_handle: "A1".into(),
        boundary,
        holes: vec![],
        stored_geometry_hash: Some(res.layout.geometry_hash.clone()),
        layout: Some(res.layout.clone()),
        edge_clearance_mm: None,
    };
    let req = CheckRequest { unit_scale: 1.0, instances, layouts: vec![layout], zones };
    let t = Instant::now();
    let issues = check(&req, &catalog).unwrap();
    println!("{} boxes, {} issues in {:?}", req.instances.len(), issues.len(), t.elapsed());
    assert!(issues.is_empty());
    assert!(t.elapsed().as_secs() < 2);
}

#[test]
#[ignore = "timing"]
fn check_fifty_thousand_boxes_against_two_thousand_zones() {
    let boxes: Vec<CheckInstance> = (0..50_000)
        .map(|k| CheckInstance {
            base: ExistingInstance {
                handle: format!("{k:X}"),
                instance_id: None,
                layout_id: None,
                product_id: Some("ubot-h13-7".into()),
                block_name: "UBOT_H13_7".into(),
                insert: [f64::from(k % 250) * 600.0, f64::from(k / 250) * 600.0],
                rotation_deg: 0.0,
                scale: 1.0,
            },
            xdata_valid: true,
            legacy: false,
        })
        .collect();
    let zones: Vec<ZoneInput> = (0..2_000)
        .map(|k| {
            let (x, y) = (f64::from(k % 250) * 600.0 + 290.0, f64::from(k / 250) * 600.0 + 290.0);
            ZoneInput {
                id: format!("z{k}"),
                ring: RingInput { vertices: vec![[x, y, 0.0], [x + 20.0, y, 0.0], [x + 20.0, y + 20.0, 0.0], [x, y + 20.0, 0.0]] },
                zone_type: ZoneType::Column,
                clearance_mm: None,
                alt_product_id: None,
            }
        })
        .collect();
    let req = CheckRequest { unit_scale: 1.0, instances: boxes, layouts: vec![], zones };
    let t = Instant::now();
    let issues = check(&req, &Catalog::builtin()).unwrap();
    println!("50 000 boxes x 2 000 zones, {} issues in {:?}", issues.len(), t.elapsed());
    assert!(issues.is_empty());
    assert!(t.elapsed().as_secs() < 2);
}
