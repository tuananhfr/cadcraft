//! Builds for picking under the cursor: only the block contents near the pick box are built, and
//! solid hatch fills come back as their boundary loops instead of triangles.

use cadcraft_doc::{Block, Common, Drawing, Entity, EntityKind, Handle, Hatch, HatchLoop, Insert, Line, Space};
use cadcraft_geom::{Bounds2, PolyVertex, Vec2, Vec3};
use cadcraft_render::{DisplayList, Kind, Options, build_entities};
use std::sync::Arc;

fn square(lo: f64, hi: f64) -> HatchLoop {
    let v = [(lo, lo), (hi, lo), (hi, hi), (lo, hi)].into_iter().map(|(x, y)| PolyVertex::new(Vec2::new(x, y))).collect();
    HatchLoop { vertices: v, outer: true, sources: Vec::new() }
}

fn solid(loops: Vec<HatchLoop>) -> EntityKind {
    EntityKind::Hatch(Hatch {
        pattern: "SOLID".into(),
        solid: true,
        loops,
        scale: 1.0,
        angle: 0.0,
        associative: false,
        style: 0,
        elevation: 0.0,
        gradient: None,
        origin: Vec2::ZERO,
        background: None,
        pattern_lines: Vec::new(),
    })
}

fn insert(block: &str, at: Vec2, rotation: f64, cols: u32, col_spacing: f64) -> EntityKind {
    EntityKind::Insert(Insert {
        block: block.into(),
        insert: Vec3::new(at.x, at.y, 0.0),
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation,
        attribs: vec![],
        cols,
        rows: 1,
        col_spacing,
        row_spacing: 0.0,
    })
}

/// Block "B": a line (0,0)-(10,0) and a solid hatch over the square 20..30.
fn with_block_b() -> Drawing {
    let mut d = Drawing::new_imperial();
    let mut blk = Block::new("B");
    blk.entities.push(Entity::new(Handle(1), EntityKind::Line(Line { a: Vec3::new(0.0, 0.0, 0.0), b: Vec3::new(10.0, 0.0, 0.0) })));
    blk.entities.push(Entity::new(Handle(2), solid(vec![square(20.0, 30.0)])));
    d.blocks.insert("B".into(), Arc::new(blk));
    d
}

fn pick_build(d: &Drawing, near: Vec2) -> DisplayList {
    let e = d.model.iter().last().expect("insert").clone();
    let opts = Options { near: Some(Bounds2::new(near, near).expand(1.0)), fill_loops: true, text: false, ..Default::default() };
    build_entities(d, std::iter::once(e.as_ref()), &opts)
}

fn has_tris(l: &DisplayList) -> bool {
    l.prims.iter().any(|p| p.kind == Kind::Tris)
}

fn fill_bounds(l: &DisplayList) -> Vec<Bounds2> {
    l.hit_fills.iter().map(|loops| Bounds2::from_points(loops.iter().flatten().copied())).collect()
}

fn close(a: Bounds2, b: Bounds2) -> bool {
    a.min.dist(b.min) < 1e-6 && a.max.dist(b.max) < 1e-6
}

#[test]
fn near_box_away_from_a_block_hatch_builds_none_of_it() {
    let mut d = with_block_b();
    d.add(&Space::Model, Common::default(), insert("B", Vec2::ZERO, 0.0, 1, 0.0)).unwrap();
    let l = pick_build(&d, Vec2::new(5.0, 0.0));
    assert!(l.prims.iter().any(|p| p.kind == Kind::Polyline), "the line under the box is built");
    assert!(!has_tris(&l), "no triangles");
    assert!(l.hit_fills.is_empty(), "the hatch away from the box is left out");
}

#[test]
fn fill_loops_gives_hatch_loops_instead_of_triangles() {
    let mut d = with_block_b();
    // Turned a quarter turn about (100, 0): the square 20..30 lands on x 70..80, y 20..30.
    d.add(&Space::Model, Common::default(), insert("B", Vec2::new(100.0, 0.0), std::f64::consts::FRAC_PI_2, 1, 0.0)).unwrap();
    let l = pick_build(&d, Vec2::new(75.0, 25.0));
    assert!(!has_tris(&l), "no triangles");
    let b = fill_bounds(&l);
    assert_eq!(b.len(), 1, "one fill: {b:?}");
    assert!(close(b[0], Bounds2::new(Vec2::new(70.0, 20.0), Vec2::new(80.0, 30.0))), "{:?}", b[0]);
}

#[test]
fn near_applies_inside_nested_blocks_and_minsert_copies() {
    let mut d = with_block_b();
    let mut outer = Block::new("N");
    outer.entities.push(Entity::new(Handle(3), insert("B", Vec2::ZERO, 0.0, 1, 0.0)));
    d.blocks.insert("N".into(), Arc::new(outer));
    // Two copies 100 apart: only the second copy's hatch (x 120..130) is under the box.
    d.add(&Space::Model, Common::default(), insert("N", Vec2::ZERO, 0.0, 2, 100.0)).unwrap();
    let l = pick_build(&d, Vec2::new(125.0, 25.0));
    let b = fill_bounds(&l);
    assert_eq!(b.len(), 1, "one fill: {b:?}");
    assert!(close(b[0], Bounds2::new(Vec2::new(120.0, 20.0), Vec2::new(130.0, 30.0))), "{:?}", b[0]);
    assert!(!l.prims.iter().any(|p| p.kind == Kind::Polyline), "neither copy's line is near the box");
}

#[test]
fn without_the_pick_options_a_block_hatch_still_draws_as_triangles() {
    let mut d = with_block_b();
    d.add(&Space::Model, Common::default(), insert("B", Vec2::ZERO, 0.0, 1, 0.0)).unwrap();
    let e = d.model.iter().last().unwrap().clone();
    let l = build_entities(&d, std::iter::once(e.as_ref()), &Options::default());
    assert!(has_tris(&l), "screen builds keep triangles");
    assert!(l.hit_fills.is_empty());
}
