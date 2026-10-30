//! Picking a block reference under the cursor (the hover highlight picks the same way): on its line
//! work, inside its solid hatches as they are filled, and fast enough to run on every pointer move
//! over a title block full of hatches.

use cadcraft_engine::doc::{Block, Common, Drawing, Entity, EntityKind, Handle, Hatch, HatchLoop, Insert, Line, Space};
use cadcraft_engine::geom::{PolyVertex, Vec2, Vec3};
use cadcraft_engine::select::pick;
use std::sync::Arc;

fn ring(pts: &[Vec2], outer: bool) -> HatchLoop {
    HatchLoop { vertices: pts.iter().map(|p| PolyVertex::new(*p)).collect(), outer, sources: Vec::new() }
}

fn square(lo: f64, hi: f64, outer: bool) -> HatchLoop {
    ring(&[Vec2::new(lo, lo), Vec2::new(hi, lo), Vec2::new(hi, hi), Vec2::new(lo, hi)], outer)
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

fn line(a: (f64, f64), b: (f64, f64)) -> EntityKind {
    EntityKind::Line(Line { a: Vec3::new(a.0, a.1, 0.0), b: Vec3::new(b.0, b.1, 0.0) })
}

fn insert_of(block: &str, at: Vec2, scale: Vec3, rotation: f64) -> EntityKind {
    EntityKind::Insert(Insert {
        block: block.into(),
        insert: Vec3::new(at.x, at.y, 0.0),
        scale,
        rotation,
        attribs: vec![],
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
    })
}

/// A drawing holding block "B" (`contents`) and one reference to it; returns the reference.
fn one_insert(contents: Vec<EntityKind>, at: Vec2, scale: Vec3, rotation: f64) -> (Drawing, Handle) {
    let mut d = Drawing::new_imperial();
    let mut blk = Block::new("B");
    for (i, k) in contents.into_iter().enumerate() {
        blk.entities.push(Entity::new(Handle(i as u64 + 1), k));
    }
    d.blocks.insert("B".into(), Arc::new(blk));
    let h = d.add(&Space::Model, Common::default(), insert_of("B", at, scale, rotation)).unwrap();
    (d, h)
}

fn unit() -> Vec3 {
    Vec3::new(1.0, 1.0, 1.0)
}

#[test]
fn inside_a_solid_hatch_of_a_block_picks_the_insert() {
    let (d, h) = one_insert(vec![line((0.0, 0.0), (100.0, 0.0)), solid(vec![square(20.0, 30.0, true)])], Vec2::ZERO, unit(), 0.0);
    assert_eq!(pick(&d, &Space::Model, Vec2::new(25.0, 25.0), 0.5), Some(h));
    assert_eq!(pick(&d, &Space::Model, Vec2::new(26.3, 22.1), 0.5), Some(h));
}

#[test]
fn on_a_line_of_a_rotated_mirrored_scaled_insert_picks_it() {
    let at = Vec2::new(500.0, 200.0);
    let (scale, rotation) = (Vec3::new(-2.0, 3.0, 1.0), 0.5);
    let (d, h) = one_insert(vec![line((0.0, 0.0), (100.0, 0.0)), line((0.0, 0.0), (0.0, 100.0)), solid(vec![square(20.0, 30.0, true)])], at, scale, rotation);
    let m = match &d.entity(h).unwrap().kind {
        EntityKind::Insert(ins) => ins.transform(Vec2::ZERO),
        _ => unreachable!(),
    };
    let world = |x: f64, y: f64| m.apply(Vec2::new(x, y));
    assert_eq!(pick(&d, &Space::Model, world(50.0, 0.0), 0.5), Some(h), "on the first line");
    assert_eq!(pick(&d, &Space::Model, world(0.0, 70.0), 0.5), Some(h), "on the second line");
    assert_eq!(pick(&d, &Space::Model, world(25.0, 25.0), 0.5), Some(h), "inside the hatch");
    assert_eq!(pick(&d, &Space::Model, world(60.0, 60.0), 0.5), None, "inside the bounds, on nothing");
}

#[test]
fn inside_a_hatch_island_of_a_block_does_not_pick_the_insert() {
    let (d, h) = one_insert(vec![solid(vec![square(20.0, 30.0, true), square(23.0, 27.0, false)])], Vec2::ZERO, unit(), 0.0);
    assert_eq!(pick(&d, &Space::Model, Vec2::new(21.0, 21.0), 0.2), Some(h), "in the fill");
    assert_eq!(pick(&d, &Space::Model, Vec2::new(25.0, 25.5), 0.2), None, "in the island");
}

/// A wavy ring of `n` points around `c`, like a filled logo in a title block.
fn wavy(c: Vec2, r: f64, n: usize) -> HatchLoop {
    let pts: Vec<Vec2> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            let rr = r * (1.0 + 0.15 * (37.0 * a).sin());
            c + Vec2::new(rr * a.cos(), rr * 0.6 * a.sin())
        })
        .collect();
    ring(&pts, true)
}

