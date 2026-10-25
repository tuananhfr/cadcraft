//! Hand-computed grid fills. Box 520×520, rib 140 → pitch 660. Default origin = bbox top-left + (330, −330), so on the
//! 0..6600 square cell (i, j) has centre (330 + 660i, 6270 − 660j) and footprint x ∈ [70 + 660i, 590 + 660i],
//! y ∈ [6010 − 660j, 6530 − 660j]: every edge box sits 70 mm from the boundary.

use cadcraft_geom::Vec2;
use cadcraft_ubot::{
    Anchor, ApprovalStatus, BlockMapping, Candidate, Catalog, ExistingInstance, FillRequest, FillResult, MAX_CLEARANCE_MM, Product, Reason,
    RingInput, Status, UbotError, ZoneInput, ZoneType, distance_footprint_ring_edges, fill, footprint, footprint_intersects_ring, geometry_hash,
    ring_contains_footprint, ring_from_input,
};

const LAYOUT: &str = "0123456789abcdef";
const OTHER: &str = "ffffffffffffffff";

fn poly(pts: &[[f64; 2]]) -> RingInput {
    RingInput { vertices: pts.iter().map(|p| [p[0], p[1], 0.0]).collect() }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> RingInput {
    poly(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

fn square() -> RingInput {
    rect(0.0, 0.0, 6600.0, 6600.0)
}

fn circle(cx: f64, cy: f64, r: f64) -> RingInput {
    RingInput { vertices: vec![[cx - r, cy, 1.0], [cx + r, cy, 1.0]] }
}

fn zone(id: &str, ring: RingInput, zone_type: ZoneType) -> ZoneInput {
    ZoneInput { id: id.into(), ring, zone_type, clearance_mm: None, alt_product_id: None }
}

fn req(boundary: RingInput) -> FillRequest {
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

fn run(r: &FillRequest) -> FillResult {
    fill(r, &Catalog::builtin()).unwrap()
}

/// Turns the `add` list of a run into the boxes a CAD adapter would have written, handles `100`, `101`, … (hex).
fn as_existing(res: &FillResult) -> Vec<ExistingInstance> {
    res.add
        .iter()
        .enumerate()
        .map(|(k, p)| ExistingInstance {
            handle: format!("{:X}", 0x100 + k),
            instance_id: Some(p.instance_id.clone()),
            layout_id: Some(res.layout.layout_id.clone()),
            product_id: Some(p.product_id.clone()),
            block_name: p.block_name.clone(),
            insert: p.insert,
            rotation_deg: p.rotation_deg,
            scale: 1.0,
        })
        .collect()
}

fn id(i: i64, j: i64) -> String {
    format!("{LAYOUT}:{i}:{j}")
}

fn cells_of(c: &[Candidate]) -> Vec<[i64; 2]> {
    c.iter().map(|p| p.cell).collect()
}

fn assert_near(a: [f64; 2], b: [f64; 2], tol: f64) {
    assert!((a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol, "{a:?} != {b:?}");
}

fn all_have(problems: &[Candidate], status: Status, reasons: &[Reason]) {
    for p in problems {
        assert_eq!(p.status, status, "{p:?}");
        assert_eq!(p.reasons, reasons, "{p:?}");
    }
}

#[test]
fn rect_6600_square_gives_100_boxes() {
    let res = run(&req(square()));
    assert_eq!(res.add.len(), 100);
    assert!(res.problems.is_empty(), "{:?}", res.problems);
    assert_eq!((res.summary.valid, res.summary.invalid, res.summary.uncertain, res.summary.added), (100, 0, 0, 100));
    let first = &res.add[0];
    assert_eq!(first.instance_id, id(0, 0));
    assert_eq!(first.cell, [0, 0]);
    assert_eq!(first.product_id, "ubot-h13-7");
    assert_eq!(first.block_name, "UBOT_H13_7");
    assert_near(first.center, [330.0, 6270.0], 1e-9);
    assert_near(first.insert, [330.0, 6270.0], 1e-9);
    assert_eq!(first.rotation_deg, 0.0);
    let last = &res.add[99];
    assert_eq!(last.instance_id, id(9, 9));
    assert_near(last.center, [6270.0, 330.0], 1e-9);
    assert_eq!(res.layout.layout_id, LAYOUT);
    assert_eq!(res.layout.rulepack, "lpc-draft-1@1");
    assert_eq!((res.layout.pitch_x_mm, res.layout.pitch_y_mm), (660.0, 660.0));
    assert_near(res.layout.origin, [330.0, 6270.0], 1e-9);
    assert_eq!(res.layout.revision, 0);
    assert!(res.warnings.contains(&"RULEPACK_DRAFT lpc-draft-1@1".to_string()), "{:?}", res.warnings);
    assert!(res.warnings.contains(&"PRODUCT_DRAFT ubot-h13-7".to_string()), "{:?}", res.warnings);
}

#[test]
fn rect_respects_edge_clearance() {
    // Edge boxes are 70 mm from the edge; 100 mm clearance short by 30 > 0.5 → the outer ring of 36 cells is invalid.
    let mut r = req(square());
    r.edge_clearance_mm = Some(100.0);
    let res = run(&r);
    assert_eq!(res.add.len(), 64);
    assert_eq!(res.problems.len(), 36);
    all_have(&res.problems, Status::Invalid, &[Reason::EdgeClearance]);
    for p in &res.problems {
        assert!(p.cell[0] == 0 || p.cell[0] == 9 || p.cell[1] == 0 || p.cell[1] == 9, "{p:?}");
    }
    for a in &res.add {
        assert!((1..=8).contains(&a.cell[0]) && (1..=8).contains(&a.cell[1]));
    }
    assert_eq!(res.summary.invalid, 36);
}

#[test]
fn edge_clearance_equal_is_valid() {
    let mut r = req(square());
    r.edge_clearance_mm = Some(70.0);
    let res = run(&r);
    assert_eq!(res.add.len(), 100);
    assert!(res.problems.is_empty(), "{:?}", res.problems);
}

#[test]
fn near_limit_within_half_mm_is_uncertain() {
    // 70.3 − 70 = 0.3 ≤ 0.5 → uncertain; 70.6 − 70 = 0.6 > 0.5 → invalid.
    let mut r = req(square());
    r.edge_clearance_mm = Some(70.3);
    let res = run(&r);
    assert_eq!(res.add.len(), 64);
    assert_eq!(res.problems.len(), 36);
    all_have(&res.problems, Status::Uncertain, &[Reason::NearLimit]);
    assert_eq!((res.summary.uncertain, res.summary.invalid), (36, 0));
    r.edge_clearance_mm = Some(70.6);
    let res = run(&r);
    assert_eq!(res.add.len(), 64);
    all_have(&res.problems, Status::Invalid, &[Reason::EdgeClearance]);
}

#[test]
fn l_shape_counts() {
    // Upper half (y > 3300) only spans x 0..3300: rows j = 0..4 hold i = 0..4 (5 each); rows j = 5..9 hold 10 each.
    let l = poly(&[[0.0, 0.0], [6600.0, 0.0], [6600.0, 3300.0], [3300.0, 3300.0], [3300.0, 6600.0], [0.0, 6600.0]]);
    let res = run(&req(l));
    assert_eq!(res.add.len(), 75);
    for j in 0..10 {
        let n = res.add.iter().filter(|p| p.cell[1] == j).count();
        assert_eq!(n, if j < 5 { 5 } else { 10 }, "row {j}");
    }
    assert!(res.problems.is_empty(), "{:?}", res.problems);
}

#[test]
fn skew_edge_marks_partial_cells_invalid() {
    // Top edge y = 3300 + x/2. Cell (i, j) is contained when 6530 − 660j ≤ 3335 + 330i (top-left corner under the edge)
    // and wholly outside when 6010 − 660j > 3595 + 330i. Per column i = 0..9 that leaves one crossing cell at
    // j = 4,4,3,3,2,2,1,1,0,0 and 5,5,6,6,7,7,8,8,9,9 contained cells (70 in all).
    let skew = poly(&[[0.0, 0.0], [6600.0, 0.0], [6600.0, 6600.0], [0.0, 3300.0]]);
    let res = run(&req(skew.clone()));
    assert_eq!(res.add.len(), 70);
    assert_eq!(cells_of(&res.problems), [[8, 0], [9, 0], [6, 1], [7, 1], [4, 2], [5, 2], [2, 3], [3, 3], [0, 4], [1, 4]]);
    all_have(&res.problems, Status::Invalid, &[Reason::OutsideBoundary]);
    let ring = ring_from_input(&skew, 1.0, 0.5).unwrap();
    for a in &res.add {
        let fp = footprint(Vec2::new(a.center[0], a.center[1]), 520.0, 520.0, 0.0);
        assert!(ring_contains_footprint(&ring, &fp), "{a:?}");
        // Straight edges are 70 mm away; the closest contained boxes are 105 mm under the skew edge vertically,
        // 105·cos(26.57°) ≈ 93.9 mm away, so nothing touches it.
        assert!(distance_footprint_ring_edges(&fp, &ring).0 >= 70.0 - 1e-9, "{a:?}");
    }
}

#[test]
fn hole_and_column_zone_excluded() {
    // Hole 2640..3960 holds cells i, j ∈ {4, 5}. Column 1120..1520 sits on the rib crossing of i ∈ {1, 2}, j ∈ {7, 8}.
    // A second column 5410..5810 lies wholly inside the footprint of cell (8, 1).
    let mut r = req(square());
    r.holes = vec![rect(2640.0, 2640.0, 3960.0, 3960.0)];
    r.zones =
        vec![zone("c1", rect(1120.0, 1120.0, 1520.0, 1520.0), ZoneType::Column), zone("c2", rect(5410.0, 5410.0, 5810.0, 5810.0), ZoneType::Column)];
    let res = run(&r);
    assert_eq!(res.add.len(), 91);
    assert_eq!(cells_of(&res.problems), [[8, 1], [4, 4], [5, 4], [4, 5], [5, 5], [1, 7], [2, 7], [1, 8], [2, 8]]);
    for p in &res.problems {
        let expect = if (4..=5).contains(&p.cell[0]) { Reason::InHole } else { Reason::InZone };
        assert_eq!((p.status, p.reasons.as_slice()), (Status::Invalid, [expect].as_slice()), "{p:?}");
    }
}

#[test]
fn opening_zone_counts_as_hole() {
    let mut r = req(square());
    r.zones = vec![zone("o", rect(2640.0, 2640.0, 3960.0, 3960.0), ZoneType::Opening)];
    let res = run(&r);
    assert_eq!(res.add.len(), 96);
    all_have(&res.problems, Status::Invalid, &[Reason::InHole]);
}

#[test]
fn zone_clearance_uses_zone_then_request_then_rulepack() {
    // Column 1120..1520: the cells beside the four cut ones are 530 mm away (e.g. (0, 7) ends at x = 590, (3, 7) starts at
    // x = 2050, (1, 6) starts at y = 2050).
    let mut r = req(square());
    let mut z = zone("c", rect(1120.0, 1120.0, 1520.0, 1520.0), ZoneType::Column);
    z.clearance_mm = Some(600.0);
    r.zones = vec![z];
    let res = run(&r);
    // Cut cells (1,7),(2,7),(1,8),(2,8) are InZone; cells 530 mm away (x or y) violate 600: (0,7),(0,8),(3,7),(3,8),
    // (1,6),(2,6),(1,9),(2,9). Diagonal neighbours such as (0,6) are √(530²+530²) ≈ 749.5 mm away → valid.
    let zone_clear: Vec<[i64; 2]> = res.problems.iter().filter(|p| p.reasons == [Reason::ZoneClearance]).map(|p| p.cell).collect();
    assert_eq!(zone_clear, [[1, 6], [2, 6], [0, 7], [3, 7], [0, 8], [3, 8], [1, 9], [2, 9]]);
    assert_eq!(res.problems.iter().filter(|p| p.reasons == [Reason::InZone]).count(), 4);
    // Request-level exclusion clearance applies when the zone has none.
    r.zones[0].clearance_mm = None;
    r.exclusion_clearance_mm = Some(600.0);
    assert_eq!(run(&r).problems.len(), 12);
    r.exclusion_clearance_mm = None;
    assert_eq!(run(&r).problems.len(), 4);
}

#[test]
fn curved_edge_cells_near_arc_are_uncertain() {
    // Concave notch in the right edge: two quarter arcs of radius 69.8 around (6600, 6270) meeting at (6530.2, 6270), an
    // exact vertex. Cell (9, 0) ends at x = 6530, so it is 0.2 mm from an arc edge → uncertain; everything else is ≥ 70 mm.
    let b = (std::f64::consts::PI / 8.0).tan();
    let notch = |r: f64| RingInput {
        vertices: vec![
            [0.0, 0.0, 0.0],
            [6600.0, 0.0, 0.0],
            [6600.0, 6270.0 - r, -b],
            [6600.0 - r, 6270.0, -b],
            [6600.0, 6270.0 + r, 0.0],
            [6600.0, 6600.0, 0.0],
            [0.0, 6600.0, 0.0],
        ],
    };
    let res = run(&req(notch(69.8)));
    assert_eq!(res.add.len(), 99);
    assert_eq!(cells_of(&res.problems), [[9, 0]]);
    all_have(&res.problems, Status::Uncertain, &[Reason::CurvedEdgeTolerance]);
    assert_eq!(res.summary.uncertain, 1);
    // 0.7 mm away is outside the 0.5 mm tessellation band.
    let res = run(&req(notch(69.3)));
    assert_eq!(res.add.len(), 100);
    assert!(res.problems.is_empty(), "{:?}", res.problems);
    // Circular hole whose left vertex (590.2, 6270) is 0.2 mm right of cell (0, 0); cell (1, 0) starts at 730 (39.8 mm).
    let mut r = req(square());
    r.holes = vec![circle(640.2, 6270.0, 50.0)];
    let res = run(&r);
    assert_eq!(res.add.len(), 99);
    assert_eq!(cells_of(&res.problems), [[0, 0]]);
    all_have(&res.problems, Status::Uncertain, &[Reason::CurvedEdgeTolerance]);
}

#[test]
fn alt_product_zone_switches_product() {
    // Alt zone 220..2420 holds centres i ∈ 0..=3 (330, 990, 1650, 2310), j ∈ 6..=9; minus the 4 column cells → 12 boxes.
    let mut r = req(square());
    let mut alt = zone("alt", rect(220.0, 220.0, 2420.0, 2420.0), ZoneType::AltProduct);
    alt.alt_product_id = Some("ubot-h10-7".into());
    r.zones = vec![zone("c", rect(1120.0, 1120.0, 1520.0, 1520.0), ZoneType::Column), alt];
    let res = run(&r);
    assert_eq!(res.add.len(), 96);
    let alt_cells: Vec<[i64; 2]> = res.add.iter().filter(|p| p.product_id == "ubot-h10-7").map(|p| p.cell).collect();
    assert_eq!(alt_cells, [[0, 6], [1, 6], [2, 6], [3, 6], [0, 7], [3, 7], [0, 8], [3, 8], [0, 9], [1, 9], [2, 9], [3, 9]]);
    assert!(res.add.iter().filter(|p| p.product_id == "ubot-h10-7").all(|p| p.block_name == "UBOT_H10_7"));
    assert_eq!(res.add.iter().filter(|p| p.product_id == "ubot-h13-7").count(), 84);
    assert!(res.warnings.contains(&"PRODUCT_DRAFT ubot-h10-7".to_string()), "{:?}", res.warnings);
}

fn catalog_with(products: Vec<Product>, mappings: Vec<BlockMapping>) -> Catalog {
    let builtin = Catalog::builtin();
    let extra = Catalog { products, mappings, rulepacks: vec![], ..builtin.clone() };
    builtin.overlay(&extra)
}

fn product(id: &str, w: f64, l: f64) -> Product {
    Product {
        id: id.into(),
        sku: id.to_uppercase(),
        name: id.into(),
        manufacturer: None,
        width_mm: w,
        length_mm: l,
        height_mm: None,
        slab_thickness_mm: None,
        version: "1".into(),
        approval_status: ApprovalStatus::Approved,
        source_ref: None,
    }
}

#[test]
fn alt_product_with_different_size_is_rejected() {
    let catalog = catalog_with(vec![product("ubot-wide", 600.0, 520.0)], vec![]);
    let mut r = req(square());
    let mut alt = zone("alt", rect(220.0, 220.0, 2420.0, 2420.0), ZoneType::AltProduct);
    alt.alt_product_id = Some("ubot-wide".into());
    r.zones = vec![alt];
    assert!(matches!(fill(&r, &catalog), Err(UbotError::BadInput(_))));
    r.zones[0].alt_product_id = Some("nope".into());
    assert!(matches!(fill(&r, &catalog), Err(UbotError::UnknownProduct(_))));
    r.zones[0].alt_product_id = None;
    assert!(matches!(fill(&r, &catalog), Err(UbotError::BadInput(_))));
}

fn rotate(p: [f64; 2], deg: f64) -> [f64; 2] {
    let v = Vec2::new(p[0], p[1]).rotate(deg.to_radians());
    [v.x, v.y]
}

#[test]
fn rotation_30_degrees_keeps_count_of_large_square() {
    let corners = [[0.0, 0.0], [6600.0, 0.0], [6600.0, 6600.0], [0.0, 6600.0]].map(|p| rotate(p, 30.0));
    let mut r = req(poly(&corners));
    r.angle_deg = 30.0;
    let res = run(&r);
    assert_eq!(res.add.len(), 100);
    assert!(res.problems.is_empty(), "{:?}", res.problems);
    assert_near(res.add[0].center, rotate([330.0, 6270.0], 30.0), 1e-6);
    assert_near(res.add[99].center, rotate([6270.0, 330.0], 30.0), 1e-6);
    assert!(res.add.iter().all(|p| p.rotation_deg == 30.0));
}

#[test]
fn units_m_and_mm_same_result() {
    let mm = run(&req(square()));
    let mut r = req(rect(0.0, 0.0, 6.6, 6.6));
    r.unit_scale = 1000.0;
    let m = run(&r);
    assert_eq!(m.add.len(), 100);
    let ids_mm: Vec<&str> = mm.add.iter().map(|p| p.instance_id.as_str()).collect();
    let ids_m: Vec<&str> = m.add.iter().map(|p| p.instance_id.as_str()).collect();
    assert_eq!(ids_mm, ids_m);
    for (a, b) in mm.add.iter().zip(&m.add) {
        assert_near(a.insert, [b.insert[0] * 1000.0, b.insert[1] * 1000.0], 1e-6);
        assert_near(a.center, [b.center[0] * 1000.0, b.center[1] * 1000.0], 1e-6);
    }
    assert_near(m.layout.origin, [0.33, 6.27], 1e-9);
    assert_eq!(m.layout.unit_scale, 1000.0);
    assert_eq!(mm.layout.geometry_hash, m.layout.geometry_hash);
    assert_ne!(mm.input_hash, m.input_hash);
}

#[test]
fn geometry_hash_is_public_and_unit_normalised() {
    let a = geometry_hash(1.0, &square(), &[], &[]).unwrap();
    let b = geometry_hash(1000.0, &rect(0.0, 0.0, 6.6, 6.6), &[], &[]).unwrap();
    assert_eq!(a, b);
    let c = geometry_hash(1.0, &square(), &[rect(1.0, 1.0, 2.0, 2.0)], &[]).unwrap();
    assert_ne!(a, c);
    let z = [zone("c", rect(1.0, 1.0, 2.0, 2.0), ZoneType::Column)];
    let d = geometry_hash(1.0, &square(), &[], &z).unwrap();
    assert_ne!(a, d);
    assert!(matches!(geometry_hash(f64::NAN, &square(), &[], &[]), Err(UbotError::BadInput(_))));
    assert_eq!(run(&req(square())).layout.geometry_hash, a);
}

#[test]
fn rerun_with_existing_is_all_keep() {
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    let second = run(&r);
    assert_eq!((second.add.len(), second.update.len(), second.remove.len(), second.keep.len()), (0, 0, 0, 100));
    assert_eq!(second.input_hash, first.input_hash);
    assert!(second.problems.is_empty(), "{:?}", second.problems);
    assert_eq!(second.keep[0], "100");
    assert_eq!((second.summary.kept, second.summary.added, second.summary.valid), (100, 0, 100));
}

#[test]
fn shifted_origin_updates_and_removes() {
    // Origin one pitch left: cell i now centres at −330 + 660i. Cell 0 is wholly outside, cells 1..=10 are valid.
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    r.origin = Some([330.0 - 660.0, 6270.0]);
    r.reset = true;
    let res = run(&r);
    // Boxes 1..=9 of every row move to the new cell centres, box 0 goes, cell 10 is new.
    assert_eq!((res.update.len(), res.remove.len(), res.add.len(), res.keep.len()), (90, 10, 10, 0));
    assert!(res.update.iter().all(|u| u.placement.cell[0] >= 1 && u.placement.cell[0] <= 9));
    let u = res.update.iter().find(|u| u.placement.instance_id == id(1, 0)).unwrap();
    assert_eq!(u.handle, "101");
    assert_near(u.placement.center, [330.0, 6270.0], 1e-9);
    assert!(res.add.iter().all(|p| p.cell[0] == 10));
    let mut ids: Vec<&str> =
        res.update.iter().map(|u| u.placement.instance_id.as_str()).chain(res.add.iter().map(|p| p.instance_id.as_str())).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "instance ids must be unique");
    let removed: Vec<&str> = res.remove.iter().map(String::as_str).collect();
    let expect: Vec<String> = (0..10).map(|j| format!("{:X}", 0x100 + 10 * j)).collect();
    assert_eq!(removed, expect.iter().map(String::as_str).collect::<Vec<_>>());

    // Without reset the shifted boxes stay put (manually moved relative to the new grid) and block the cell they occupy.
    r.reset = false;
    let res = run(&r);
    assert_eq!((res.update.len(), res.remove.len(), res.add.len(), res.keep.len()), (0, 10, 0, 90));
    let moved = res.problems.iter().filter(|p| p.status == Status::Info && p.reasons == [Reason::ManualMoved]).count();
    assert_eq!(moved, 90);
    let blocked: Vec<[i64; 2]> = res.problems.iter().filter(|p| p.reasons == [Reason::OverlapsExisting]).map(|p| p.cell).collect();
    assert_eq!(blocked, (0..10).map(|j| [10, j]).collect::<Vec<_>>());
    assert_eq!(res.summary.valid, 90);
}

#[test]
fn manual_move_is_kept_and_reported() {
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    let k = first.add.iter().position(|p| p.instance_id == id(3, 2)).unwrap();
    r.existing[k].insert[0] += 50.0;
    let res = run(&r);
    assert_eq!((res.keep.len(), res.update.len(), res.add.len(), res.remove.len()), (100, 0, 0, 0));
    assert_eq!(cells_of(&res.problems), [[3, 2]]);
    all_have(&res.problems, Status::Info, &[Reason::ManualMoved]);
    assert_eq!((res.summary.valid, res.summary.invalid, res.summary.uncertain), (100, 0, 0));

    r.reset = true;
    let res = run(&r);
    assert_eq!((res.keep.len(), res.update.len()), (99, 1));
    assert_eq!(res.update[0].handle, r.existing[k].handle);
    assert_eq!(res.update[0].placement.instance_id, id(3, 2));
    assert_near(res.update[0].placement.insert, [330.0 + 3.0 * 660.0, 6270.0 - 2.0 * 660.0], 1e-9);
    assert!(res.problems.is_empty());

    // A turned box is moved too.
    r.reset = false;
    r.existing[k].insert[0] -= 50.0;
    r.existing[k].rotation_deg = 90.0;
    let res = run(&r);
    assert_eq!(cells_of(&res.problems), [[3, 2]]);
    // Turning by a full 360° is not a move.
    r.existing[k].rotation_deg = 360.0;
    assert!(run(&r).problems.is_empty());
}

#[test]
fn move_threshold_is_one_mm() {
    // Spec §14.1: a box counts as moved when it is more than 1 mm off its cell.
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    r.existing[7].insert[1] += 0.5;
    let res = run(&r);
    assert_eq!(res.keep.len(), 100);
    assert!(res.problems.is_empty(), "{:?}", res.problems);
    r.existing[7].insert[1] += 1.0;
    let res = run(&r);
    assert_eq!(res.keep.len(), 100);
    assert_eq!(cells_of(&res.problems), [[7, 0]]);
    all_have(&res.problems, Status::Info, &[Reason::ManualMoved]);
}

#[test]
fn copied_floor_previous_layout_is_rehomed() {
    const OLD: &str = "aaaaaaaaaaaaaaaa";
    const NEW: &str = "bbbbbbbbbbbbbbbb";
    let mut r0 = req(square());
    r0.layout_id = OLD.into();
    let original = run(&r0);
    let mut originals = as_existing(&original);
    let mut copies = as_existing(&original);
    for (k, c) in copies.iter_mut().enumerate() {
        c.handle = format!("{:X}", 0x900 + k);
        c.insert[0] += 10_000.0;
    }
    // A box of the old layout outside the new boundary whose footprint (9640..10160) overlaps new cell (0, 0) (10070..10590).
    originals.push(ExistingInstance {
        handle: "8FF".into(),
        instance_id: Some(format!("{OLD}:-1:0")),
        layout_id: Some(OLD.into()),
        product_id: Some("ubot-h13-7".into()),
        block_name: "UBOT_H13_7".into(),
        insert: [9900.0, 6270.0],
        rotation_deg: 0.0,
        scale: 1.0,
    });
    let mut r = req(rect(10_000.0, 0.0, 16_600.0, 6600.0));
    r.layout_id = NEW.into();
    r.previous_layout_id = Some(OLD.into());
    r.existing = originals.iter().chain(&copies).cloned().collect();
    let res = run(&r);
    assert_eq!((res.update.len(), res.remove.len(), res.add.len(), res.keep.len()), (99, 1, 0, 0));
    assert_eq!(res.remove, ["900"]);
    assert_eq!(cells_of(&res.problems), [[0, 0]]);
    all_have(&res.problems, Status::Invalid, &[Reason::OverlapsExisting]);
    for u in &res.update {
        assert!(u.placement.instance_id.starts_with(NEW), "{u:?}");
        let src = copies.iter().find(|c| c.handle == u.handle).unwrap();
        assert_near(u.placement.insert, src.insert, 1e-9);
    }
    let touched: Vec<&str> = res.update.iter().map(|u| u.handle.as_str()).chain(res.remove.iter().map(String::as_str)).collect();
    assert!(originals.iter().all(|o| !touched.contains(&o.handle.as_str())));
}

#[test]
fn deleted_cell_is_added_back() {
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first).into_iter().filter(|e| e.instance_id.as_deref() != Some(id(4, 4).as_str())).collect();
    let res = run(&r);
    assert_eq!((res.add.len(), res.keep.len(), res.remove.len()), (1, 99, 0));
    assert_eq!(res.add[0].instance_id, id(4, 4));
}

#[test]
fn duplicate_cell_keeps_first_handle() {
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    let mut dup = r.existing[0].clone();
    dup.handle = "FF".into();
    r.existing.push(dup);
    let res = run(&r);
    // "FF" < "100" in handle order (shorter hex first), so the original "100" is the extra copy.
    assert_eq!(res.remove, ["100"]);
    assert_eq!(res.keep.len(), 100);
    assert!(res.keep.contains(&"FF".to_string()));
    assert!(res.warnings.contains(&format!("DUPLICATE_CELL {}", id(0, 0))), "{:?}", res.warnings);
}

#[test]
fn removed_when_cell_becomes_invalid() {
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    r.zones = vec![zone("c", rect(1120.0, 1120.0, 1520.0, 1520.0), ZoneType::Column)];
    let res = run(&r);
    assert_eq!((res.remove.len(), res.keep.len()), (4, 96));
}

#[test]
fn uncertain_cell_keeps_existing_box() {
    let first = run(&req(square()));
    let mut r = req(square());
    r.existing = as_existing(&first);
    r.edge_clearance_mm = Some(70.3);
    let res = run(&r);
    assert_eq!((res.keep.len(), res.remove.len(), res.add.len()), (100, 0, 0));
    assert_eq!(res.summary.uncertain, 36);
    // Uncertain cells are never filled.
    let mut empty = req(square());
    empty.edge_clearance_mm = Some(70.3);
    assert_eq!(run(&empty).add.len(), 64);
}

#[test]
fn circle_zone_two_bulge_ring_excludes_cells() {
    // r = 300 around the rib crossing (1320, 1320) reaches the corners of 4 boxes (99 mm away); r = 60 at (3960, 3960)
    // stays 99 − 60 ≈ 39 mm clear of every box.
    let mut r = req(square());
    r.zones = vec![zone("big", circle(1320.0, 1320.0, 300.0), ZoneType::Column), zone("small", circle(3960.0, 3960.0, 60.0), ZoneType::Column)];
    let res = run(&r);
    assert_eq!(res.add.len(), 96);
    assert_eq!(cells_of(&res.problems), [[1, 7], [2, 7], [1, 8], [2, 8]]);
    all_have(&res.problems, Status::Invalid, &[Reason::InZone]);
}

#[test]
fn other_layout_instances_block_cells() {
    let mut r = req(square());
    r.existing = vec![
        // Box of another layout exactly on cell (0, 0).
        ExistingInstance {
            handle: "A1".into(),
            instance_id: Some(format!("{OTHER}:0:0")),
            layout_id: Some(OTHER.into()),
            product_id: Some("ubot-h13-7".into()),
            block_name: "UBOT_H13_7".into(),
            insert: [330.0, 6270.0],
            rotation_deg: 0.0,
            scale: 1.0,
        },
        // Untagged legacy UB-1 (top_center anchor): insert (990, 6530) → centre (990, 6270) = cell (1, 0).
        ExistingInstance {
            handle: "A2".into(),
            instance_id: None,
            layout_id: None,
            product_id: None,
            block_name: "UB-1".into(),
            insert: [990.0, 6530.0],
            rotation_deg: 0.0,
            scale: 1.0,
        },
        // Our own layout id but outside the boundary: not ours, and it overlaps nothing.
        ExistingInstance {
            handle: "A3".into(),
            instance_id: Some(id(5, 5)),
            layout_id: Some(LAYOUT.into()),
            product_id: Some("ubot-h13-7".into()),
            block_name: "UBOT_H13_7".into(),
            insert: [-5000.0, -5000.0],
            rotation_deg: 0.0,
            scale: 1.0,
        },
        // Flush against cell (9, 9)'s right side (6530..7050): touching is not overlapping.
        ExistingInstance {
            handle: "A4".into(),
            instance_id: None,
            layout_id: None,
            product_id: Some("ubot-h13-7".into()),
            block_name: "UBOT_H13_7".into(),
            insert: [6790.0, 330.0],
            rotation_deg: 0.0,
            scale: 1.0,
        },
    ];
    let res = run(&r);
    assert_eq!(res.add.len(), 98);
    assert_eq!(cells_of(&res.problems), [[0, 0], [1, 0]]);
    all_have(&res.problems, Status::Invalid, &[Reason::OverlapsExisting]);
    assert!(res.keep.is_empty() && res.update.is_empty() && res.remove.is_empty());
    // The (5, 5) id belongs to an outside box, so the cell is simply added.
    assert!(res.add.iter().any(|p| p.instance_id == id(5, 5)));
}

#[test]
fn legacy_top_center_anchor_offsets_insert() {
    let ub1 = BlockMapping {
        product_id: Some("ubot-h13-7".into()),
        cad_engine: "any".into(),
        block_name: "UB-1".into(),
        units_mm_per_block_unit: 1.0,
        anchor: Anchor::TopCenter,
        rotation_offset_deg: 0.0,
    };
    let catalog = catalog_with(vec![], vec![ub1]);
    let res = fill(&req(square()), &catalog).unwrap();
    assert_eq!(res.add[0].block_name, "UB-1");
    assert_near(res.add[0].center, [330.0, 6270.0], 1e-9);
    assert_near(res.add[0].insert, [330.0, 6530.0], 1e-9);
    // Re-running with those inserts finds every box at its cell.
    let mut r = req(square());
    r.existing = as_existing(&res);
    let again = fill(&r, &catalog).unwrap();
    assert_eq!((again.keep.len(), again.add.len(), again.update.len()), (100, 0, 0));
    // Grid at 90°: û = (0, 1), v̂ = (−1, 0); origin in grid frame (330, −330) → world (330, 330). The top-centre offset
    // (0, 260) turned by 90° is (−260, 0).
    let mut r = req(square());
    r.angle_deg = 90.0;
    let res = fill(&r, &catalog).unwrap();
    assert_eq!(res.add.len(), 100);
    assert_near(res.add[0].center, [330.0, 330.0], 1e-9);
    assert_near(res.add[0].insert, [70.0, 330.0], 1e-9);
    assert_eq!(res.add[0].rotation_deg, 90.0);
}

#[test]
fn order_is_top_down_left_right() {
    let res = run(&req(square()));
    let cells: Vec<[i64; 2]> = res.add.iter().map(|p| p.cell).collect();
    let expect: Vec<[i64; 2]> = (0..10).flat_map(|j| (0..10).map(move |i| [i, j])).collect();
    assert_eq!(cells, expect);
    for w in res.add.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.cell[1] == b.cell[1] {
            assert!(b.center[0] > a.center[0]);
        } else {
            assert!(b.center[1] < a.center[1]);
        }
    }
}

#[test]
fn explicit_origin_allows_negative_cells() {
    // Origin (990, 6270) puts cell (−1, 0) at x = 330.
    let mut r = req(square());
    r.origin = Some([990.0, 6270.0]);
    let res = run(&r);
    assert_eq!(res.add.len(), 100);
    assert_eq!(res.add[0].instance_id, format!("{LAYOUT}:-1:0"));
    assert_eq!(res.add[0].cell, [-1, 0]);
    assert_near(res.add[0].center, [330.0, 6270.0], 1e-9);
    assert_near(res.layout.origin, [990.0, 6270.0], 1e-9);
}

#[test]
fn narrow_pitch_reports_cells_straddling_the_bounds() {
    // Pitch 520 with the default origin (330, 6270): cell −1 centres at x = −190 (footprint −450..70) and cell 12 at 6570
    // (6310..6830), both crossing the boundary; cells 0..=11 fit (cell 11 ends at 6310).
    let mut r = req(square());
    r.pitch_x_mm = Some(520.0);
    let res = run(&r);
    assert_eq!(res.add.len(), 120);
    assert_eq!(res.problems.len(), 20);
    all_have(&res.problems, Status::Invalid, &[Reason::OutsideBoundary]);
    assert_eq!(&cells_of(&res.problems)[..2], [[-1, 0], [12, 0]]);
}

#[test]
fn rotated_grid_matches_brute_force_on_concave_boundary() {
    // Non-aligned concave slab (V notch at the top) with a skewed hole; expected cells come from testing every cell of a
    // wide window directly with the geometry predicates.
    let boundary = poly(&[[0.0, 0.0], [9000.0, 1200.0], [8000.0, 7000.0], [4500.0, 4000.0], [1000.0, 8000.0], [-1500.0, 3000.0]]);
    let hole = poly(&[[2000.0, 2000.0], [3500.0, 2300.0], [3300.0, 3400.0], [1900.0, 3100.0]]);
    let (b_ring, h_ring) = (ring_from_input(&boundary, 1.0, 0.5).unwrap(), ring_from_input(&hole, 1.0, 0.5).unwrap());
    let clearance = 50.0;
    let origin = [123.0, 456.0];
    for angle in [0.0, 17.0, -33.0, 90.0, 123.4] {
        let mut r = req(boundary.clone());
        r.holes = vec![hole.clone()];
        r.angle_deg = angle;
        r.origin = Some(origin);
        r.edge_clearance_mm = Some(clearance);
        let res = run(&r);
        let a = f64::to_radians(angle);
        let (u, v) = (Vec2::new(a.cos(), a.sin()), Vec2::new(-a.sin(), a.cos()));
        let mut expect = Vec::new();
        for j in -40..=40 {
            for i in -40..=40 {
                let c = Vec2::new(origin[0], origin[1]) + u * (660.0 * i as f64) - v * (660.0 * j as f64);
                let fp = footprint(c, 520.0, 520.0, angle);
                let inside = ring_contains_footprint(&b_ring, &fp) && distance_footprint_ring_edges(&fp, &b_ring).0 >= clearance;
                let clear_of_hole = !footprint_intersects_ring(&fp, &h_ring) && distance_footprint_ring_edges(&fp, &h_ring).0 >= clearance;
                if inside && clear_of_hole {
                    expect.push([i as i64, j as i64]);
                }
            }
        }
        let got: Vec<[i64; 2]> = res.add.iter().map(|p| p.cell).collect();
        assert!(expect.len() > 50, "{angle}°: fixture too small ({})", expect.len());
        assert_eq!(got, expect, "{angle}°");
    }
}

#[test]
fn layout_ids_are_validated() {
    for bad in ["", "a:b", "ä", &"a".repeat(65)] {
        let mut r = req(square());
        r.layout_id = bad.into();
        assert!(matches!(fill(&r, &Catalog::builtin()), Err(UbotError::BadInput(_))), "{bad:?}");
        let mut r = req(square());
        r.previous_layout_id = Some(bad.into());
        assert!(matches!(fill(&r, &Catalog::builtin()), Err(UbotError::BadInput(_))), "{bad:?}");
    }
    let mut r = req(square());
    r.layout_id = format!("Ab_-{}", "9".repeat(60));
    assert!(fill(&r, &Catalog::builtin()).is_ok());
}

#[test]
fn hostile_numbers_never_panic() {
    let c = Catalog::builtin();
    let bad_input = |r: &FillRequest, what: &str| {
        let e = fill(r, &c);
        assert!(matches!(e, Err(UbotError::BadInput(_))), "{what}: {e:?}");
    };
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        let mut r = req(square());
        r.rib_mm = Some(bad);
        bad_input(&r, "rib");
        let mut r = req(square());
        r.pitch_x_mm = Some(bad);
        bad_input(&r, "pitchX");
        let mut r = req(square());
        r.pitch_y_mm = Some(bad);
        bad_input(&r, "pitchY");
        let mut r = req(square());
        r.unit_scale = bad;
        bad_input(&r, "unitScale");
        let mut r = req(square());
        r.edge_clearance_mm = Some(bad);
        bad_input(&r, "edgeClearance");
        let mut r = req(square());
        r.exclusion_clearance_mm = Some(bad);
        bad_input(&r, "exclusionClearance");
        let mut r = req(square());
        let mut z = zone("c", rect(1.0, 1.0, 2.0, 2.0), ZoneType::Column);
        z.clearance_mm = Some(bad);
        r.zones = vec![z];
        bad_input(&r, "zone clearance");
        let mut r = req(square());
        r.existing = as_existing(&run(&req(square())));
        r.existing[0].scale = bad;
        if bad != -1.0 {
            bad_input(&r, "existing scale");
        }
    }
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut r = req(square());
        r.angle_deg = bad;
        bad_input(&r, "angle");
        let mut r = req(square());
        r.origin = Some([bad, 0.0]);
        bad_input(&r, "origin");
        let mut r = req(square());
        r.existing = as_existing(&run(&req(square())));
        r.existing[3].insert[1] = bad;
        bad_input(&r, "existing insert");
        let mut r = req(square());
        r.existing = as_existing(&run(&req(square())));
        r.existing[3].rotation_deg = bad;
        bad_input(&r, "existing rotation");
    }
    // Clearances past MAX_CLEARANCE_MM are rejected up front, whichever source they come from.
    for huge in [1e9, MAX_CLEARANCE_MM + 1.0] {
        let started = std::time::Instant::now();
        let mut r = req(square());
        r.edge_clearance_mm = Some(huge);
        bad_input(&r, "huge edge clearance");
        let mut r = req(square());
        r.exclusion_clearance_mm = Some(huge);
        bad_input(&r, "huge exclusion clearance");
        let mut r = req(square());
        let mut z = zone("c", rect(1.0, 1.0, 2.0, 2.0), ZoneType::Column);
        z.clearance_mm = Some(huge);
        r.zones = vec![z];
        bad_input(&r, "huge zone clearance");
        let mut pack = c.rulepacks[0].clone();
        pack.edge_clearance_mm = huge;
        let loose = c.overlay(&Catalog { products: vec![], mappings: vec![], rulepacks: vec![pack], ..c.clone() });
        assert!(matches!(fill(&req(square()), &loose), Err(UbotError::BadInput(_))), "huge rulepack clearance");
        assert!(started.elapsed().as_secs_f64() < 1.0, "{:?}", started.elapsed());
    }
    let mut r = req(square());
    r.edge_clearance_mm = Some(MAX_CLEARANCE_MM);
    let res = fill(&r, &c).unwrap();
    assert_eq!((res.add.len(), res.summary.invalid), (0, 100));
    // A negative angle is an ordinary angle.
    let mut r = req(square());
    r.angle_deg = -30.0;
    assert!(fill(&r, &c).is_ok());
    // Pitch below the box size would overlap boxes; zero scale is degenerate.
    for (px, py) in [(Some(0.0), None), (Some(519.0), None), (None, Some(100.0))] {
        let mut r = req(square());
        r.pitch_x_mm = px;
        r.pitch_y_mm = py;
        bad_input(&r, "small pitch");
    }
    let mut r = req(square());
    r.existing = as_existing(&run(&req(square())));
    r.existing[0].scale = 0.0;
    bad_input(&r, "zero scale");
    // Vertices that overflow once scaled to mm.
    let mut r = req(rect(0.0, 0.0, 1e300, 1e300));
    r.unit_scale = 1e10;
    bad_input(&r, "overflow");
    // Too many cells, too many existing boxes, too many vertices, origin far away.
    let r = req(rect(0.0, 0.0, 1e9, 1e9));
    assert!(matches!(fill(&r, &c), Err(UbotError::Limit(_))));
    let mut r = req(square());
    r.origin = Some([1e300, 0.0]);
    assert!(matches!(fill(&r, &c), Err(UbotError::Limit(_))));
    let mut r = req(square());
    let one = as_existing(&run(&req(square())))[0].clone();
    r.existing = vec![one; 100_001];
    assert!(matches!(fill(&r, &c), Err(UbotError::Limit(_))));
    let mut r = req(square());
    let gon = |cx: f64, n: usize| RingInput {
        vertices: (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                [cx + 100.0 * a.cos(), 3000.0 + 100.0 * a.sin(), 0.0]
            })
            .collect(),
    };
    r.zones = vec![zone("a", gon(1000.0, 10_000), ZoneType::Column), zone("b", gon(4000.0, 10_000), ZoneType::Column)];
    assert!(matches!(fill(&r, &c), Err(UbotError::Limit(_))));
    // Unknown references.
    let mut r = req(square());
    r.product_id = "nope".into();
    assert!(matches!(fill(&r, &c), Err(UbotError::UnknownProduct(_))));
    let mut r = req(square());
    r.rulepack_id = Some("nope".into());
    bad_input(&r, "rulepack");
    // Broken boundary geometry is reported, not panicked on.
    let r = req(poly(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]]));
    assert!(matches!(fill(&r, &c), Err(UbotError::Geometry(_))));
}

