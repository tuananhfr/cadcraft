//! Drawing audit. Fixtures come from real fills of a 6600 mm square: box 520, pitch 660, cell (i, j) centred at
//! (330 + 660i, 6270 - 660j), 70 mm from the slab edge.

use cadcraft_ubot::{
    Catalog, CheckInstance, CheckRequest, ExistingInstance, FillRequest, FillResult, Issue, IssueCode, LayoutInput, RingInput, Severity, UbotError,
    ZoneInput, ZoneType, check, fill, geometry_hash,
};

const LAYOUT: &str = "0123456789abcdef";

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RingInput {
    RingInput { vertices: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].iter().map(|p| [p[0], p[1], 0.0]).collect() }
}

fn square() -> RingInput {
    rect(0.0, 0.0, 6600.0, 6600.0)
}

fn fill_req(boundary: RingInput) -> FillRequest {
    FillRequest {
        unit_scale: 1.0,
        layout_id: LAYOUT.into(),
        previous_layout_id: None,
        reset: false,
        boundary,
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
    }
}

fn filled() -> FillResult {
    fill(&fill_req(square()), &Catalog::builtin()).unwrap()
}

/// Instances as an adapter would read them after writing the fill result; handles `100`, `101`, … (hex) in `add` order.
fn instances(res: &FillResult) -> Vec<CheckInstance> {
    res.add
        .iter()
        .enumerate()
        .map(|(k, p)| CheckInstance {
            base: ExistingInstance {
                handle: format!("{:X}", 0x100 + k),
                instance_id: Some(p.instance_id.clone()),
                layout_id: Some(LAYOUT.into()),
                product_id: Some(p.product_id.clone()),
                block_name: p.block_name.clone(),
                insert: p.insert,
                rotation_deg: p.rotation_deg,
                scale: 1.0,
                scale_y: None,
            },
            xdata_valid: true,
            legacy: false,
        })
        .collect()
}

fn layout_of(res: &FillResult, boundary: RingInput) -> LayoutInput {
    LayoutInput {
        layout_id: LAYOUT.into(),
        boundary_handle: "A1".into(),
        boundary,
        holes: vec![],
        stored_geometry_hash: Some(res.layout.geometry_hash.clone()),
        layout: Some(res.layout.clone()),
        edge_clearance_mm: None,
    }
}

fn request(res: &FillResult) -> CheckRequest {
    CheckRequest { unit_scale: 1.0, instances: instances(res), layouts: vec![layout_of(res, square())], zones: vec![] }
}

fn run(r: &CheckRequest) -> Vec<Issue> {
    check(r, &Catalog::builtin()).unwrap()
}

/// A loose box (no layout) of the main product centred at `(x, y)`.
fn loose(handle: &str, id: Option<&str>, x: f64, y: f64) -> CheckInstance {
    CheckInstance {
        base: ExistingInstance {
            handle: handle.into(),
            instance_id: id.map(Into::into),
            layout_id: None,
            product_id: Some("ubot-h13-7".into()),
            block_name: "UBOT_H13_7".into(),
            insert: [x, y],
            rotation_deg: 0.0,
            scale: 1.0,
            scale_y: None,
        },
        xdata_valid: true,
        legacy: false,
    }
}

fn loose_req(instances: Vec<CheckInstance>) -> CheckRequest {
    CheckRequest { unit_scale: 1.0, instances, layouts: vec![], zones: vec![] }
}

fn with_code(issues: &[Issue], code: IssueCode) -> Vec<&Issue> {
    issues.iter().filter(|i| i.code == code).collect()
}

#[test]
fn clean_fill_has_no_issues() {
    assert!(run(&request(&filled())).is_empty());
}

#[test]
fn duplicate_layout_flagged() {
    let res = filled();
    let mut r = request(&res);
    let mut copy = layout_of(&res, square());
    copy.boundary_handle = "B2".into();
    r.layouts.push(copy);
    let issues = run(&r);
    let dups = with_code(&issues, IssueCode::DuplicateLayout);
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].handle.as_deref(), Some("B2"));
    assert_eq!(dups[0].severity, Severity::Error);
    assert_eq!(issues.len(), 1, "the original boundary still owns the layout: {issues:?}");
}

