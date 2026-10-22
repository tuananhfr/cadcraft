use cadcraft_doc::{Common, Drawing, EntityKind, Layer, Line, Space};
use cadcraft_geom::{Bounds2, Vec2};
use cadcraft_io::pdf::{PlotFrame, pdf};
use cadcraft_io::{IoError, PdfOptions, plotter};
use serde_json::{Value, json};

fn outline(center: Vec2) -> [Vec2; 4] {
    [Vec2::new(-210.0, -148.5), Vec2::new(210.0, -148.5), Vec2::new(210.0, 148.5), Vec2::new(-210.0, 148.5)]
        .map(|p| center + p.rotate(std::f64::consts::PI / 6.0))
}

fn vertices(points: &[Vec2]) -> Value {
    json!(points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>())
}

fn frame_drawing(points: &[Vec2], space: &Space) -> Drawing {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer::new("FRAME"));
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        let line = EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) });
        d.add(space, Common { layer: "FRAME".into(), ..Default::default() }, line).unwrap();
    }
    d
}

fn plotted(d: &Drawing, opts: Value) -> String {
    let mut opts = opts;
    opts.as_object_mut().unwrap().insert("compress".into(), json!(false));
    String::from_utf8_lossy(&plotter::plot(d, &Space::Model, &opts).unwrap()).into_owned()
}

fn media_boxes(pdf: &str) -> Vec<String> {
    pdf.split("/MediaBox ").skip(1).map(|s| s.split(']').next().unwrap().to_string() + "]").collect()
}

fn assert_frame_edges(center: Vec2) {
    let points = outline(center);
    let d = frame_drawing(&points, &Space::Model);
    let opts = json!({"pages": [{"outline": vertices(&points), "width": 420, "height": 297}], "margins": [0, 0, 0, 0]});
    let text = plotted(&d, opts);
    let layer = d.layers.iter().position(|l| l.name == "FRAME").unwrap();
    let mut marked = false;
    let mut bounds = Bounds2::EMPTY;
    let mut count = 0;
    for line in text.lines() {
        if line == format!("/OC /L{layer} BDC") {
            marked = true;
        } else if line == "EMC" {
            marked = false;
        } else if marked && (line.ends_with(" m") || line.ends_with(" l")) {
            let mut coords = line.split_whitespace();
            let x = coords.next().unwrap().parse().unwrap();
            let y = coords.next().unwrap().parse().unwrap();
            bounds.add(Vec2::new(x, y));
            count += 1;
        }
    }
    assert_eq!(count, 8, "only the four frame segments: {text}");
    for (got, want) in [(bounds.min.x, 0.0), (bounds.min.y, 0.0), (bounds.max.x, 1190.55), (bounds.max.y, 841.89)] {
        assert!((got - want).abs() < 0.05, "{bounds:?}, expected {want}, got {got}");
    }
}

#[test]
fn pages_outline_rotated_30_maps_frame_to_page_edges() {
    assert_frame_edges(Vec2::ZERO);
}

#[test]
fn pages_custom_width_height_per_page_have_own_mediabox() {
    let d = Drawing::new_metric();
    let text = plotted(
        &d,
        json!({"pages": [
            {"window": [[0, 0], [841, 2400]], "width": 841, "height": 2400},
            {"window": [[0, 0], [297, 420]], "width": 297, "height": 420}
        ]}),
    );
    assert_eq!(media_boxes(&text), ["[0 0 2383.937 6803.15]", "[0 0 841.89 1190.551]"]);
}

