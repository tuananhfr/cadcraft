use std::sync::Arc;

use cadcraft_doc::{Common, Drawing, EntityKind, Layer, Line, Space, Text, TextStyle};
use cadcraft_fonts::subset::{Embedding, embedding};
use cadcraft_fonts::ttf::Face;
use cadcraft_geom::Vec3;
use cadcraft_io::PdfOptions;
use cadcraft_io::pdf::pdf_items;
use sha2::{Digest, Sha256};

const EXPECTED_SHA256: &str = "c7cd7dac753e586858eaa784c4ed05a2060bdf2fe1475b0f1c0dee7529eaa502";

fn fixture() -> (Drawing, Vec<(Space, PdfOptions)>, PdfOptions, bool) {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer::new("Frame"));
    d.layers.push(Layer::new("Notes"));
    let font = cadcraft_fonts::CRAFT_FONTS.iter().find(|font| {
        Face::new(Arc::new(font.bytes.to_vec()), 0)
            .is_some_and(|face| embedding(&face) == Embedding::Allowed && "CADAB12XY".chars().all(|ch| face.has_glyph(ch)))
    });
    let font_name = if let Some(font) = font {
        cadcraft_fonts::ttf::register("cadcraft-pdf-identity.ttf", font.bytes.to_vec());
        "cadcraft-pdf-identity.ttf"
    } else {
        "CADCraft Stroke"
    };
    d.text_styles.push(TextStyle { name: "Identity".into(), font: font_name.into(), ..TextStyle::default() });
    let spaces = [Space::Model, Space::Paper("Layout1".into()), Space::Paper("Layout2".into())];
    let mut items = Vec::new();
    let mut links = Vec::new();
    for (index, (space, value)) in spaces.into_iter().zip(["CAD AB", "CAD 12", "CAD XY"]).enumerate() {
        let line = EntityKind::Line(Line { a: Vec3::new(10.0, 10.0, 0.0), b: Vec3::new(100.0, 70.0, 0.0) });
        let handle = d.add(&space, Common { layer: "Frame".into(), ..Common::default() }, line).unwrap();
        if index == 0 {
            links.push((handle, "https://example.com/cad-identity".into()));
        }
        let text = Text {
            insert: Vec3::new(20.0, 25.0, 0.0),
            align_pt: None,
            height: 5.0,
            value: value.into(),
            rotation: 0.2,
            width_factor: 0.9,
            oblique: 0.1,
            style: "Identity".into(),
            halign: Default::default(),
            valign: Default::default(),
        };
        d.add(&space, Common { layer: "Notes".into(), ..Common::default() }, EntityKind::Text(text)).unwrap();
        let options = PdfOptions {
            paper: Some("A4".into()),
            landscape: Some(false),
            scale: Some(1.0),
            fit: Some(false),
            margins: Some([0.0; 4]),
            page_margins: true,
            label: Some(format!("Identity {}", index + 1)),
            ..PdfOptions::default()
        };
        items.push((space, options));
    }
    let options = PdfOptions { compress: false, title: "Writer identity".into(), links, ..PdfOptions::default() };
    (d, items, options, font.is_some())
}

#[test]
fn streaming_writer_is_byte_identical_to_previous() {
    let (drawing, items, options, ttf) = fixture();
    let bytes = pdf_items(&drawing, &items, &options).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Type /Page ").count(), 3);
    assert!(text.contains("/Type /OCG") && text.contains("/Subtype /Link"));
    assert!(!text.contains("/FlateDecode"));
    assert_eq!(text.contains("/FontFile2"), ttf);
    let path = if ttf { "TTF" } else { "stroke (CRAFT_FONTS_DIR supplied no usable embedded font)" };
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    eprintln!("PDF writer identity: text path={path}, sha256={sha256}");
    // The reference digest was taken with CRAFT_FONTS_DIR set; the stroke path has no recorded digest.
    if !ttf {
        return;
    }
    assert_eq!(sha256, EXPECTED_SHA256, "text path={path}; computed sha256={sha256}; fill EXPECTED_SHA256 from the pre-refactor writer");
}
