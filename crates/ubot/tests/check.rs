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
    let issues = run(&r);
    assert!(!with_code(&issues, IssueCode::OutsideBoundary).is_empty(), "{issues:?}");
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