#[test]
fn manual_moved_info() {
    let res = filled();
    let mut r = request(&res);
    r.instances[0].base.insert[0] += 5.0;
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, IssueCode::ManualMoved);
    assert_eq!(issues[0].severity, Severity::Info);
    assert_eq!(issues[0].handle.as_deref(), Some("100"));
    // Within the 1 mm fill tolerance nothing is reported.
    let mut r = request(&res);
    r.instances[0].base.insert[0] += 0.5;
    assert!(run(&r).is_empty());
}

#[test]
fn manual_moved_box_that_overlaps_a_neighbour_yields_both_issues() {
    let res = filled();
    let mut r = request(&res);
    // Cell (0, 0) slides 200 mm towards cell (1, 0): the 140 mm rib closes and 60 mm of overlap remain.
    r.instances[0].base.insert[0] += 200.0;
    let issues = run(&r);
    assert_eq!(with_code(&issues, IssueCode::ManualMoved).len(), 1, "{issues:?}");
    let overlaps = with_code(&issues, IssueCode::Overlap);
    assert_eq!(overlaps.len(), 1, "{issues:?}");
    assert_eq!(overlaps[0].severity, Severity::Error);
    assert_eq!(issues.len(), 2, "{issues:?}");
}

#[test]
fn duplicate_ids_flagged_once_per_extra_copy() {
    let r = loose_req(vec![
        loose("10", Some("x:0:0"), 0.0, 0.0),
        loose("11", Some("x:0:0"), 2000.0, 0.0),
        loose("12", Some("x:0:0"), 4000.0, 0.0),
        loose("13", Some("x:1:0"), 6000.0, 0.0),
    ]);
    let issues = run(&r);
    let dups = with_code(&issues, IssueCode::DuplicateId);
    let mut handles: Vec<_> = dups.iter().map(|i| i.handle.clone().unwrap()).collect();
    handles.sort();
    assert_eq!(handles, ["11", "12"]);
    assert_eq!(issues.len(), 2);
}

#[test]
fn invalid_xdata_flagged() {
    let mut r = loose_req(vec![loose("10", None, 0.0, 0.0)]);
    r.instances[0].xdata_valid = false;
    let issues = run(&r);
    assert_eq!(issues.len(), 1);
    assert_eq!((issues[0].code, issues[0].severity), (IssueCode::XdataInvalid, Severity::Error));
}

#[test]
fn unknown_product_flagged() {
    let mut r = loose_req(vec![loose("10", None, 0.0, 0.0)]);
    r.instances[0].base.product_id = Some("ubot-h99-9".into());
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].code, issues[0].severity), (IssueCode::UnknownProduct, Severity::Error));
    assert!(issues[0].message.contains("ubot-h99-9"));
}

#[test]
fn legacy_block_is_warning() {
    let mut r = loose_req(vec![loose("10", None, 0.0, 0.0)]);
    r.instances[0].base.product_id = None;
    r.instances[0].base.block_name = "UB-1".into();
    r.instances[0].legacy = true;
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].code, issues[0].severity), (IssueCode::LegacyBlock, Severity::Warning));
}

#[test]
fn overlap_between_two_boxes() {
    let r = loose_req(vec![loose("10", None, 0.0, 0.0), loose("11", None, 300.0, 0.0), loose("12", None, 3000.0, 0.0)]);
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, IssueCode::Overlap);
    assert_eq!(issues[0].handle.as_deref(), Some("11"));
    // Boxes sharing an edge do not overlap.
    assert!(run(&loose_req(vec![loose("10", None, 0.0, 0.0), loose("11", None, 520.0, 0.0)])).is_empty());
}