#[test]
fn json_shape_is_camel_case_and_self_describing() {
    let r: FillRequest = serde_json::from_value(serde_json::json!({
        "unitScale": 1.0,
        "layoutId": LAYOUT,
        "reset": true,
        "boundary": {"vertices": [[0, 0, 0], [6600, 0, 0], [6600, 6600, 0], [0, 6600, 0]]},
        "zones": [{"id": "c", "ring": {"vertices": [[1120, 1120, 0], [1520, 1120, 0], [1520, 1520, 0], [1120, 1520, 0]]}, "zoneType": "column"}],
        "productId": "ubot-h13-7",
        "angleDeg": 0.0,
        "existing": [{"handle": "1F", "instanceId": format!("{LAYOUT}:0:0"), "layoutId": LAYOUT, "productId": "ubot-h13-7",
                      "blockName": "UBOT_H13_7", "insert": [400.0, 6270.0], "rotationDeg": 0.0, "scale": 1.0}]
    }))
    .unwrap();
    assert!(r.holes.is_empty() && r.rib_mm.is_none() && r.origin.is_none());
    let v = serde_json::to_value(run(&r)).unwrap();
    assert_eq!(v["update"][0]["handle"], "1F");
    assert_eq!(v["update"][0]["placement"]["instanceId"], format!("{LAYOUT}:0:0"));
    assert_eq!(v["problems"][0]["status"], "INVALID");
    assert_eq!(v["problems"][0]["reasons"][0], "IN_ZONE");
    assert!(v["layout"]["geometryHash"].is_string());
    assert!(v["layout"]["pitchXMm"].is_number());
    assert!(v["summary"]["uncertain"].is_number());
    assert_eq!(serde_json::to_value(ZoneType::AltProduct).unwrap(), "alt_product");
    assert_eq!(serde_json::to_value(Reason::CurvedEdgeTolerance).unwrap(), "CURVED_EDGE_TOLERANCE");
}

