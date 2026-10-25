use cadcraft_geom::{Bounds2, Vec2};
use cadcraft_ubot::{
    EdgeIndex, Footprint, Ring, RingInput, UbotError, distance_footprint_ring_edges, distance_footprint_ring_edges_with, footprint,
    footprint_intersects_ring, footprint_intersects_ring_with, point_in_ring, ring_contains_footprint, ring_contains_footprint_with, ring_from_input,
};

fn input(v: &[[f64; 2]]) -> RingInput {
    RingInput { vertices: v.iter().map(|p| [p[0], p[1], 0.0]).collect() }
}

fn square(side: f64) -> Ring {
    ring_from_input(&input(&[[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]]), 1.0, 0.5).unwrap()
}

#[test]
fn square_cw_becomes_ccw() {
    let r = ring_from_input(&input(&[[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]]), 1.0, 0.5).unwrap();
    assert_eq!(r.points.len(), 4);
    assert_eq!(r.points[0], Vec2::new(10.0, 0.0));
    assert_eq!(r.points[1], Vec2::new(10.0, 10.0));
    assert_eq!(r.curved, [false; 4]);
}

#[test]
fn units_cm_scale_by_10() {
    let r = ring_from_input(&input(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]), 10.0, 0.5).unwrap();
    assert_eq!(r.points[2], Vec2::new(100.0, 100.0));
}

#[test]
fn closing_duplicate_vertex_is_dropped() {
    let r = ring_from_input(&input(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]), 1.0, 0.5).unwrap();
    assert_eq!(r.points.len(), 4);
}

#[test]
fn self_intersecting_bowtie_is_rejected() {
    let e = ring_from_input(&input(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]]), 1.0, 0.5).unwrap_err();
    assert!(matches!(e, UbotError::Geometry(_)), "{e:?}");
}

#[test]
fn spike_and_vertex_touch_are_rejected() {
    // Edge folds back on itself (collinear overlap).
    let spike = input(&[[0.0, 0.0], [10.0, 0.0], [5.0, 0.0], [5.0, 10.0]]);
    assert!(matches!(ring_from_input(&spike, 1.0, 0.5), Err(UbotError::Geometry(_))));
    // Non-adjacent vertex touches another edge.
    let touch = input(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [5.0, 0.0], [5.0, -10.0], [0.0, -10.0]]);
    assert!(matches!(ring_from_input(&touch, 1.0, 0.5), Err(UbotError::Geometry(_))));
}