#[test]
fn box_outside_boundary_and_in_zone() {
    let res = filled();
    let mut r = request(&res);
    r.zones.push(ZoneInput {
        id: "c1".into(),
        ring: rect(1900.0, 5500.0, 2300.0, 5900.0),
        zone_type: ZoneType::Column,
        clearance_mm: None,
        alt_product_id: None,
    });
    let mut stray = loose("FF0", None, 7000.0, 3000.0);
    stray.base.layout_id = Some(LAYOUT.into());
    r.instances.push(stray);
    let issues = run(&r);
    let handles = |code| -> Vec<String> { with_code(&issues, code).iter().map(|i| i.handle.clone().unwrap()).collect() };
    assert_eq!(handles(IssueCode::OutsideBoundary), ["FF0"], "{issues:?}");
    assert!(issues.iter().filter(|i| matches!(i.code, IssueCode::OutsideBoundary | IssueCode::InZone)).all(|i| i.severity == Severity::Error));
    // Column x 1900..2300, y 5500..5900 reaches the boxes of cells (2, 1) and (3, 1) only.
    let cell = |i: i64, j: i64| {
        r.instances.iter().find(|n| n.base.instance_id.as_deref() == Some(format!("{LAYOUT}:{i}:{j}").as_str())).unwrap().base.handle.clone()
    };
    let mut in_zone = handles(IssueCode::InZone);
    in_zone.sort();
    let mut expected = vec![cell(2, 1), cell(3, 1)];
    expected.sort();
    assert_eq!(in_zone, expected, "{issues:?}");
}

#[test]
fn box_in_a_hole_is_outside_the_slab() {
    let res = filled();
    let mut r = request(&res);
    r.layouts[0].holes.push(rect(300.0, 5500.0, 900.0, 6100.0));
    r.layouts[0].stored_geometry_hash = None;
    let issues = run(&r);
    // The hole 300..900 x 5500..6100 reaches the boxes of cells (0,0), (1,0), (0,1) and (1,1).
    assert_eq!(issues.len(), 4, "{issues:?}");
    assert!(issues.iter().all(|i| i.code == IssueCode::OutsideBoundary && i.severity == Severity::Error && i.message.contains("hole")));
}

#[test]
fn box_of_a_shrunken_boundary_is_outside() {
    let res = filled();
    let mut r = request(&res);
    r.layouts[0].boundary = rect(0.0, 0.0, 3000.0, 6600.0);
    let issues = run(&r);
    let outside: Vec<String> = with_code(&issues, IssueCode::OutsideBoundary).iter().map(|i| i.handle.clone().unwrap()).collect();
    // Cell (5, 0) has its centre at x = 3630, wholly right of the new boundary; cell (4, 0) straddles it.
    let h = |i: i64, j: i64| {
        r.instances.iter().find(|n| n.base.instance_id.as_deref() == Some(format!("{LAYOUT}:{i}:{j}").as_str())).unwrap().base.handle.clone()
    };
    assert!(outside.contains(&h(5, 0)), "{outside:?}");
    assert!(outside.contains(&h(4, 0)), "{outside:?}");
    assert!(!outside.contains(&h(3, 0)), "{outside:?}");
    assert_eq!(with_code(&issues, IssueCode::StaleLayout).len(), 1);
}

#[test]
fn stale_layout_when_geometry_hash_differs() {
    let res = filled();
    let mut r = request(&res);
    assert!(run(&r).is_empty());
    r.layouts[0].stored_geometry_hash = Some("0".repeat(64));
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].code, issues[0].severity), (IssueCode::StaleLayout, Severity::Warning));
    assert_eq!(issues[0].handle.as_deref(), Some("A1"));
    // The stored hash is the one the shared function computes.
    assert_eq!(res.layout.geometry_hash, geometry_hash(1.0, &square(), &[], &[]).unwrap());
    r.layouts[0].stored_geometry_hash = None;
    assert!(run(&r).is_empty());
}

