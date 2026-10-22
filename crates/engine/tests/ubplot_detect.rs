mod support;

use std::sync::Arc;

use cadcraft_doc::{Block, Common, Entity, EntityKind, Handle, Line, Space};
use cadcraft_engine::Session;
use cadcraft_engine::sheetplan::codes;
use cadcraft_engine::sheetplan::outline::{Relation, normalize_outline, rect_of, relation};
use cadcraft_engine::sheetplan::{FrameCandidate, FrameRegistry, GeometryRule, Severity, detect_frames, is_frame_candidate};
use cadcraft_geom::{Mat3, PolyVertex, Vec2};
use support::frames::{anon_dynamic, frame_block, nest, place, polyline, title_only_block};

fn setup() -> Session {
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "A1", 841.0, 594.0);
    s
}

fn detect(s: &Session, handles: &[Handle]) -> Vec<FrameCandidate> {
    detect_frames(s.doc().unwrap(), handles, &FrameRegistry::builtin())
}

fn expected(w: f64, h: f64, m: Mat3) -> [[f64; 2]; 4] {
    [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].map(|p| {
        let p = m.apply(p);
        [p.x, p.y]
    })
}

fn assert_outline(candidate: &FrameCandidate, expected: [[f64; 2]; 4]) {
    let actual = candidate.outline.expect("frame outline");
    for (a, e) in actual.into_iter().zip(expected) {
        for (a, e) in a.into_iter().zip(e) {
            assert!((a - e).abs() <= 1e-6, "{actual:?} != {expected:?}");
        }
    }
    let [a, b, c, _] = actual.map(|[x, y]| Vec2::new(x, y));
    assert!((b - a).cross(c - b) > 0.0);
}

fn has(candidate: &FrameCandidate, code: &str, severity: Severity) -> bool {
    candidate.issues.iter().any(|issue| issue.code == code && issue.severity == severity)
}

fn line(s: &mut Session, a: Vec2, b: Vec2) -> Handle {
    s.doc_mut().unwrap().add(&Space::Model, Common::default(), EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) })).unwrap()
}

#[test]
fn outline_is_frame_rectangle_not_insert_aabb() {
    let mut s = setup();
    let d = s.doc_mut().unwrap();
    let h = d.new_handle();
    Arc::make_mut(d.blocks.get_mut("A1").unwrap())
        .entities
        .push(Entity::new(h, EntityKind::Line(Line { a: Vec2::new(841.0, 0.0).to3(0.0), b: Vec2::new(891.0, 0.0).to3(0.0) })));
    let insert = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[("sheet_no", "KT-01"), ("REV", "A")]);
    let candidates = detect(&s, &[insert]);
    assert_outline(&candidates[0], expected(841.0, 594.0, Mat3::IDENTITY));
    assert_eq!(candidates[0].local_mm, [841.0, 594.0]);
    assert_eq!(candidates[0].insert_scale, 1.0);
    assert_eq!(candidates[0].attributes.get("SHEET_NO").map(String::as_str), Some("KT-01"));
    assert_eq!(candidates[0].attributes.get("REV").map(String::as_str), Some("A"));
}

#[test]
fn scaled_insert_reports_scale_and_paper_size() {
    let mut s = setup();
    let insert = place(&mut s, "A1", [0.0, 0.0], 100.0, 0.0, false, &[]);
    let candidates = detect(&s, &[insert]);
    assert_outline(&candidates[0], expected(841.0, 594.0, Mat3::scale(100.0, 100.0)));
    for (actual, expected) in candidates[0].local_mm.into_iter().zip([841.0, 594.0]) {
        assert!((actual - expected).abs() < 1e-6);
    }
    assert_eq!(candidates[0].insert_scale, 100.0);
}

#[test]
fn rotated_90_and_30_degrees() {
    let mut s = setup();
    for rotation in [90.0_f64, 30.0] {
        let insert = place(&mut s, "A1", [500_000.0, 2_100_000.0], 1.0, rotation, false, &[]);
        let candidates = detect(&s, &[insert]);
        let m = Mat3::translate(Vec2::new(500_000.0, 2_100_000.0)).then_before(Mat3::rotate(rotation.to_radians()));
        assert_outline(&candidates[0], expected(841.0, 594.0, m));
        assert!((candidates[0].rotation_deg - rotation).abs() < 1e-9);
    }
}

