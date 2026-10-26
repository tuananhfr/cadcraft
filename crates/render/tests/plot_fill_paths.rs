//! Solid hatch fills carry their boundary loops in plots ([`DisplayList::fill_paths`]), kept in
//! step with viewport clipping, rotation and plot style fill patterns.

use std::sync::Arc;

use cadcraft_color::Color;
use cadcraft_doc::{Common, Drawing, EntityKind, FillStyle, Hatch, HatchLoop, PlotStyle, PlotStyleTable, Space, Viewport};
use cadcraft_geom::{Bounds2, PolyVertex, Vec2, Vec3};
use cadcraft_render::{DisplayList, Kind, Options, VIEWPORT_CONTENT, build, build_plot, pen};

fn square(lo: f64, hi: f64) -> HatchLoop {
    let v = [(lo, lo), (hi, lo), (hi, hi), (lo, hi)].into_iter().map(|(x, y)| PolyVertex::new(Vec2::new(x, y))).collect();
    HatchLoop { vertices: v, outer: lo == 0.0, sources: Vec::new() }
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

fn red() -> Common {
    Common { color: Color::Index(1), ..Common::default() }
}

/// A red square 0..10 with the island 4..6.
fn drawing() -> Drawing {
    let mut d = Drawing::new_metric();
    d.add(&Space::Model, red(), solid(vec![square(0.0, 10.0), square(4.0, 6.0)])).unwrap();
    d
}

fn signed_area(l: &[Vec2]) -> f64 {
    l.iter().zip(l.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum::<f64>() / 2.0
}

fn tri_prim(l: &DisplayList, prim: u32) -> bool {
    l.prims.get(prim as usize).is_some_and(|p| p.kind == Kind::Tris)
}

#[test]
fn plots_carry_solid_fills_as_oriented_loops() {
    let d = drawing();
    assert!(build(&d, &Space::Model, &Options::default()).fill_paths.is_empty(), "screen builds have no fill paths");
    let l = build_plot(&d, &Space::Model, &Options::default());
    assert_eq!(l.fill_paths.len(), 1);
    let f = &l.fill_paths[0];
    assert!(tri_prim(&l, f.prim));
    let areas: Vec<f64> = f.loops.iter().map(|l| signed_area(l)).collect();
    assert_eq!(areas, [100.0, -4.0]);
}

#[test]
fn rotating_a_plot_rotates_its_fill_paths() {
    let mut l = build_plot(&drawing(), &Space::Model, &Options::default());
    l.rotate_about(Vec2::ZERO, std::f64::consts::FRAC_PI_2);
    let outer = &l.fill_paths[0].loops[0];
    assert!(outer.iter().all(|p| p.x <= 1e-9 && p.y >= -1e-9), "{outer:?}");
    assert!((signed_area(outer) - 100.0).abs() < 1e-9);
}

#[test]
fn viewport_clips_fill_paths_to_its_rectangle() {
    let mut d = drawing();
    // 1:1 viewport onto model x 5..15, y -5..15, at paper 105..115, 95..115: cuts the square in half.
    let vp = Viewport {
        center: Vec3::new(110.0, 105.0, 0.0),
        width: 10.0,
        height: 20.0,
        view_center: Vec2::new(10.0, 5.0),
        view_height: 20.0,
        id: 2,
        locked: false,
        frozen_layers: Vec::new(),
        layer_colors: Vec::new(),
        clip: None,
        on: true,
    };
    let space = Space::Paper("Layout1".into());
    d.add(&space, Common::default(), EntityKind::Viewport(vp)).unwrap();
    let l = build_plot(&d, &space, &Options::default());
    assert_eq!(l.fill_paths.len(), 1);
    let f = &l.fill_paths[0];
    assert!(tri_prim(&l, f.prim) && l.prims[f.prim as usize].handle == VIEWPORT_CONTENT);
    let rect = Bounds2::new(Vec2::new(105.0, 95.0), Vec2::new(115.0, 115.0)).expand(1e-9);
    assert!(f.loops.iter().flatten().all(|p| rect.contains(*p)), "{:?}", f.loops);
    // Right half of the square (50) and of the island (2).
    let areas: Vec<f64> = f.loops.iter().map(|l| (signed_area(l) * 1e6).round() / 1e6).collect();
    assert_eq!(areas, [50.0, -2.0]);
}

/// Colour 1 prints with `fill` and 50 % screening.
fn table(fill: FillStyle) -> Options {
    let t = PlotStyleTable::color_dependent("fills.ctb", "", |i| (i == 1).then(|| PlotStyle { fill, screening: 50, ..PlotStyle::default() }));
    Options { plot_style_table: Some(Arc::new(t)), ..Options::default() }
}

#[test]
fn plot_style_fill_patterns_drop_the_fill_path_and_screening_keeps_it() {
    let d = drawing();
    let mut patterned = build_plot(&d, &Space::Model, &table(FillStyle::Checkerboard));
    pen::apply(&mut patterned, 1.0);
    assert!(patterned.fill_paths.is_empty(), "the pattern's pieces are drawn instead");
    let mut screened = build_plot(&d, &Space::Model, &table(FillStyle::Object));
    pen::apply(&mut screened, 1.0);
    assert_eq!(screened.fill_paths.len(), 1);
    let plain = build_plot(&d, &Space::Model, &Options::default());
    assert_ne!(screened.prims[0].color, plain.prims[0].color, "screening still lightens the fill");
}