#[test]
fn edge_clearance_shortfall_is_reported() {
    let res = filled();
    let mut r = request(&res);
    r.layouts[0].edge_clearance_mm = Some(100.0);
    let issues = run(&r);
    let short = with_code(&issues, IssueCode::Clearance);
    // The 36 boxes on the rim of the 10 x 10 grid sit 70 mm from the slab edge.
    assert_eq!(short.len(), 36, "{issues:?}");
    assert!(short.iter().all(|i| i.severity == Severity::Error));
    r.layouts[0].edge_clearance_mm = Some(70.0);
    assert!(run(&r).is_empty());
}

#[test]
fn issues_run_top_down_then_left_to_right() {
    let r = loose_req(vec![
        loose("10", None, 0.0, 0.0),
        loose("11", None, 100.0, 100.0),
        loose("12", None, 9000.0, 5000.0),
        loose("13", None, 9100.0, 5100.0),
    ]);
    let issues = run(&r);
    let handles: Vec<_> = issues.iter().map(|i| i.handle.clone().unwrap()).collect();
    // Overlaps are carried by the later handle of each pair: 13 at y 5100 comes before 11 at y 100.
    assert_eq!(handles, ["13", "11"]);
}

#[test]
fn json_shape_is_camel_case_and_screaming_codes() {
    let issues = run(&loose_req(vec![loose("10", None, 0.0, 0.0), loose("11", None, 300.0, 0.0)]));
    let json = serde_json::to_value(&issues[0]).unwrap();
    assert_eq!(json["code"], "OVERLAP");
    assert_eq!(json["severity"], "error");
    let req: CheckRequest = serde_json::from_value(serde_json::json!({
        "unitScale": 1.0,
        "instances": [{"base": {"handle": "1", "blockName": "UB-1", "insert": [0, 0], "rotationDeg": 0, "scale": 1}, "xdataValid": false, "legacy": true}]
    }))
    .unwrap();
    assert!(req.layouts.is_empty() && req.zones.is_empty());
}

#[test]
fn hostile_input_returns_errors() {
    let bad = |mut f: Box<dyn FnMut(&mut CheckRequest)>| {
        let mut r = loose_req(vec![loose("10", None, 0.0, 0.0)]);
        f(&mut r);
        check(&r, &Catalog::builtin())
    };
    assert!(matches!(bad(Box::new(|r| r.unit_scale = 0.0)), Err(UbotError::BadInput(_))));
    assert!(matches!(bad(Box::new(|r| r.instances[0].base.insert[0] = f64::NAN)), Err(UbotError::BadInput(_))));
    assert!(matches!(bad(Box::new(|r| r.instances[0].base.scale = 0.0)), Err(UbotError::BadInput(_))));
    assert!(matches!(
        bad(Box::new(|r| {
            r.unit_scale = 1e300;
            r.instances[0].base.insert[0] = 1e10;
        })),
        Err(UbotError::BadInput(_))
    ));
    // Finite but extreme coordinates must not panic.
    let _ = bad(Box::new(|r| r.instances[0].base.insert[0] = f64::MAX));
    let res = filled();
    let mut r = request(&res);
    r.layouts[0].edge_clearance_mm = Some(1e9);
    assert!(matches!(check(&r, &Catalog::builtin()), Err(UbotError::BadInput(_))));
    let mut r = request(&res);
    r.layouts[0].boundary = RingInput { vertices: vec![[0.0, 0.0, 0.0], [f64::NAN, 1.0, 0.0], [1.0, 1.0, 0.0]] };
    assert!(check(&r, &Catalog::builtin()).is_err());
}

#[test]
fn stacked_boxes_hit_the_work_limit_quickly() {
    let r = loose_req((0..30_000).map(|k| loose(&format!("{k:X}"), None, 0.0, 0.0)).collect());
    let t = std::time::Instant::now();
    assert!(matches!(check(&r, &Catalog::builtin()), Err(UbotError::Limit(_))));
    assert!(t.elapsed().as_secs() < 5);
}

