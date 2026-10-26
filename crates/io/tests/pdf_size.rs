//! Plotted PDF size: gapless and paper-dense linetypes plot continuous, solid hatches are filled
//! as their boundary loops (nonzero rule), touching segments share a subpath, near-collinear
//! points are dropped and coordinates have two decimals.

use cadcraft_color::Color;
use cadcraft_doc::{Common, Drawing, EntityKind, Hatch, HatchLoop, Line, Linetype, LwPolyline, Space};
use cadcraft_geom::{PolyVertex, Vec2, Vec3};
use cadcraft_io::PdfOptions;
use cadcraft_io::pdf::pdf;

/// A subpath of the page content and the operator that painted it.
#[derive(Debug)]
struct Subpath {
    pts: Vec<Vec2>,
    closed: bool,
    paint: String,
}

/// The page content streams of an uncompressed PDF. Splitting at `stream\n` also splits
/// `endstream\n`, so each stream ends at the `\nend` left over.
fn contents(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    text.split("stream\n")
        .skip(1)
        .filter_map(|s| s.split_once("\nend").map(|(c, _)| c))
        .filter(|c| c.starts_with("q\n1 J 1 j\n"))
        .map(str::to_string)
        .collect()
}

/// The painted subpaths of a content stream (the clip rectangle and `re` marks left out).
fn subpaths(content: &str) -> Vec<Subpath> {
    let mut out: Vec<Subpath> = Vec::new();
    let mut open: Vec<Subpath> = Vec::new();
    let mut nums: Vec<f64> = Vec::new();
    for tok in content.split_whitespace() {
        if let Ok(v) = tok.parse::<f64>() {
            nums.push(v);
            continue;
        }
        match tok {
            "m" | "l" => {
                let [.., x, y] = nums[..] else { panic!("{tok} without coordinates") };
                if tok == "m" {
                    open.push(Subpath { pts: Vec::new(), closed: false, paint: String::new() });
                }
                open.last_mut().expect("l after m").pts.push(Vec2::new(x, y));
            }
            "h" => open.last_mut().expect("h after m").closed = true,
            "S" | "f" | "f*" => out.extend(open.drain(..).map(|s| Subpath { paint: tok.into(), ..s })),
            "n" => open.clear(),
            _ => {}
        }
        nums.clear();
    }
    out
}

fn page_subpaths(bytes: &[u8]) -> Vec<Subpath> {
    let pages = contents(bytes);
    assert_eq!(pages.len(), 1);
    subpaths(&pages[0])
}

/// A4 landscape, no margins, at `scale` paper mm per drawing unit; the drawing's lower-left
/// corner goes to the page's.
fn options(scale: f64) -> PdfOptions {
    PdfOptions {
        paper: Some("A4".into()),
        landscape: Some(true),
        fit: Some(false),
        scale: Some(scale),
        center: Some(false),
        margins: Some([0.0; 4]),
        lineweights: Some(false),
        compress: false,
        ..PdfOptions::default()
    }
}

fn line(d: &mut Drawing, linetype: &str, a: [f64; 2], b: [f64; 2]) {
    let common = Common { linetype: linetype.into(), ..Common::default() };
    d.add(&Space::Model, common, EntityKind::Line(Line { a: Vec3::new(a[0], a[1], 0.0), b: Vec3::new(b[0], b[1], 0.0) })).unwrap();
}

fn drawing_with(linetypes: &[(&str, &[f64])]) -> Drawing {
    let mut d = Drawing::new_metric();
    for (name, dashes) in linetypes {
        d.linetypes.push(Linetype::simple(name, "", dashes));
    }
    d
}

#[test]
fn gapless_linetype_plots_as_one_segment() {
    let mut d = drawing_with(&[("KETCAU-1", &[1.0, 1.0]), ("DOTTED-ON", &[1.0, 0.0])]);
    line(&mut d, "KETCAU-1", [10.0, 10.0], [110.0, 10.0]);
    line(&mut d, "DOTTED-ON", [10.0, 50.0], [110.0, 50.0]);
    let paths = page_subpaths(&pdf(&d, &Space::Model, &options(1.0)).unwrap());
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.iter().all(|p| p.pts.len() == 2 && p.paint == "S"), "{paths:?}");
    let text = String::from_utf8_lossy(&pdf(&d, &Space::Model, &options(1.0)).unwrap()).into_owned();
    assert!(!text.contains(" re f"), "a dot on a gapless line is not drawn on its own");
}