#[test]
fn zero_area_and_duplicate_points_rejected() {
    let collinear = input(&[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]);
    assert!(matches!(ring_from_input(&collinear, 1.0, 0.5), Err(UbotError::Geometry(_))));
    let dup = input(&[[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]);
    assert!(ring_from_input(&dup, 1.0, 0.5).is_err());
    let two = input(&[[0.0, 0.0], [1.0, 0.0]]);
    assert!(matches!(ring_from_input(&two, 1.0, 0.5), Err(UbotError::BadInput(_))));
}

#[test]
fn nan_vertex_rejected() {
    for bad in [f64::NAN, f64::INFINITY] {
        let mut i = input(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        i.vertices[1][0] = bad;
        assert!(matches!(ring_from_input(&i, 1.0, 0.5), Err(UbotError::BadInput(_))));
        let mut j = input(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        j.vertices[1][2] = bad;
        assert!(matches!(ring_from_input(&j, 1.0, 0.5), Err(UbotError::BadInput(_))));
    }
    let ok = input(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    for s in [f64::NAN, 0.0, -1.0, f64::INFINITY] {
        assert!(matches!(ring_from_input(&ok, s, 0.5), Err(UbotError::BadInput(_))));
    }
    assert!(matches!(ring_from_input(&ok, 1.0, 0.0), Err(UbotError::BadInput(_))));
}

#[test]
fn too_many_vertices_is_limit_error() {
    let vertices: Vec<[f64; 3]> = (0..20_001).map(|i| [i as f64, (i % 2) as f64, 0.0]).collect();
    assert!(matches!(ring_from_input(&RingInput { vertices }, 1.0, 0.5), Err(UbotError::Limit(_))));
}

#[test]
fn tessellation_over_vertex_cap_is_limit_error() {
    // 10 000 semicircles at a tiny tolerance would otherwise explode into millions of points.
    let vertices: Vec<[f64; 3]> = (0..10_000).map(|i| [i as f64 * 1e6, 0.0, 1.0]).collect();
    let r = ring_from_input(&RingInput { vertices }, 1.0, 1e-6);
    assert!(matches!(r, Err(UbotError::Limit(_))), "{r:?}");
}

#[test]
fn semicircle_bulge_tessellates_within_half_mm() {
    // Diameter 2000 along x, bulging to -y (bulge 1 = CCW semicircle).
    let i = RingInput { vertices: vec![[-1000.0, 0.0, 1.0], [1000.0, 0.0, 0.0]] };
    let r = ring_from_input(&i, 1.0, 0.5).unwrap();
    assert!(r.points.len() > 8);
    let curved = r.curved.iter().filter(|c| **c).count();
    assert_eq!(curved, r.points.len() - 1);
    assert!(!r.curved[r.points.len() - 1]);
    for p in &r.points {
        let d = p.dist(Vec2::ZERO);
        assert!((d - 1000.0).abs() <= 0.5, "{p:?} {d}");
    }
    assert!(r.points.iter().any(|p| p.y < -900.0));
    for i in 0..r.points.len() - 1 {
        let mid = r.points[i].mid(r.points[i + 1]);
        assert!((mid.dist(Vec2::ZERO) - 1000.0).abs() <= 0.5, "sagitta at edge {i}");
    }
}

#[test]
fn negative_bulge_curves_the_other_way() {
    let i = RingInput { vertices: vec![[-1000.0, 0.0, -1.0], [1000.0, 0.0, 0.0]] };
    let r = ring_from_input(&i, 1.0, 0.5).unwrap();
    assert!(r.points.iter().any(|p| p.y > 900.0));
    for p in &r.points {
        assert!((p.dist(Vec2::ZERO) - 1000.0).abs() <= 0.5);
    }
}

#[test]
fn curved_flags_follow_ccw_reversal() {
    // CW input with a curved first edge: after reversal the arc edges must still be flagged.
    let i = RingInput { vertices: vec![[1000.0, 0.0, -1.0], [-1000.0, 0.0, 0.0]] };
    let r = ring_from_input(&i, 1.0, 0.5).unwrap();
    let last = r.points.len() - 1;
    assert_eq!(r.points[last], Vec2::new(1000.0, 0.0));
    assert!(!r.curved[last]);
    assert!(r.curved[..last].iter().all(|c| *c));
}

#[test]
fn footprint_rotated_45_corners() {
    let f = footprint(Vec2::ZERO, 520.0, 520.0, 45.0);
    let mut xs: Vec<f64> = f.corners.iter().map(|c| c.x.abs()).collect();
    xs.extend(f.corners.iter().map(|c| c.y.abs()));
    let expected = 260.0 * 2f64.sqrt();
    assert_eq!(xs.iter().filter(|v| **v < 1e-9).count(), 4);
    assert_eq!(xs.iter().filter(|v| (**v - expected).abs() < 1e-9).count(), 4);
    assert!((expected - 367.7).abs() < 0.05);
}

#[test]
fn contains_and_distance() {
    let ring = square(2000.0);
    let f = footprint(Vec2::new(1000.0, 1000.0), 520.0, 520.0, 0.0);
    assert!(ring_contains_footprint(&ring, &f));
    // A box inside the ring overlaps its region (this is how holes are tested).
    assert!(footprint_intersects_ring(&f, &ring));
    let (d, curved) = distance_footprint_ring_edges(&f, &ring);
    assert!((d - 740.0).abs() < 1e-9 && !curved);
    assert!(point_in_ring(Vec2::new(1.0, 1.0), &ring));
    assert!(!point_in_ring(Vec2::new(-1.0, 1.0), &ring));
}

#[test]
fn crossing_and_outside_footprints() {
    let ring = square(2000.0);
    let cross = footprint(Vec2::new(1900.0, 1000.0), 520.0, 520.0, 0.0);
    assert!(!ring_contains_footprint(&ring, &cross));
    assert!(footprint_intersects_ring(&cross, &ring));
    assert_eq!(distance_footprint_ring_edges(&cross, &ring).0, 0.0);
    let outside = footprint(Vec2::new(5000.0, 1000.0), 520.0, 520.0, 0.0);
    assert!(!ring_contains_footprint(&ring, &outside));
    assert!(!footprint_intersects_ring(&outside, &ring));
    // Footprint larger than the ring: ring vertices lie inside it.
    let big = footprint(Vec2::new(1000.0, 1000.0), 5000.0, 5000.0, 0.0);
    assert!(footprint_intersects_ring(&big, &ring));
    assert!(!ring_contains_footprint(&ring, &big));
}

#[test]
fn flush_footprint_is_contained_and_identical_hole_intersects() {
    let ring = square(520.0);
    let f = footprint(Vec2::new(260.0, 260.0), 520.0, 520.0, 0.0);
    assert!(ring_contains_footprint(&ring, &f));
    assert!(footprint_intersects_ring(&f, &ring));
}

#[test]
fn nearest_edge_curved_flag() {
    let i = RingInput { vertices: vec![[-1000.0, 0.0, 1.0], [1000.0, 0.0, 0.0]] };
    let ring = ring_from_input(&i, 1.0, 0.5).unwrap();
    let f = footprint(Vec2::new(0.0, -700.0), 100.0, 100.0, 0.0);
    let (d, curved) = distance_footprint_ring_edges(&f, &ring);
    assert!(curved && d > 240.0 && d < 250.0, "{d}");
}

#[test]
fn hostile_footprints_never_panic() {
    let ring = square(100.0);
    let f = Footprint { corners: [Vec2::new(f64::NAN, 0.0); 4] };
    assert!(!ring_contains_footprint(&ring, &f));
    assert!(!footprint_intersects_ring(&f, &ring));
    let _ = distance_footprint_ring_edges(&f, &ring);
    let g = footprint(Vec2::new(f64::INFINITY, 0.0), 1.0, 1.0, f64::NAN);
    assert!(!ring_contains_footprint(&ring, &g));
}

#[test]
fn edge_index_returns_only_nearby_edges() {
    let a = square(1000.0);
    let b = ring_from_input(&input(&[[10_000.0, 0.0], [11_000.0, 0.0], [11_000.0, 1000.0], [10_000.0, 1000.0]]), 1.0, 0.5).unwrap();
    let idx = EdgeIndex::new(&[&a, &b], 500.0);
    let near: Vec<(usize, usize)> = idx.query(Bounds2::new(Vec2::new(400.0, -50.0), Vec2::new(600.0, 50.0)), 10.0).collect();
    assert_eq!(near, vec![(0, 0)]);
    let far: Vec<(usize, usize)> = idx.query(Bounds2::new(Vec2::new(10_400.0, 400.0), Vec2::new(10_600.0, 600.0)), 0.0).collect();
    assert!(far.is_empty());
    let all: Vec<(usize, usize)> = idx.query(Bounds2::new(Vec2::new(-1.0, -1.0), Vec2::new(20_000.0, 2000.0)), 0.0).collect();
    assert_eq!(all.len(), 8);
}

#[test]
fn edge_index_hostile_queries_and_cell_sizes() {
    let a = square(1000.0);
    for cell in [0.0, -5.0, f64::NAN, f64::INFINITY, 1e-9] {
        let idx = EdgeIndex::new(&[&a], cell);
        let n = idx.query(Bounds2::new(Vec2::new(-1.0, -1.0), Vec2::new(2000.0, 2000.0)), 0.0).count();
        assert_eq!(n, 4, "cell {cell}");
    }
    let idx = EdgeIndex::new(&[&a], 100.0);
    assert_eq!(idx.query(Bounds2::EMPTY, 0.0).count(), 0);
    let nan = Bounds2 { min: Vec2::new(f64::NAN, 0.0), max: Vec2::new(1.0, 1.0) };
    assert_eq!(idx.query(nan, 0.0).count(), 0);
    let inf = Bounds2 { min: Vec2::new(0.0, 0.0), max: Vec2::new(f64::INFINITY, 1.0) };
    assert_eq!(idx.query(inf, 0.0).count(), 0);
    assert_eq!(idx.query(Bounds2::new(Vec2::ZERO, Vec2::new(1.0, 1.0)), f64::NAN).count(), 0);
    assert_eq!(EdgeIndex::new(&[], 10.0).query(Bounds2::new(Vec2::ZERO, Vec2::new(1.0, 1.0)), 0.0).count(), 0);
}

fn ring_of(v: &[[f64; 2]]) -> Ring {
    ring_from_input(&input(v), 1.0, 0.5).unwrap()
}

fn box_at(x: f64, y: f64) -> Footprint {
    footprint(Vec2::new(x, y), 520.0, 520.0, 0.0)
}

#[test]
fn hole_flush_with_box_sides_still_intersects() {
    // A: hole overlaps the box by 220x520 and has vertices exactly on the box boundary.
    let f = box_at(260.0, 260.0);
    let a = ring_of(&[[300.0, 0.0], [1000.0, 0.0], [1000.0, 520.0], [300.0, 520.0]]);
    assert!(footprint_intersects_ring(&f, &a));
    // B: triangle whose hypotenuse is the box diagonal.
    let b = ring_of(&[[0.0, 0.0], [520.0, 0.0], [520.0, 520.0]]);
    assert!(footprint_intersects_ring(&f, &b));
    // D: rotated box, triangle through three of its corners.
    let r = footprint(Vec2::ZERO, 520.0, 520.0, 45.0);
    let c = r.corners;
    let d = ring_of(&[[c[1].x, c[1].y], [c[2].x, c[2].y], [c[3].x, c[3].y]]);
    assert!(footprint_intersects_ring(&r, &d));
    // A hole merely sharing a side does not intersect.
    let touching = ring_of(&[[520.0, 0.0], [1000.0, 0.0], [1000.0, 520.0], [520.0, 520.0]]);
    assert!(!footprint_intersects_ring(&f, &touching));
}

#[test]
fn slot_with_vertices_on_box_boundary_breaks_containment() {
    // C: notch x 300..400 reaching y=520 cuts into the box; wall vertices sit at y=0 and y=520.
    let ring = ring_of(&[
        [-1000.0, -1000.0],
        [300.0, -1000.0],
        [300.0, 0.0],
        [300.0, 520.0],
        [400.0, 520.0],
        [400.0, 0.0],
        [400.0, -1000.0],
        [2000.0, -1000.0],
        [2000.0, 2000.0],
        [-1000.0, 2000.0],
    ]);
    assert!(!ring_contains_footprint(&ring, &box_at(260.0, 260.0)));
    assert!(ring_contains_footprint(&ring, &box_at(1000.0, 1000.0)));
}

#[test]
fn l_and_u_shapes() {
    let l = ring_of(&[[0.0, 0.0], [2000.0, 0.0], [2000.0, 1000.0], [1000.0, 1000.0], [1000.0, 2000.0], [0.0, 2000.0]]);
    assert!(ring_contains_footprint(&l, &box_at(500.0, 1500.0)));
    assert!(!ring_contains_footprint(&l, &box_at(1200.0, 1200.0)));
    assert!(footprint_intersects_ring(&box_at(1200.0, 1200.0), &l));
    let u = ring_of(&[
        [0.0, 0.0],
        [3000.0, 0.0],
        [3000.0, 2000.0],
        [2000.0, 2000.0],
        [2000.0, 1000.0],
        [1000.0, 1000.0],
        [1000.0, 2000.0],
        [0.0, 2000.0],
    ]);
    let bridge = footprint(Vec2::new(1500.0, 1500.0), 1200.0, 200.0, 0.0);
    assert!(!ring_contains_footprint(&u, &bridge));
    assert!(ring_contains_footprint(&u, &box_at(500.0, 500.0)));
}

#[test]
fn skew_edge_and_rotated_box_flush_against_it() {
    let tri = ring_of(&[[0.0, 0.0], [2000.0, 0.0], [0.0, 2000.0]]);
    assert!(ring_contains_footprint(&tri, &box_at(500.0, 500.0)));
    assert!(!ring_contains_footprint(&tri, &box_at(900.0, 900.0)));
    // Box side lies exactly on the hypotenuse x + y = 2000.
    let k = 260.0 / 2f64.sqrt();
    let flush = footprint(Vec2::new(1000.0 - k, 1000.0 - k), 520.0, 520.0, -45.0);
    assert!(ring_contains_footprint(&tri, &flush));
    let out = footprint(Vec2::new(1001.0 - k, 1001.0 - k), 520.0, 520.0, -45.0);
    assert!(!ring_contains_footprint(&tri, &out));
}

#[test]
fn candidate_variants_match_full_scan() {
    let ring = ring_of(&[[0.0, 0.0], [2000.0, 0.0], [2000.0, 1000.0], [1000.0, 1000.0], [1000.0, 2000.0], [0.0, 2000.0]]);
    let idx = EdgeIndex::new(&[&ring], 300.0);
    for (x, y) in [(500.0, 1500.0), (1200.0, 1200.0), (1000.0, 500.0), (5000.0, 5000.0), (1000.0, 1000.0)] {
        let f = box_at(x, y);
        let near: Vec<usize> = idx.query(cadcraft_geom::Bounds2::from_points(f.corners), 1.0).map(|(_, e)| e).collect();
        assert_eq!(ring_contains_footprint_with(&ring, &f, near.iter().copied()), ring_contains_footprint(&ring, &f), "{x},{y}");
        assert_eq!(footprint_intersects_ring_with(&f, &ring, near.iter().copied()), footprint_intersects_ring(&f, &ring), "{x},{y}");
    }
    let f = box_at(500.0, 1500.0);
    let (d, c) = distance_footprint_ring_edges_with(&f, &ring, [3usize]);
    assert!((d - 240.0).abs() < 1e-9 && !c);
    assert_eq!(distance_footprint_ring_edges_with(&f, &ring, std::iter::empty()).0, f64::INFINITY);
}

#[test]
fn curved_flag_true_when_any_nearest_edge_is_curved() {
    // Box corner touches the vertex between a straight and a curved edge: both at distance 0.
    let i = RingInput { vertices: vec![[-1000.0, 0.0, 1.0], [1000.0, 0.0, 0.0]] };
    let ring = ring_from_input(&i, 1.0, 0.5).unwrap();
    let f = footprint(Vec2::new(1000.0 - 50.0, 50.0), 100.0, 100.0, 0.0);
    let (d, curved) = distance_footprint_ring_edges(&f, &ring);
    assert_eq!(d, 0.0);
    assert!(curved);
}

fn comb(teeth: usize, height: f64) -> Vec<[f64; 2]> {
    let mut v = Vec::new();
    for t in 0..teeth {
        let x = t as f64;
        v.extend([[x, 0.0], [x, height], [x + 0.4, height], [x + 0.4, 0.0]]);
    }
    v.push([teeth as f64, -1.0]);
    v.push([0.0, -1.0]);
    v
}

#[test]
fn adversarial_tall_comb_validates_quickly() {
    // 19 602 vertices, every edge ~10 000 mm long and 1 mm apart: defeats a bucket grid sized by edge length.
    let v = comb(4_900, 10_000.0);
    assert!(v.len() <= 20_000);
    let start = std::time::Instant::now();
    let r = ring_from_input(&input(&v), 1.0, 0.5).unwrap();
    let secs = start.elapsed().as_secs_f64();
    assert_eq!(r.points.len(), v.len());
    let bound = if cfg!(debug_assertions) { 5.0 } else { 0.5 };
    assert!(secs < bound, "{secs}");
}

#[test]
fn adversarial_wide_comb_validates_quickly() {
    // Same comb rotated 90 degrees so the long edges are horizontal.
    let v: Vec<[f64; 2]> = comb(4_900, 10_000.0).into_iter().map(|p| [p[1], p[0]]).collect();
    let start = std::time::Instant::now();
    ring_from_input(&input(&v), 1.0, 0.5).unwrap();
    let bound = if cfg!(debug_assertions) { 5.0 } else { 0.5 };
    assert!(start.elapsed().as_secs_f64() < bound, "{:?}", start.elapsed());
}

#[test]
fn comb_with_touching_tooth_is_rejected() {
    let mut v = comb(50, 100.0);
    // Pull one tooth wall onto its neighbour.
    v[5][0] = 2.0;
    v[6][0] = 2.0;
    assert!(matches!(ring_from_input(&input(&v), 1.0, 0.5), Err(UbotError::Geometry(_))));
}