#[test]
fn far_apart_boxes_do_not_cost_quadratic_work() {
    // 20 000 non-overlapping boxes in one column: all share the same x extent.
    let r = loose_req((0..20_000).map(|k| loose(&format!("{k:X}"), None, 0.0, f64::from(k) * 600.0)).collect());
    let t = std::time::Instant::now();
    assert!(check(&r, &Catalog::builtin()).unwrap().is_empty());
    assert!(t.elapsed().as_secs() < 10);
}

fn zone_rect(id: &str, ring: RingInput, clearance_mm: Option<f64>) -> ZoneInput {
    ZoneInput { id: id.into(), ring, zone_type: ZoneType::Column, clearance_mm, alt_product_id: None }
}

#[test]
fn many_big_circular_zones_hit_the_vertex_limit_fast() {
    let mut r = loose_req(vec![loose("10", None, 0.0, 0.0)]);
    r.zones = (0..9_000)
        .map(|k| zone_rect(&format!("z{k}"), RingInput { vertices: vec![[-4.0e7, f64::from(k), 1.0], [4.0e7, f64::from(k), 1.0]] }, None))
        .collect();
    let t = std::time::Instant::now();
    assert!(matches!(check(&r, &Catalog::builtin()), Err(UbotError::Limit(_))));
    assert!(t.elapsed().as_secs_f64() < 2.0, "{:?}", t.elapsed());
}

#[test]
fn thousands_of_small_zones_are_looked_up_locally() {
    // 30 000 boxes x 4 000 zones would be 120 M zone visits if every box scanned every zone.
    let boxes = (0..30_000).map(|k| loose(&format!("{k:X}"), None, f64::from(k % 200) * 600.0, f64::from(k / 200) * 600.0)).collect();
    let mut r = loose_req(boxes);
    r.zones = (0..4_000)
        .map(|k| {
            let (x, y) = (f64::from(k % 200) * 600.0 + 290.0, f64::from(k / 200) * 600.0 + 290.0);
            zone_rect(&format!("z{k}"), rect(x, y, x + 20.0, y + 20.0), None)
        })
        .collect();
    assert!(check(&r, &Catalog::builtin()).unwrap().is_empty());
}

#[test]
fn stale_layout_follows_the_zones_that_meet_the_slab() {
    let res = filled();
    let mut r = request(&res);
    r.zones.push(zone_rect("far", rect(1.0e6, 1.0e6, 1.0e6 + 400.0, 1.0e6 + 400.0), None));
    assert!(run(&r).is_empty(), "a zone far from the slab must not stale the layout");
    // In the rib gap between cells (0, 9) and (1, 9): changes the slab's zone set without touching a box.
    r.zones.push(zone_rect("near", rect(600.0, 100.0, 700.0, 200.0), None));
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, IssueCode::StaleLayout);
}

#[test]
fn geometry_hash_ignores_zone_order_and_far_zones() {
    let a = zone_rect("a", rect(600.0, 100.0, 700.0, 200.0), None);
    let b = zone_rect("b", rect(2000.0, 2000.0, 2100.0, 2100.0), None);
    let far = zone_rect("far", rect(1.0e6, 1.0e6, 1.0e6 + 1.0, 1.0e6 + 1.0), None);
    let h = |zones: &[ZoneInput]| geometry_hash(1.0, &square(), &[], zones).unwrap();
    assert_eq!(h(&[a.clone(), b.clone()]), h(&[b.clone(), a.clone()]));
    assert_eq!(h(&[a.clone(), b.clone()]), h(&[far, b.clone(), a.clone()]));
    assert_ne!(h(std::slice::from_ref(&a)), h(&[a, b]));
    // Millimetres and metres stay the same hash.
    let metre = RingInput { vertices: vec![[0.0, 0.0, 0.0], [6.6, 0.0, 0.0], [6.6, 6.6, 0.0], [0.0, 6.6, 0.0]] };
    assert_eq!(geometry_hash(1000.0, &metre, &[], &[]).unwrap(), h(&[]));
}

