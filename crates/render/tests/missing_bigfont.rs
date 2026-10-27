//! Text set with the `vns.shx` bigfont, which CADCraft can't read: the pair `/g` is one bigfont
//! character, the diameter sign AutoCAD draws for rebar labels (`/g12a150` is `⌀12a150`), and
//! pairs like `/S` put Vietnamese marks on the vowel before them (`THE/SP` is `THÉP`).

use cadcraft_doc::{Drawing, Text, TextStyle};
use cadcraft_geom::Vec3;
use cadcraft_render::place_text_entity;

fn style(name: &str, big_font: &str) -> TextStyle {
    TextStyle { name: name.into(), font: "romans.shx".into(), big_font: big_font.into(), width_factor: 0.6, ..Default::default() }
}

fn drawing() -> Drawing {
    let mut d = Drawing::new_metric();
    d.text_styles.push(style("VNS", "vns.shx"));
    d.text_styles.push(style("UPPER", "C:\\Fonts\\VNS.SHX"));
    d.text_styles.push(style("PLAIN", ""));
    d.text_styles.push(style("OTHER", "chineset.shx"));
    d
}

/// The drawn strokes and glyph outlines, and the width.
fn shaped(d: &Drawing, style: &str, value: &str) -> (cadcraft_fonts::Shaped, f64) {
    let t = Text {
        insert: Vec3::ZERO,
        align_pt: None,
        height: 250.0,
        value: value.into(),
        rotation: 0.0,
        width_factor: 0.6,
        oblique: 0.0,
        style: style.into(),
        halign: Default::default(),
        valign: Default::default(),
    };
    let (s, b) = place_text_entity(d, &t, value);
    (s, b.width())
}

#[test]
fn vns_slash_g_draws_the_diameter_sign() {
    let d = drawing();
    for st in ["VNS", "UPPER"] {
        assert_eq!(shaped(&d, st, "/g12a150"), shaped(&d, "PLAIN", "%%c12a150"), "style {st}");
        assert_eq!(shaped(&d, st, "3/g16"), shaped(&d, "PLAIN", "3%%c16"), "style {st}");
    }
}

#[test]
fn other_text_and_other_fonts_stay_as_typed() {
    let d = drawing();
    // Without the bigfont, or with another one, the slash and the g are two characters.
    for st in ["PLAIN", "OTHER"] {
        assert_ne!(shaped(&d, st, "/g12a150"), shaped(&d, "PLAIN", "%%c12a150"), "style {st}");
    }
    // vns.shx pairs not proven against AutoCAD's output are drawn as typed.
    for text in ["/Q12A150", "X/S", "1/2", "a/", "/"] {
        assert_eq!(shaped(&d, "VNS", text), shaped(&d, "PLAIN", text), "{text}");
    }
}

#[test]
fn vns_marks_draw_vietnamese_letters() {
    let d = drawing();
    for (typed, drawn) in [("THE/SP", "THÉP"), ("TO/M/RNG", "TỔNG"), ("L/}/{/JNG", "LƯỢNG"), ("/DA/S", "ĐÁ"), ("/G12A150", "%%c12A150")] {
        assert_eq!(shaped(&d, "VNS", typed), shaped(&d, "PLAIN", drawn), "{typed}");
    }
}