/// A self-intersecting bow tie: `ring_from_input` refuses it.
fn bow_tie(x: f64, y: f64) -> RingInput {
    poly(&[[x, y], [x + 1000.0, y + 1000.0], [x + 1000.0, y], [x, y + 1000.0]])
}

#[test]
fn zones_out_of_reach_are_dropped_before_any_check() {
    let base = run(&req(square()));
    let mut r = req(square());
    // A broken zone and a 20 000-vertex zone on another floor, both beyond MAX_CLEARANCE_MM of the slab.
    let far = 6600.0 + MAX_CLEARANCE_MM + 1000.0;
    let many = RingInput {
        vertices: (0..20_000)
            .map(|k| {
                let a = f64::from(k) * std::f64::consts::TAU / 20_000.0;
                [far + 5000.0 + 1000.0 * a.cos(), 1000.0 * a.sin(), 0.0]
            })
            .collect(),
    };
    r.zones = vec![zone("broken", bow_tie(far, 0.0), ZoneType::Exclusion), zone("many", many, ZoneType::Column)];
    let res = run(&r);
    assert_eq!(res.summary, base.summary);
    assert_eq!(res.input_hash, base.input_hash, "far zones do not change the request hash");
    assert_eq!(res.layout.geometry_hash, base.layout.geometry_hash);
    // Within reach the broken zone is an error again.
    r.zones = vec![zone("broken", bow_tie(1000.0, 1000.0), ZoneType::Exclusion)];
    assert!(fill(&r, &Catalog::builtin()).is_err());
}

#[test]
fn non_finite_zone_is_an_error_wherever_it_is() {
    let mut r = req(square());
    r.zones = vec![zone("nan", poly(&[[1e9, 0.0], [f64::NAN, 0.0], [1e9, 10.0]]), ZoneType::Exclusion)];
    let err = fill(&r, &Catalog::builtin()).unwrap_err();
    assert!(matches!(&err, UbotError::BadInput(m) if m.contains("zone nan")), "{err}");
    assert!(geometry_hash(1.0, &square(), &[], &r.zones).is_err());
}