#[test]
fn duplicate_ids_with_handles_in_reverse_order_and_different_lengths() {
    let r = loose_req(vec![loose("100", Some("x:0:0"), 0.0, 0.0), loose("FF", Some("x:0:0"), 3000.0, 0.0)]);
    // FF is the smaller handle (shorter hex string), so the copy 100 is the extra.
    assert_eq!(run(&r).iter().map(|i| i.handle.clone().unwrap()).collect::<Vec<_>>(), ["100"]);
    let r = loose_req(vec![loose("FF", Some("x:0:0"), 0.0, 0.0), loose("100", Some("x:0:0"), 3000.0, 0.0)]);
    assert_eq!(run(&r).iter().map(|i| i.handle.clone().unwrap()).collect::<Vec<_>>(), ["100"]);
}

#[test]
fn issue_order_breaks_ties_on_x_ascending() {
    let mut r = loose_req(vec![loose("10", None, 2000.0, 0.0), loose("11", None, 0.0, 0.0), loose("12", None, 1000.0, 0.0)]);
    for i in &mut r.instances {
        i.xdata_valid = false;
    }
    let handles: Vec<_> = run(&r).iter().map(|i| i.handle.clone().unwrap()).collect();
    assert_eq!(handles, ["11", "12", "10"]);
}

#[test]
fn rotated_grid_in_a_metre_drawing_is_clean_then_flags_one_moved_box() {
    let metre = RingInput { vertices: vec![[0.0, 0.0, 0.0], [6.6, 0.0, 0.0], [6.6, 6.6, 0.0], [0.0, 6.6, 0.0]] };
    let mut f = fill_req(metre.clone());
    f.unit_scale = 1000.0;
    f.angle_deg = 30.0;
    let res = fill(&f, &Catalog::builtin()).unwrap();
    assert!(res.add.len() > 20);
    let mut r = CheckRequest { unit_scale: 1000.0, instances: instances(&res), layouts: vec![layout_of(&res, metre)], zones: vec![] };
    let issues = run(&r);
    assert!(issues.is_empty(), "{issues:?}");
    r.instances[3].base.insert[0] += 0.005;
    let issues = run(&r);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, IssueCode::ManualMoved);
    assert_eq!(issues[0].handle, Some(r.instances[3].base.handle.clone()));
}

#[test]
fn edge_clearance_shortfall_inside_the_arc_band_is_a_warning() {
    let res = filled();
    let mut r = request(&res);
    r.layouts[0].edge_clearance_mm = Some(70.3);
    let issues = run(&r);
    assert_eq!(issues.len(), 36, "{issues:?}");
    assert!(issues.iter().all(|i| i.code == IssueCode::Clearance && i.severity == Severity::Warning));
    r.layouts[0].edge_clearance_mm = Some(71.0);
    assert!(run(&r).iter().all(|i| i.code == IssueCode::Clearance && i.severity == Severity::Error));
}

#[test]
fn zone_clearance_own_value_and_rulepack_fallback() {
    let res = filled();
    // 20 mm wide zone 50 mm right of cell (0, 0) (x 70..590) and 70 mm left of cell (1, 0) (x 730..1250).
    let cell00 = format!("{LAYOUT}:0:0");
    let run_with = |clearance: Option<f64>, catalog: &Catalog| {
        let mut r = request(&res);
        r.layouts[0].stored_geometry_hash = None;
        r.zones.push(zone_rect("c", rect(640.0, 6100.0, 660.0, 6200.0), clearance));
        let handle = r.instances.iter().find(|i| i.base.instance_id.as_deref() == Some(cell00.as_str())).unwrap().base.handle.clone();
        (check(&r, catalog).unwrap(), handle)
    };
    let (issues, h) = run_with(Some(60.0), &Catalog::builtin());
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].code, issues[0].severity, issues[0].handle.clone()), (IssueCode::Clearance, Severity::Error, Some(h)));
    let (issues, _) = run_with(Some(50.3), &Catalog::builtin());
    assert_eq!((issues.len(), issues[0].severity), (1, Severity::Warning));
    assert!(run_with(Some(50.0), &Catalog::builtin()).0.is_empty());
    let mut rulepack = Catalog::builtin();
    rulepack.rulepacks[0].exclusion_clearance_mm = 60.0;
    let (issues, h2) = run_with(None, &rulepack);
    assert_eq!((issues.len(), issues[0].severity, issues[0].handle.clone()), (1, Severity::Error, Some(h2)));
    assert!(run_with(None, &Catalog::builtin()).0.is_empty());
}