/// Title block "TB" (A1 frame, 7 solid hatches of 3000 points in the corner) inserted 15 times,
/// plus `filler` short lines below them.
fn title_blocks(filler: u32) -> Drawing {
    let mut d = Drawing::new_imperial();
    let mut blk = Block::new("TB");
    let frame = [((0.0, 0.0), (841.0, 0.0)), ((841.0, 0.0), (841.0, 594.0)), ((841.0, 594.0), (0.0, 594.0)), ((0.0, 594.0), (0.0, 0.0))];
    let mut n = 1;
    for (a, b) in frame {
        blk.entities.push(Entity::new(Handle(n), line(a, b)));
        n += 1;
    }
    for k in 0..7 {
        blk.entities.push(Entity::new(Handle(n), solid(vec![wavy(Vec2::new(620.0 + 28.0 * f64::from(k), 40.0), 12.0, 3000)])));
        n += 1;
    }
    d.blocks.insert("TB".into(), Arc::new(blk));
    for i in 0..15 {
        let at = Vec2::new(f64::from(i % 5) * 900.0, f64::from(i / 5) * 650.0);
        d.add(&Space::Model, Common::default(), insert_of("TB", at, unit(), 0.0)).unwrap();
    }
    for i in 0..filler {
        let y = -1000.0 - f64::from(i);
        d.add(&Space::Model, Common::default(), line((0.0, y), (10.0, y))).unwrap();
    }
    d
}

/// Milliseconds per pick inside the title block away from its hatches, and on a hatch.
fn hover_pick_ms(d: &Drawing) -> (f64, f64) {
    let time = |label: &str, pts: &[Vec2]| {
        let t = std::time::Instant::now();
        let hits = pts.iter().filter(|p| pick(d, &Space::Model, **p, 2.0).is_some()).count();
        let ms = t.elapsed().as_secs_f64() * 1000.0 / pts.len() as f64;
        println!("{label}: {ms:.3} ms/pick, {hits}/{} hits", pts.len());
        ms
    };
    let away: Vec<Vec2> = (0..200).map(|i| Vec2::new(300.0 + f64::from(i % 20) * 5.0, 300.0 + f64::from(i / 20) * 5.0)).collect();
    let on_hatch: Vec<Vec2> = (0..200).map(|i| Vec2::new(616.0 + f64::from(i % 20) * 0.4, 36.0 + f64::from(i / 20) * 0.8)).collect();
    (time("inside the title block, away from the hatches", &away), time("on a hatch", &on_hatch))
}

#[test]
#[ignore = "timing; run in release: cargo test --release -p cadcraft-engine --test pick_insert -- --ignored --nocapture"]
fn hover_pick_over_a_heavy_title_block() {
    // Past spatial::MIN_INDEXED, as a drawing with title blocks usually is.
    let (a, b) = hover_pick_ms(&title_blocks(300));
    assert!(a < 2.0 && b < 2.0, "{a:.3} / {b:.3} ms per pick");
}

#[test]
#[ignore = "timing; run in release: cargo test --release -p cadcraft-engine --test pick_insert -- --ignored --nocapture"]
fn hover_pick_over_heavy_title_blocks_in_a_small_drawing() {
    // Too small to index: every pick measures all 15 blocks (4-8 ms on a dev machine). The bound
    // leaves room for a loaded machine and still fails at the 26 / 57 ms picks had before.
    let (a, b) = hover_pick_ms(&title_blocks(0));
    assert!(a < 20.0 && b < 20.0, "{a:.3} / {b:.3} ms per pick");
}
