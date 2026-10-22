use cadcraft_doc::{Common, EntityKind, Line, Lineweight, Space};
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::base64_decode;
use cadcraft_geom::Vec2;
use serde_json::json;

#[test]
fn preview_outline_renders_the_rotated_frame() {
    let mut s = Session::new();
    let center = Vec2::new(512345.678, 2109876.543);
    let rotation = std::f64::consts::PI / 6.0;
    let world = |p: Vec2| center + p.rotate(rotation);
    let outline = [Vec2::new(-210.0, -148.5), Vec2::new(210.0, -148.5), Vec2::new(210.0, 148.5), Vec2::new(-210.0, 148.5)].map(world);
    // A two-pixel border sits three pixels inside the crop so corner samples are unambiguous.
    let inset = 3.5 * 420.0 / 400.0;
    let border = [
        Vec2::new(-210.0 + inset, -148.5 + inset),
        Vec2::new(210.0 - inset, -148.5 + inset),
        Vec2::new(210.0 - inset, 148.5 - inset),
        Vec2::new(-210.0 + inset, 148.5 - inset),
    ]
    .map(world);
    let d = s.doc_mut().unwrap();
    d.header.set("MEASUREMENT", cadcraft_doc::HVal::Int(1));
    for (a, b) in border.iter().zip(border.iter().cycle().skip(1)) {
        let common = Common { lineweight: Lineweight::Mm100(210), ..Default::default() };
        d.add(&Space::Model, common, EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) })).unwrap();
    }
    d.add(
        &Space::Model,
        Common::default(),
        EntityKind::Line(Line { a: world(Vec2::new(5000.0, 0.0)).to3(0.0), b: world(Vec2::new(6000.0, 0.0)).to3(0.0) }),
    )
    .unwrap();
    let outline = json!(outline.map(|p| [p.x, p.y]));
    let result = s.execute("preview", &json!({"outline": outline, "width": 400})).unwrap();
    assert_eq!(result["width"], 400);
    let png = base64_decode(result["data"].as_str().unwrap()).unwrap();
    let image = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert_eq!((image.width(), image.height()), (400, 283));
    for (x, y) in [(3, 3), (396, 3), (396, 279), (3, 279)] {
        let pixel = image.pixel(x, y).unwrap();
        assert!(pixel.red() < 250 && pixel.green() < 250 && pixel.blue() < 250, "white corner ({x}, {y})");
    }
    let middle = image.pixel(200, 141).unwrap();
    assert_eq!((middle.red(), middle.green(), middle.blue()), (255, 255, 255));
    for outline in [json!([[0, 0], [420, 0], [400, 297], [0, 297]]), json!([[0, 0], [1, 0], [1, 1]]), json!([[null, 0], [1, 0], [1, 1], [0, 1]])] {
        assert!(s.execute("preview", &json!({"outline": outline, "width": 400})).is_err());
    }
}