#[test]
fn legacy_block_with_a_mapped_product_gets_a_footprint_and_can_overlap() {
    let mut c = Catalog::builtin();
    c.mappings.push(cadcraft_ubot::BlockMapping {
        product_id: Some("ubot-h13-7".into()),
        cad_engine: "any".into(),
        block_name: "UB-9".into(),
        units_mm_per_block_unit: 1.0,
        anchor: cadcraft_ubot::Anchor::Center,
        rotation_offset_deg: 0.0,
    });
    let mut legacy = loose("11", None, 300.0, 0.0);
    legacy.base.product_id = None;
    legacy.base.block_name = "UB-9".into();
    legacy.legacy = true;
    let issues = check(&loose_req(vec![loose("10", None, 0.0, 0.0), legacy]), &c).unwrap();
    let codes: Vec<_> = issues.iter().map(|i| i.code).collect();
    assert!(codes.contains(&IssueCode::Overlap) && codes.contains(&IssueCode::LegacyBlock), "{issues:?}");
    assert_eq!(issues.len(), 2);
}

#[test]
fn far_broken_zone_does_not_fail_check_but_nan_zone_does() {
    let res = filled();
    let mut r = request(&res);
    let far = 6600.0 + 10_000.0 + 1000.0;
    let bow = RingInput { vertices: vec![[far, 0.0, 0.0], [far + 1000.0, 1000.0, 0.0], [far + 1000.0, 0.0, 0.0], [far, 1000.0, 0.0]] };
    r.zones.push(ZoneInput { id: "broken".into(), ring: bow, zone_type: ZoneType::Exclusion, clearance_mm: None, alt_product_id: None });
    assert!(run(&r).is_empty());
    r.zones.push(ZoneInput {
        id: "nan".into(),
        ring: rect(f64::NAN, 0.0, 1.0, 1.0),
        zone_type: ZoneType::Exclusion,
        clearance_mm: None,
        alt_product_id: None,
    });
    let err = check(&r, &Catalog::builtin()).unwrap_err();
    assert!(matches!(&err, UbotError::BadInput(m) if m.contains("zone nan")), "{err}");
}

fn far_zones(n: u32, vertices_each: u32) -> Vec<ZoneInput> {
    (0..n)
        .map(|k| {
            let (cx, cy) = (5.0e6 + f64::from(k) * 3000.0, 5.0e6);
            let ring = (0..vertices_each)
                .map(|v| f64::from(v) * std::f64::consts::TAU / f64::from(vertices_each))
                .map(|a| [cx + 1000.0 * a.cos(), cy + 1000.0 * a.sin(), 0.0])
                .collect();
            zone_rect(&format!("f{k}"), RingInput { vertices: ring }, None)
        })
        .collect()
}

/// `n` small slabs 20 m apart, each with a stored geometry hash that the far zones do not change.
fn many_layouts(n: u32) -> Vec<LayoutInput> {
    (0..n)
        .map(|k| {
            let x = f64::from(k % 40) * 20_000.0;
            let y = f64::from(k / 40) * 20_000.0;
            let boundary = rect(x, y, x + 6600.0, y + 6600.0);
            LayoutInput {
                layout_id: format!("{k:016x}"),
                boundary_handle: format!("{:X}", 0x1000 + k),
                stored_geometry_hash: Some(geometry_hash(1.0, &boundary, &[], &[]).unwrap()),
                boundary,
                holes: vec![],
                layout: None,
                edge_clearance_mm: None,
            }
        })
        .collect()
}