#[test]
fn dense_linetype_on_paper_plots_continuous_and_normal_dashes_stay() {
    // 2 units per repeat: 2 mm on paper at 1:1 (dashed), 0.02 mm at 1:100 (continuous).
    let mut d = drawing_with(&[("DASHED", &[1.0, -1.0])]);
    line(&mut d, "DASHED", [10.0, 10.0], [110.0, 10.0]);
    assert_eq!(page_subpaths(&pdf(&d, &Space::Model, &options(1.0)).unwrap()).len(), 50);
    let mut far = drawing_with(&[("DASHED", &[1.0, -1.0])]);
    line(&mut far, "DASHED", [1000.0, 1000.0], [11000.0, 1000.0]);
    let paths = page_subpaths(&pdf(&far, &Space::Model, &options(0.01)).unwrap());
    assert_eq!(paths.len(), 1, "{paths:?}");
    // 0.12 mm on paper is above the 0.1 mm threshold: still dashed.
    let mut fine = drawing_with(&[("FINE", &[0.06, -0.06])]);
    line(&mut fine, "FINE", [10.0, 10.0], [22.0, 10.0]);
    assert_eq!(page_subpaths(&pdf(&fine, &Space::Model, &options(1.0)).unwrap()).len(), 100);
}

fn square(lo: f64, hi: f64, ccw: bool) -> HatchLoop {
    let mut v: Vec<PolyVertex> = [(lo, lo), (hi, lo), (hi, hi), (lo, hi)].into_iter().map(|(x, y)| PolyVertex::new(Vec2::new(x, y))).collect();
    if !ccw {
        v.reverse();
    }
    HatchLoop { vertices: v, outer: false, sources: Vec::new() }
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

/// Nonzero winding number of `p` for closed subpaths.
fn winding(paths: &[&Subpath], p: Vec2) -> i32 {
    let mut w = 0;
    for s in paths {
        let n = s.pts.len();
        for i in 0..n {
            let (a, b) = (s.pts[i], s.pts[(i + 1) % n]);
            let side = (b - a).cross(p - a);
            if a.y <= p.y && b.y > p.y && side > 0.0 {
                w += 1;
            } else if a.y > p.y && b.y <= p.y && side < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

#[test]
fn solid_hatch_with_islands_is_filled_as_oriented_boundary_loops() {
    let mut d = Drawing::new_metric();
    // All three loops wound the same way: the nonzero fill must still leave the island empty.
    d.add(&Space::Model, Common::default(), solid(vec![square(0.0, 100.0, true), square(20.0, 80.0, true), square(40.0, 60.0, true)])).unwrap();
    let paths = page_subpaths(&pdf(&d, &Space::Model, &options(1.0)).unwrap());
    assert_eq!(paths.len(), 3, "one closed subpath per loop, no triangles: {paths:?}");
    assert!(paths.iter().all(|p| p.closed && p.paint == "f" && p.pts.len() == 4), "{paths:?}");
    let filled: Vec<&Subpath> = paths.iter().collect();
    let k = 72.0 / 25.4;
    let at = |x: f64, y: f64| winding(&filled, Vec2::new(x * k, y * k));
    assert_ne!(at(10.0, 10.0), 0, "outer ring is filled");
    assert_eq!(at(30.0, 30.0), 0, "the island stays empty");
    assert_ne!(at(50.0, 50.0), 0, "the island in the island is filled");
    assert_eq!(at(200.0, 150.0), 0);
}

#[test]
fn touching_segments_share_one_subpath() {
    let mut d = Drawing::new_metric();
    for (a, b) in [([10.0, 10.0], [50.0, 10.0]), ([50.0, 10.0], [50.0, 50.0]), ([50.0, 50.0], [90.0, 50.0])] {
        line(&mut d, "Continuous", a, b);
    }
    // Same end point, other colour: its own path.
    let red = Common { color: Color::Index(1), ..Common::default() };
    d.add(&Space::Model, red, EntityKind::Line(Line { a: Vec3::new(90.0, 50.0, 0.0), b: Vec3::new(90.0, 90.0, 0.0) })).unwrap();
    let paths = page_subpaths(&pdf(&d, &Space::Model, &options(1.0)).unwrap());
    assert_eq!(paths.iter().map(|p| p.pts.len()).collect::<Vec<_>>(), [4, 2], "{paths:?}");
}

#[test]
fn near_collinear_points_are_dropped_and_corners_kept() {
    let mut d = Drawing::new_metric();
    // 0.002 mm off the line is dropped, a 0.02 mm bump and the corner are kept.
    let pts = [(0.0, 0.0), (10.0, 0.002), (20.0, 0.0), (30.0, 0.0), (30.0, 10.0), (20.0, 10.02), (10.0, 10.0)];
    let vertices = pts.iter().map(|(x, y)| PolyVertex::new(Vec2::new(*x, *y))).collect();
    d.add(
        &Space::Model,
        Common::default(),
        EntityKind::LwPolyline(LwPolyline { vertices, closed: false, const_width: 0.0, elevation: 0.0, plinegen: false }),
    )
    .unwrap();
    let paths = page_subpaths(&pdf(&d, &Space::Model, &options(1.0)).unwrap());
    let k = 72.0 / 25.4;
    let mm: Vec<(f64, f64)> = paths[0].pts.iter().map(|p| ((p.x / k * 100.0).round() / 100.0, (p.y / k * 100.0).round() / 100.0)).collect();
    assert_eq!(paths.len(), 1);
    assert_eq!(mm, [(0.0, 0.0), (30.0, 0.0), (30.0, 10.0), (20.0, 10.02), (10.0, 10.0)]);
}

#[test]
fn path_coordinates_have_two_decimals() {
    let mut d = Drawing::new_metric();
    line(&mut d, "Continuous", [10.123_456, 10.654_321], [77.777_777, 33.333_333]);
    d.add(&Space::Model, Common::default(), solid(vec![square(100.111, 140.777, true)])).unwrap();
    let content = contents(&pdf(&d, &Space::Model, &options(1.0)).unwrap()).remove(0);
    let path_lines: Vec<&str> = content.lines().filter(|l| l.ends_with(" m") || l.ends_with(" l")).collect();
    assert!(path_lines.len() >= 6, "{content}");
    for l in path_lines {
        for n in l.split_whitespace().filter(|t| t.parse::<f64>().is_ok()) {
            assert!(n.split_once('.').is_none_or(|(_, f)| f.len() <= 2), "{n} in {l:?}");
        }
    }
}

/// A closed loop of `n` vertices wiggling around a circle (a traced signature's density).
fn wiggly_loop(center: Vec2, r: f64, n: usize) -> HatchLoop {
    let vertices = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let rr = r * (1.0 + 0.05 * (a * 37.0).sin());
            PolyVertex::new(center + Vec2::new(a.cos(), a.sin()) * rr)
        })
        .collect();
    HatchLoop { vertices, outer: true, sources: Vec::new() }
}

#[test]
fn gapless_lines_and_a_dense_solid_hatch_stay_small() {
    let mut d = drawing_with(&[("KETCAU-1", &[1.0, 1.0])]);
    // 1:100 on A4: 100 lines of 10 m, 5000 dash repeats each.
    for i in 0..100 {
        let y = f64::from(i) * 150.0;
        line(&mut d, "KETCAU-1", [0.0, y], [10_000.0, y]);
    }
    d.add(&Space::Model, Common::default(), solid(vec![wiggly_loop(Vec2::new(20_000.0, 10_000.0), 4000.0, 5000)])).unwrap();
    let paths = page_subpaths(&pdf(&d, &Space::Model, &options(0.01)).unwrap());
    assert_eq!(paths.iter().filter(|p| p.paint == "S").count(), 100);
    let fills: Vec<&Subpath> = paths.iter().filter(|p| p.paint == "f").collect();
    assert!(fills.len() == 1 && fills[0].closed && fills[0].pts.len() > 1000, "the hatch is one loop");
    // 5.3 MB before: every dash its own subpath, the hatch as slab triangles; 8 KB now.
    let bytes = pdf(&d, &Space::Model, &PdfOptions { compress: true, ..options(0.01) }).unwrap();
    assert!(bytes.len() < 40_000, "{} bytes", bytes.len());
}