#[test]
fn mirrored_frame_is_normalized_and_warned() {
    let mut s = setup();
    let insert = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, true, &[]);
    let candidates = detect(&s, &[insert]);
    let c = &candidates[0];
    assert!(c.mirrored);
    assert!(has(c, codes::FRAME_MIRRORED, Severity::Warn));
    assert_eq!(c.rotation_deg, 0.0);
    assert_outline(c, [[-841.0, 0.0], [0.0, 0.0], [0.0, 594.0], [-841.0, 594.0]]);
    assert_eq!(c.local_mm, [841.0, 594.0]);
}

#[test]
fn nested_block_transform_is_applied() {
    let mut s = setup();
    nest(s.doc_mut().unwrap(), "MIDDLE", "A1", [5.0, 5.0], 10.0);
    nest(s.doc_mut().unwrap(), "OUTER", "MIDDLE", [5.0, 5.0], 10.0);
    let insert = place(&mut s, "OUTER", [5.0, 5.0], 1.0, 10.0, false, &[]);
    let step = Mat3::translate(Vec2::new(5.0, 5.0)).then_before(Mat3::rotate(10.0_f64.to_radians()));
    let candidates = detect(&s, &[insert]);
    assert_outline(&candidates[0], expected(841.0, 594.0, step.then_before(step).then_before(step)));
    assert!((candidates[0].rotation_deg - 30.0).abs() < 1e-9);
}

#[test]
fn dynamic_u_frame_uses_effective_name_and_ignores_hidden_geometry() {
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "FRAME-A1", 841.0, 594.0);
    anon_dynamic(s.doc_mut().unwrap(), "FRAME-A1", "*U7");
    let insert = place(&mut s, "*U7", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let candidates = detect(&s, &[insert]);
    assert_eq!(candidates[0].effective_name, "FRAME-A1");
    assert_outline(&candidates[0], expected(841.0, 594.0, Mat3::IDENTITY));
}

#[test]
fn title_block_in_corner_uses_enclosing_polyline() {
    let mut s = Session::new();
    title_only_block(s.doc_mut().unwrap(), "TITLE");
    s.doc_mut().unwrap().add(&Space::Model, Common::default(), polyline(420.0, 297.0)).unwrap();
    let insert = place(&mut s, "TITLE", [235.0, 5.0], 1.0, 0.0, false, &[]);
    let candidates = detect(&s, &[insert]);
    assert_outline(&candidates[0], expected(420.0, 297.0, Mat3::IDENTITY));
    let mut block = Block::new("SINGLE-LINE");
    let d = s.doc_mut().unwrap();
    block.entities.push(Entity::new(d.new_handle(), EntityKind::Line(Line { a: Vec2::ZERO.to3(0.0), b: Vec2::X.to3(0.0) })));
    d.blocks.insert("SINGLE-LINE".into(), Arc::new(block));
    let insert = place(&mut s, "SINGLE-LINE", [10.0, 10.0], 1.0, 0.0, false, &[]);
    assert_outline(&detect(&s, &[insert])[0], expected(420.0, 297.0, Mat3::IDENTITY));
}

#[test]
fn enclosing_rect_uses_smallest_outline_in_insert_space() {
    let mut s = Session::new();
    title_only_block(s.doc_mut().unwrap(), "TITLE");
    let d = s.doc_mut().unwrap();
    d.add(&Space::Model, Common::default(), polyline(450.0, 320.0)).unwrap();
    d.add(&Space::Model, Common::default(), polyline(420.0, 297.0)).unwrap();
    d.layouts.push(cadcraft_doc::Layout::new("Plot", 100));
    let paper = Space::Paper("Plot".into());
    d.add(&paper, Common::default(), polyline(600.0, 400.0)).unwrap();
    let insert = place(&mut s, "TITLE", [235.0, 5.0], 1.0, 0.0, false, &[]);
    assert_outline(&detect(&s, &[insert])[0], expected(420.0, 297.0, Mat3::IDENTITY));
    let d = s.doc_mut().unwrap();
    let entity = d.remove_entity(insert).unwrap();
    d.space_mut(&paper).unwrap().push((*entity).clone());
    assert_outline(&detect(&s, &[insert])[0], expected(600.0, 400.0, Mat3::IDENTITY));
}

