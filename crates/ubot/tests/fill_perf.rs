//! Spec §6 performance goal: 10 000 boxes < 2 s (release). Run with
//! `cargo test --release -p cadcraft-ubot --test fill_perf -- --ignored --nocapture`.

use std::time::Instant;

use cadcraft_ubot::{Catalog, ExistingInstance, FillRequest, RingInput, ZoneInput, ZoneType, fill};

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RingInput {
    RingInput { vertices: vec![[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]] }
}

#[test]
#[ignore]
fn ten_thousand_boxes_under_two_seconds() {
    // 66 000 mm square → 100 × 100 cells; a 400 mm column in every sixth cell (1 667 columns, 6 668 zone vertices).
    let mut zones = Vec::new();
    for k in (0..10_000).step_by(6) {
        let (i, j) = ((k % 100) as f64, (k / 100) as f64);
        let (cx, cy) = (330.0 + 660.0 * i, 65_670.0 - 660.0 * j);
        zones.push(ZoneInput {
            id: format!("col{k}"),
            ring: rect(cx - 200.0, cy - 200.0, cx + 200.0, cy + 200.0),
            zone_type: ZoneType::Column,
            clearance_mm: None,
            alt_product_id: None,
        });
    }
    let mut req = FillRequest {
        unit_scale: 1.0,
        layout_id: "0123456789abcdef".into(),
        previous_layout_id: None,
        reset: false,
        boundary: rect(0.0, 0.0, 66_000.0, 66_000.0),
        holes: vec![],
        zones,
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
    };
    let catalog = Catalog::builtin();
    let t = Instant::now();
    let first = fill(&req, &catalog).unwrap();
    let fresh = t.elapsed();
    assert_eq!(first.add.len(), 10_000 - 1_667);
    assert_eq!(first.summary.invalid, 1_667);

    req.existing = first
        .add
        .iter()
        .enumerate()
        .map(|(k, p)| ExistingInstance {
            handle: format!("{:X}", 0x100 + k),
            instance_id: Some(p.instance_id.clone()),
            layout_id: Some(req.layout_id.clone()),
            product_id: Some(p.product_id.clone()),
            block_name: p.block_name.clone(),
            insert: p.insert,
            rotation_deg: p.rotation_deg,
            scale: 1.0,
            scale_y: None,
        })
        .collect();
    let t = Instant::now();
    let rerun = fill(&req, &catalog).unwrap();
    let again = t.elapsed();
    assert_eq!((rerun.add.len(), rerun.keep.len()), (0, 10_000 - 1_667));

    // Rotated grid: every cell meets the boundary at an angle, so the edge index and scanline do real work.
    req.existing.clear();
    req.angle_deg = 30.0;
    let t = Instant::now();
    let rotated = fill(&req, &catalog).unwrap();
    let turned = t.elapsed();
    assert!(!rotated.add.is_empty());

    println!("fill fresh {fresh:?}, rerun with {} existing {again:?}, rotated 30° {turned:?}", first.add.len());
    for d in [fresh, again, turned] {
        assert!(d.as_secs_f64() < 2.0, "{d:?}");
    }
}

#[test]
#[ignore]
fn dense_boundary_at_the_cell_limit() {
    // 19 996-gon of radius 104 km: its bounds hold 316 × 316 ≈ 99 856 cells, so a per-cell point-in-polygon would cost
    // ~2·10⁹ edge visits; the scanline only visits the edges each row crosses.
    let n = 19_996;
    let r = 104_000.0;
    let vertices = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            [r * a.cos(), r * a.sin(), 0.0]
        })
        .collect();
    let req = FillRequest {
        unit_scale: 1.0,
        layout_id: "0123456789abcdef".into(),
        previous_layout_id: None,
        reset: false,
        boundary: RingInput { vertices },
        holes: vec![],
        zones: vec![],
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
    };
    let t = Instant::now();
    let res = fill(&req, &Catalog::builtin()).unwrap();
    let took = t.elapsed();
    println!("dense boundary: {} boxes, {} problems in {took:?}", res.add.len(), res.problems.len());
    assert!(res.add.len() > 70_000);
    assert!(took.as_secs_f64() < 2.0, "{took:?}");

    // A generous but realistic 1 m clearance stays inside the work budget.
    let mut one_m = req.clone();
    one_m.edge_clearance_mm = Some(1000.0);
    let t = Instant::now();
    let res = fill(&one_m, &Catalog::builtin()).unwrap();
    let took = t.elapsed();
    println!("dense boundary, 1 m clearance: {} boxes in {took:?}", res.add.len());
    assert!(took.as_secs_f64() < 2.0, "{took:?}");

    // The widest legal clearance pulls ~600 000 edges into reach of each edge cell; the work budget must stop it fast.
    let mut wide = req.clone();
    wide.edge_clearance_mm = Some(cadcraft_ubot::MAX_CLEARANCE_MM);
    let t = Instant::now();
    let res = fill(&wide, &Catalog::builtin());
    let took = t.elapsed();
    println!("dense boundary, 10 m clearance: {} in {took:?}", if res.is_ok() { "ok" } else { "error" });
    assert!(matches!(res, Ok(_) | Err(cadcraft_ubot::UbotError::Limit(_))), "{:?}", res.err());
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
}