#[test]
fn pages_layout_item_uses_its_page_setup() {
    let mut d = Drawing::new_metric();
    let layout = d.layouts.iter_mut().find(|l| l.name == "Layout1").unwrap();
    layout.page.width_mm = 297.0;
    layout.page.height_mm = 420.0;
    layout.page.landscape = false;
    let paper = Space::Paper("Layout1".into());
    let line = Line { a: Vec2::new(10.0, 20.0).to3(0.0), b: Vec2::new(30.0, 40.0).to3(0.0) };
    d.add(&paper, Common::default(), EntityKind::Line(line)).unwrap();
    let expected = String::from_utf8_lossy(&pdf(&d, &Space::Paper("Layout1".into()), &PdfOptions::default()).unwrap()).into_owned();
    let text = plotted(&d, json!({"pages": [{"layout": "layout1"}]}));
    assert_eq!(media_boxes(&text), media_boxes(&expected));
    let content = |pdf: &str| pdf.split("stream\n").nth(1).unwrap().split("endstream").next().unwrap().to_string();
    assert_eq!(content(&text), content(&expected));
    assert!(text.contains("/P (Layout1)"));
    assert!(plotter::plot(&d, &Space::Model, &json!({"pages": [{"layout": "missing"}]})).is_err());
}

#[test]
fn pages_label_becomes_page_label() {
    let d = Drawing::new_metric();
    let text = plotted(&d, json!({"pages": [{"window": [[0, 0], [420, 297]], "label": "KT-01"}]}));
    assert!(text.contains("/PageLabels << /Nums [0 << /P (KT-01) >>] >>"), "{text}");
    assert!(PdfOptions::default().frame.is_none());
    assert!(PdfOptions::default().label.is_none());
}

#[test]
fn pages_require_exactly_one_area() {
    let d = Drawing::new_metric();
    let outline = vertices(&outline(Vec2::ZERO));
    for page in [
        json!({"window": [[0, 0], [420, 297]], "outline": outline}),
        json!({}),
        json!({"layout": "Layout1", "outline": outline}),
        json!({"window": [[0, 0], [420, 297]], "layout": "Layout1"}),
    ] {
        let error = plotter::plot(&d, &Space::Model, &json!({"pages": [page]})).unwrap_err();
        assert!(matches!(error, IoError::Format(ref s) if s == "each page needs exactly one of window, outline, layout"));
    }
}

#[test]
fn pages_outline_far_from_origin_utm() {
    assert_frame_edges(Vec2::new(512345.678, 2109876.543));
}

#[test]
fn hostile_outline_is_an_error() {
    let d = Drawing::new_metric();
    for outline in [
        json!([[f64::NAN, 0], [420, 0], [420, 297], [0, 297]]),
        json!([[0, 0], [420, 0], [400, 297], [0, 297]]),
        json!([[0, 0], [420, 0], [420, 297]]),
        json!([[0, 0], [420, 0], [420, 297], [0, 297], [0, 0]]),
        json!([[0, 0], [0, 0], [420, 297], [0, 297]]),
        json!([[0, 0], [420, 297], [420, 0], [0, 297]]),
        json!([[0, 0, 1], [420, 0], [420, 297], [0, 297]]),
        json!([[1e308, 0], [-1e308, 0], [-1e308, 1], [1e308, 1]]),
        json!(null),
        json!("rectangle"),
    ] {
        assert!(plotter::plot(&d, &Space::Model, &json!({"pages": [{"outline": outline}]})).is_err(), "{outline}");
        assert!(plotter::plot(&d, &Space::Model, &json!({"outline": outline})).is_err(), "{outline}");
    }
    for frame in [
        PlotFrame { center: Vec2::ZERO, size: Vec2::new(-1.0, 1.0), rotation: 0.0 },
        PlotFrame { center: Vec2::ZERO, size: Vec2::new(1.0, 1.0), rotation: f64::NAN },
    ] {
        assert!(pdf(&d, &Space::Model, &PdfOptions { frame: Some(frame), ..Default::default() }).is_err());
    }
    for output in ["png", "svg"] {
        let frame = vertices(&outline(Vec2::ZERO));
        for opts in [
            json!({"output": output, "outline": frame}),
            json!({"output": output, "pages": [{"outline": frame}]}),
            json!({"output": output, "frame": {"center": [0, 0], "size": [420, 297], "rotation": 0.5}}),
        ] {
            let error = plotter::plot(&d, &Space::Model, &opts).unwrap_err();
            assert!(matches!(error, IoError::Format(ref s) if s == "rotated frames plot to PDF only"));
        }
    }
}