#[test]
fn no_rectangle_is_ambiguous_block() {
    let mut s = Session::new();
    let mut block = Block::new("PLAIN");
    block.entities.push(Entity::new(Handle(1000), EntityKind::Line(Line { a: Vec2::ZERO.to3(0.0), b: Vec2::X.to3(0.0) })));
    s.doc_mut().unwrap().blocks.insert("PLAIN".into(), Arc::new(block));
    let insert = place(&mut s, "PLAIN", [1000.0, 1000.0], 1.0, 0.0, false, &[]);
    let candidates = detect(&s, &[insert]);
    assert_eq!(candidates[0].outline, None);
    assert!(has(&candidates[0], codes::FRAME_AMBIGUOUS, Severity::Block));
}

#[test]
fn duplicate_overlap_and_nested_are_flagged_never_dropped() {
    let mut s = setup();
    frame_block(s.doc_mut().unwrap(), "SMALL", 100.0, 100.0);
    let handles = [
        place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]),
        place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]),
        place(&mut s, "SMALL", [20.0, 20.0], 1.0, 0.0, false, &[]),
        place(&mut s, "A1", [800.0, 0.0], 1.0, 0.0, false, &[]),
        place(&mut s, "A1", [3000.0, 0.0], 1.0, 0.0, false, &[]),
    ];
    let candidates = detect(&s, &handles);
    assert_eq!(candidates.len(), 5);
    for (i, ((c, h), at)) in candidates.iter().zip(handles).zip([[0.0, 0.0], [0.0, 0.0], [20.0, 20.0], [800.0, 0.0], [3000.0, 0.0]]).enumerate() {
        assert_eq!(c.insert, h);
        let (w, h) = if i == 2 { (100.0, 100.0) } else { (841.0, 594.0) };
        assert_outline(c, expected(w, h, Mat3::translate(Vec2::new(at[0], at[1]))));
        assert_eq!(has(c, codes::FRAME_DUPLICATE, Severity::Warn), i == 1);
        assert_eq!(has(c, codes::FRAME_NESTED, Severity::Warn), i == 2);
        assert_eq!(has(c, codes::FRAME_OVERLAP, Severity::Warn), matches!(i, 0 | 1 | 3));
    }
    assert!(candidates[4].issues.is_empty());
}

#[test]
fn non_insert_and_unknown_handles_keep_their_slot() {
    let mut s = setup();
    let line = line(&mut s, Vec2::ZERO, Vec2::X);
    let insert = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let handles = [line, Handle::parse_hex("FFFFF").unwrap(), insert];
    let candidates = detect(&s, &handles);
    assert_eq!(candidates.iter().map(|c| c.insert).collect::<Vec<_>>(), handles);
    for c in &candidates[..2] {
        assert!(has(c, codes::FRAME_NOT_FOUND, Severity::Block));
        assert_eq!(c.outline, None);
    }
    assert_outline(&candidates[2], expected(841.0, 594.0, Mat3::IDENTITY));
}

#[test]
fn minsert_is_blocked() {
    let mut s = setup();
    let insert = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]);
    s.doc_mut()
        .unwrap()
        .modify_entity(insert, |e| {
            if let EntityKind::Insert(ins) = &mut e.kind {
                ins.cols = u32::MAX;
                ins.rows = u32::MAX;
            }
        })
        .unwrap();
    let candidates = detect(&s, &[insert]);
    assert_eq!(candidates[0].outline, None);
    assert!(has(&candidates[0], codes::FRAME_MINSERT, Severity::Block));
}