#[test]
fn many_layouts_and_far_zones_cost_little() {
    let r = CheckRequest { unit_scale: 1.0, instances: vec![], layouts: many_layouts(50), zones: far_zones(200, 50) };
    let t = std::time::Instant::now();
    assert!(run(&r).is_empty());
    assert!(t.elapsed().as_secs_f64() < 2.0, "{:?}", t.elapsed());
}

#[test]
#[ignore = "perf: run with --release -- --ignored --nocapture"]
fn thousand_layouts_and_two_hundred_far_zones_perf() {
    let r = CheckRequest { unit_scale: 1.0, instances: vec![], layouts: many_layouts(1_000), zones: far_zones(200, 500) };
    let t = std::time::Instant::now();
    assert!(run(&r).is_empty());
    let took = t.elapsed();
    eprintln!("check: 1 000 layouts x 200 far zones of 500 vertices: {took:?}");
    assert!(took.as_secs_f64() < 0.5, "{took:?}");
}

#[test]
fn over_a_million_raw_zone_vertices_is_a_limit() {
    let res = filled();
    let mut r = request(&res);
    let ring = (0..1_000_001u32).map(|v| [1.0e7 + f64::from(v % 1000), 1.0e7 + f64::from(v / 1000), 0.0]).collect();
    r.zones.push(zone_rect("huge", RingInput { vertices: ring }, None));
    let err = check(&r, &Catalog::builtin()).unwrap_err();
    assert!(matches!(&err, UbotError::Limit(m) if m.contains("1000000")), "{err}");
}

#[test]
fn loose_box_in_a_far_zone_is_in_zone_with_or_without_layouts() {
    let at = 5.0e6;
    let zone = zone_rect("z", rect(at - 1000.0, at - 1000.0, at + 1000.0, at + 1000.0), None);
    let mut r = loose_req(vec![loose("10", None, at, at)]);
    r.zones.push(zone.clone());
    assert_eq!(with_code(&run(&r), IssueCode::InZone).len(), 1, "no layouts");
    let res = filled();
    let mut r = request(&res);
    r.instances.push(loose("FFFF", None, at, at));
    r.zones.push(zone);
    let issues = run(&r);
    assert_eq!(with_code(&issues, IssueCode::InZone).len(), 1, "{issues:?}");
    assert!(with_code(&issues, IssueCode::StaleLayout).is_empty(), "the far zone does not reach the slab");
}

#[test]
fn stale_check_hashes_the_same_zones_as_geometry_hash() {
    // Near zones (one with arcs, one only reached through its arc bulge), far zones and a hole, in cm.
    let us = 10.0;
    let boundary = rect(0.0, 0.0, 660.0, 660.0);
    let holes = vec![rect(300.0, 300.0, 320.0, 320.0)];
    let zones = vec![
        zone_rect("near", rect(60.0, 10.0, 70.0, 20.0), None),
        zone_rect("arc", RingInput { vertices: vec![[200.0, 200.0, 1.0], [220.0, 200.0, 1.0]] }, Some(50.0)),
        zone_rect("bulge", RingInput { vertices: vec![[1900.0, 300.0, 0.0], [1910.0, 300.0, 50.0]] }, None),
        zone_rect("far", rect(1.0e5, 1.0e5, 1.0e5 + 10.0, 1.0e5 + 10.0), None),
    ];
    let layout = LayoutInput {
        layout_id: LAYOUT.into(),
        boundary_handle: "A1".into(),
        stored_geometry_hash: Some(geometry_hash(us, &boundary, &holes, &zones).unwrap()),
        boundary,
        holes,
        layout: None,
        edge_clearance_mm: None,
    };
    let r = CheckRequest { unit_scale: us, instances: vec![], layouts: vec![layout], zones };
    let issues = run(&r);
    assert!(with_code(&issues, IssueCode::StaleLayout).is_empty(), "{issues:?}");
}