#[test]
fn hostile_inserts_never_panic() {
    let mut s = setup();
    for scale in [f64::NAN, 0.0, 1e308] {
        let insert = place(&mut s, "A1", [0.0, 0.0], scale, 0.0, false, &[]);
        let candidates = detect(&s, &[insert]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].outline, None);
        assert!(has(&candidates[0], codes::FRAME_AMBIGUOUS, Severity::Block));
    }
    for i in 0..40 {
        nest(s.doc_mut().unwrap(), &format!("DEEP-{i}"), &format!("DEEP-{}", i + 1), [0.0, 0.0], 0.0);
    }
    nest(s.doc_mut().unwrap(), "DEEP-40", "DEEP-0", [0.0, 0.0], 0.0);
    let deep = place(&mut s, "DEEP-0", [0.0, 0.0], 1.0, 0.0, false, &[]);
    assert!(has(&detect(&s, &[deep])[0], codes::FRAME_AMBIGUOUS, Severity::Block));
    nest(s.doc_mut().unwrap(), "CYCLE", "CYCLE", [0.0, 0.0], 0.0);
    let cycle = place(&mut s, "CYCLE", [0.0, 0.0], 1.0, 0.0, false, &[]);
    assert!(has(&detect(&s, &[cycle])[0], codes::FRAME_AMBIGUOUS, Severity::Block));

    let mut huge = polyline(1.0, 1.0);
    if let EntityKind::LwPolyline(p) = &mut huge {
        p.vertices = vec![PolyVertex::new(Vec2::ZERO); 1_000_000];
    }
    let mut block = Block::new("HUGE");
    block.entities.push(Entity::new(Handle(1_000_000), huge));
    s.doc_mut().unwrap().blocks.insert("HUGE".into(), Arc::new(block));
    let huge = place(&mut s, "HUGE", [0.0, 0.0], 1.0, 0.0, false, &[]);
    assert!(has(&detect(&s, &[huge])[0], codes::FRAME_AMBIGUOUS, Severity::Block));

    let handles: Vec<_> = (0..10_000).map(|i| place(&mut s, "A1", [f64::from(i) * 1000.0, 0.0], 1.0, 0.0, false, &[])).collect();
    let candidates = detect(&s, &handles);
    assert_eq!(candidates.len(), 10_000);
    for (i, c) in candidates.iter().enumerate() {
        assert_outline(c, expected(841.0, 594.0, Mat3::translate(Vec2::new(i as f64 * 1000.0, 0.0))));
        assert!(c.issues.is_empty());
    }
}

#[test]
fn is_frame_candidate_filters_lines_and_plain_blocks() {
    let mut s = setup();
    let line = line(&mut s, Vec2::ZERO, Vec2::X);
    s.doc_mut().unwrap().blocks.insert("PLAIN".into(), Arc::new(Block::new("PLAIN")));
    let plain = place(&mut s, "PLAIN", [2000.0, 0.0], 1.0, 0.0, false, &[]);
    let frame = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let d = s.doc().unwrap();
    let reg = FrameRegistry::builtin();
    assert!(!is_frame_candidate(d, line, &reg));
    assert!(!is_frame_candidate(d, plain, &reg));
    assert!(is_frame_candidate(d, frame, &reg));
    assert_outline(&detect(&s, &[frame])[0], expected(841.0, 594.0, Mat3::IDENTITY));
}

#[test]
fn template_rules_precede_generic_geometry_and_ties_block() {
    let mut s = setup();
    let d = s.doc_mut().unwrap();
    d.ensure_layer("FRAME");
    let h = d.new_handle();
    let mut smaller = Entity::new(h, polyline(420.0, 297.0));
    smaller.common.layer = "FRAME".into();
    Arc::make_mut(d.blocks.get_mut("A1").unwrap()).entities.push(smaller);
    let insert = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let mut reg = FrameRegistry::builtin();
    reg.templates[0].geometry = vec![GeometryRule::LayerRect { layer: "frame".into() }, GeometryRule::LargestClosedRect];
    assert_outline(&detect_frames(s.doc().unwrap(), &[insert], &reg)[0], expected(420.0, 297.0, Mat3::IDENTITY));
    reg.templates[0].geometry = vec![GeometryRule::LocalRect { min: [-10.0, -20.0], max: [90.0, 30.0] }];
    assert_outline(&detect_frames(s.doc().unwrap(), &[insert], &reg)[0], [[-10.0, -20.0], [90.0, -20.0], [90.0, 30.0], [-10.0, 30.0]]);
    let d = s.doc_mut().unwrap();
    let h = d.new_handle();
    let mut other = polyline(841.0, 593.0);
    other.transform(&Mat3::translate(Vec2::new(1000.0, 0.0)));
    Arc::make_mut(d.blocks.get_mut("A1").unwrap()).entities.push(Entity::new(h, other));
    let candidates = detect(&s, &[insert]);
    assert_eq!(candidates[0].outline, None);
    assert!(has(&candidates[0], codes::FRAME_AMBIGUOUS, Severity::Block));
}

#[test]
fn nonuniform_and_xref_frames_are_detected_read_only() {
    let mut s = setup();
    Arc::make_mut(s.doc_mut().unwrap().blocks.get_mut("A1").unwrap()).xref_path = Some("synthetic.dxf".into());
    let insert = place(&mut s, "A1", [0.0, 0.0], 2.0, 0.0, false, &[]);
    s.doc_mut()
        .unwrap()
        .modify_entity(insert, |e| {
            if let EntityKind::Insert(ins) = &mut e.kind {
                ins.scale.y = 3.0;
            }
        })
        .unwrap();
    let before = s.doc().unwrap().clone();
    let candidates = detect(&s, &[insert]);
    assert_outline(&candidates[0], expected(841.0, 594.0, Mat3::scale(2.0, 3.0)));
    assert!(has(&candidates[0], codes::FRAME_NONUNIFORM, Severity::Warn));
    assert_eq!(candidates[0].local_mm, [841.0, 594.0]);
    assert_eq!(s.doc().unwrap(), &before);
}

#[test]
fn closed_3d_rectangles_work_and_bulged_or_hidden_geometry_is_ignored() {
    let mut s = setup();
    let d = s.doc_mut().unwrap();
    d.ensure_layer("OFF");
    d.layer_mut("OFF").unwrap().on = false;
    let three_d = Entity::new(
        d.new_handle(),
        EntityKind::Polyline3d(cadcraft_doc::Polyline3d {
            points: [Vec2::ZERO, Vec2::new(841.0, 0.0), Vec2::new(841.0, 594.0), Vec2::new(0.0, 594.0)].map(|p| p.to3(0.0)).into(),
            closed: true,
        }),
    );
    let mut bulged = polyline(2000.0, 1500.0);
    if let EntityKind::LwPolyline(p) = &mut bulged {
        p.vertices[0].bulge = 0.5;
    }
    let bulged = Entity::new(d.new_handle(), bulged);
    let mut off = Entity::new(d.new_handle(), polyline(3000.0, 2500.0));
    off.common.layer = "OFF".into();
    let block = Arc::make_mut(d.blocks.get_mut("A1").unwrap());
    block.entities = Default::default();
    for entity in [three_d, bulged, off] {
        block.entities.push(entity);
    }
    let insert = place(&mut s, "A1", [0.0, 0.0], 1.0, 0.0, false, &[]);
    assert_outline(&detect(&s, &[insert])[0], expected(841.0, 594.0, Mat3::IDENTITY));
}

#[test]
fn over_limit_scan_keeps_every_input_slot_blocked() {
    let s = Session::new();
    let handles = vec![Handle(u64::MAX); 10_001];
    let candidates = detect(&s, &handles);
    assert_eq!(candidates.len(), handles.len());
    for (i, c) in candidates.iter().enumerate() {
        assert_eq!(c.insert, handles[i]);
        assert_eq!(c.outline, None);
        let code = if i < 10_000 { codes::FRAME_NOT_FOUND } else { codes::FRAME_AMBIGUOUS };
        assert!(has(c, code, Severity::Block));
    }
}

#[test]
fn outline_geometry_rejects_invalid_rectangles_and_uses_sat() {
    let a = expected(10.0, 10.0, Mat3::IDENTITY);
    let corners = a.map(|[x, y]| Vec2::new(x, y));
    assert_eq!(rect_of(&corners), Some(corners));
    let [p, q, r, t] = corners;
    assert_eq!(rect_of(&[p, q, r, t, p]), Some(corners));
    assert_eq!(rect_of(&[p, q, r]), None);
    assert_eq!(rect_of(&[p, q, q, t]), None);
    assert_eq!(rect_of(&[p, q, Vec2::new(9.0, 10.0), t]), None);
    assert_eq!(rect_of(&[p, q, Vec2::new(f64::NAN, 10.0), t]), None);
    assert_eq!(normalize_outline([r, q, p, t], Vec2::Y).0, a);
    assert_eq!(relation(&a, &a), Relation::Duplicate);
    let small = expected(2.0, 2.0, Mat3::translate(Vec2::new(1.0, 1.0)));
    assert_eq!(relation(&small, &a), Relation::AInsideB);
    assert_eq!(relation(&a, &small), Relation::BInsideA);
    assert_eq!(relation(&a, &expected(10.0, 10.0, Mat3::translate(Vec2::new(5.0, 0.0)))), Relation::Overlap);
    assert_eq!(relation(&a, &expected(10.0, 10.0, Mat3::translate(Vec2::new(10.0, 0.0)))), Relation::Disjoint);
    let rotated = expected(20.0, 1.0, Mat3::rotate(45.0_f64.to_radians()));
    let shifted = expected(20.0, 1.0, Mat3::translate(Vec2::new(-2.0, 2.0)).then_before(Mat3::rotate(45.0_f64.to_radians())));
    assert_eq!(relation(&rotated, &shifted), Relation::Disjoint);
}
