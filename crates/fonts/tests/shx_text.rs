//! User SHX fonts: registered by content, found by file name, and drawn from their own glyphs.
//! Every font is built byte by byte here. In a process of its own: the registry is process-wide.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cadcraft_fonts::shx::ShxFont;
use cadcraft_fonts::{FontState, TextFont, char_advance, font_info, set_font_map, shape_line, shape_stroke, text_width, ttf};
use cadcraft_geom::Vec2;

const SHAPES_10: &[u8] = b"AutoCAD-86 shapes 1.0\r\n\x1a";
const UNIFONT: &[u8] = b"AutoCAD-86 unifont 1.0\r\n\x1a";
const BIGFONT: &[u8] = b"AutoCAD-86 bigfont 1.0\r\n\x1a";

fn serial() -> MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(PoisonError::into_inner)
}

fn glyph(name: &str, ops: &[u8]) -> Vec<u8> {
    let mut d = name.as_bytes().to_vec();
    d.push(0);
    d.extend_from_slice(ops);
    d.push(0);
    d
}

fn header(name: &str, above: u8) -> Vec<u8> {
    let mut d = name.as_bytes().to_vec();
    d.extend_from_slice(&[0, above, 2, 0, 0, 0]);
    d
}

fn shapes_image(name: &str, above: u8, glyphs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut entries = vec![(0u16, header(name, above))];
    entries.extend(glyphs.iter().cloned());
    let first = entries.iter().map(|e| e.0).min().unwrap_or(0);
    let last = entries.iter().map(|e| e.0).max().unwrap_or(0);
    let mut out = SHAPES_10.to_vec();
    out.extend_from_slice(&first.to_le_bytes());
    out.extend_from_slice(&last.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (code, data) in &entries {
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    }
    for (_, data) in &entries {
        out.extend_from_slice(data);
    }
    out.extend_from_slice(b"EOF");
    out
}

fn unifont_image(name: &str, above: u8, glyphs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let head = header(name, above);
    let mut out = UNIFONT.to_vec();
    out.extend_from_slice(&(glyphs.len() as u32 + 1).to_le_bytes());
    out.extend_from_slice(&(head.len() as u16).to_le_bytes());
    out.extend_from_slice(&head);
    for (code, data) in glyphs {
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// Height 10. "A": a vertical bar, advance 8. "B": a horizontal bar, advance 10. Space: advance 6.
/// Code 176 (what `%%d` asks for): a diagonal, advance 8.
fn test_font() -> Vec<u8> {
    shapes_image(
        "VHTEST",
        10,
        &[
            (0x41, glyph("A", &[0xA4, 0x02, 0x08, 8, 0xF6])),
            (0x42, glyph("B", &[0xA0])),
            (0x20, glyph("SPACE", &[0x02, 0x08, 6, 0])),
            (0xB0, glyph("DEG", &[0x82, 0x02, 0x08, 0, 0xF8])),
        ],
    )
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

fn same_points(got: &[Vec<Vec2>], want: &[&[(f64, f64)]]) -> bool {
    got.len() == want.len()
        && got.iter().zip(want).all(|(g, w)| g.len() == w.len() && g.iter().zip(w.iter()).all(|(p, q)| near(p.x, q.0) && near(p.y, q.1)))
}

#[test]
fn user_shx_font_is_found_and_draws() {
    let _s = serial();
    set_font_map([]);
    assert_eq!(font_info("vhtest.shx", "").state, FontState::Missing);
    let added = ttf::add_user_font("C:\\Fonts\\VHTEST.SHX", test_font()).unwrap();
    assert_eq!((added.stem.as_str(), added.bytes), ("vhtest.shx", test_font().len()));
    assert!(added.families.is_empty() && added.is_shx());
    assert!(ttf::user_fonts().contains(&added));
    // Not offered as a replacement: only TrueType faces can be drawn with.
    assert!(ttf::font_choices().iter().all(|c| c.origin != ttf::FontOrigin::Added));

    let info = font_info("vhtest.shx", "");
    assert_eq!((info.state, info.with, info.mapped, info.legacy), (FontState::Found, None, false, None));
    assert_eq!(font_info("VHTEST.SHX", ".VnTestH").state, FontState::Found);

    let f = TextFont::for_style("vhtest.shx", "");
    assert!(f.is_shx() && !f.is_outline());
    let sh = shape_line(&f, "AB", 2.0, 1.0, 0.0);
    assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (0.0, 2.0)], &[(1.6, 0.0), (3.6, 0.0)]]), "{:?}", sh.strokes);
    assert!(sh.glyphs.is_empty());
    assert!(near(sh.width, (0.8 + 1.0) * 2.0) && near(text_width(&f, "AB", 2.0, 1.0), sh.width));

    // Width factor scales x, oblique shears x by y, height scales both.
    let sh = shape_line(&f, "AB", 2.0, 0.5, std::f64::consts::FRAC_PI_4);
    assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (2.0, 2.0)], &[(0.8, 0.0), (1.8, 0.0)]]), "{:?}", sh.strokes);
    assert!(near(sh.width, 1.8));

    // The unit is the font's `above`, so a space moves by its own advance.
    assert!(near(text_width(&f, "A A", 10.0, 1.0), 8.0 + 6.0 + 8.0));

    assert!(ttf::remove_user_font("vhtest.shx"));
    assert_eq!(font_info("vhtest.shx", "").state, FontState::Missing);
    assert!(!TextFont::for_style("vhtest.shx", "").is_shx());
}

#[test]
fn shx_text_uses_codes_decorations_and_a_stroke_fallback() {
    let _s = serial();
    ttf::add_user_font("vhtest2.shx", test_font()).unwrap();
    let f = TextFont::for_style("vhtest2.shx", "");

    // %%d is code 176 in the font.
    let deg = shape_line(&f, "%%d", 1.0, 1.0, 0.0);
    assert!(same_points(&deg.strokes, &[&[(0.0, 0.0), (0.8, 0.8)]]), "{:?}", deg.strokes);
    assert!(near(deg.width, 0.8));

    // A letter the font lacks is drawn by the built-in stroke font, after the SHX glyph's advance.
    let ac = shape_line(&f, "AC", 10.0, 1.0, 0.0);
    let c = shape_stroke("C", 10.0, 1.0, 0.0);
    assert_eq!(ac.strokes.len(), 1 + c.strokes.len());
    assert!(ac.strokes[1..].iter().flatten().all(|p| p.x >= 8.0 - 1e-9));
    assert!(near(ac.width, 8.0 + char_advance('C') * 10.0));

    // Underline spans the characters.
    let u = shape_line(&f, "%%uAB%%u", 1.0, 1.0, 0.0);
    assert_eq!(u.strokes.len(), 3);
    assert!(same_points(&u.strokes[2..], &[&[(0.0, -0.2), (1.8, -0.2)]]), "{:?}", u.strokes);

    // Right-to-left lines keep the stroke font's shaping.
    let hebrew = shape_line(&f, "\u{05D0}\u{05D1}", 1.0, 1.0, 0.0);
    assert_eq!(hebrew, shape_stroke("\u{05D0}\u{05D1}", 1.0, 1.0, 0.0));
    ttf::remove_user_font("vhtest2.shx");
}

#[test]
fn unifont_glyphs_are_looked_up_by_code_point() {
    let _s = serial();
    let img = unifont_image("UNI", 10, &[(0x1EA0, glyph("DOT", &[0xA4, 0x02, 0x08, 5, 0xF6]))]);
    ttf::add_user_font("vhuni.shx", img).unwrap();
    for enc in [None, Some(encoding_rs::WINDOWS_1258), Some(encoding_rs::SHIFT_JIS)] {
        let f = TextFont::for_style("vhuni.shx", "").with_codepage(enc);
        let sh = shape_line(&f, "\u{1EA0}", 10.0, 1.0, 0.0);
        assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (0.0, 10.0)]]), "{:?}", sh.strokes);
        assert!(near(sh.width, 5.0) && near(text_width(&f, "\u{1EA0}", 10.0, 1.0), sh.width));
    }
    ttf::remove_user_font("vhuni.shx");
}

#[test]
fn missing_shx_still_strokes() {
    let _s = serial();
    set_font_map([]);
    // Point counts of the stroke font before SHX fonts existed (height 2.5, width factor 0.8,
    // oblique 0.2): (text, strokes, points, width).
    let baseline: [(&str, usize, usize, f64); 5] = [
        ("Hello World", 14, 63, 17.133333),
        ("AB", 4, 17, 3.2),
        ("%%c25 %%p3 %%d", 10, 51, 12.666667),
        ("%%uUnder%%u over", 14, 67, 16.933333),
        ("Mặt tiền ẠẢ", 22, 76, 17.266667),
    ];
    // A standard name stays a silent stroke font; any other missing SHX is reported missing but
    // drawn the same way.
    for (name, state) in
        [("romans.shx", FontState::Stroke), ("ROMANS.SHX", FontState::Stroke), ("isocp.shx", FontState::Stroke), ("vhtime.shx", FontState::Missing)]
    {
        assert_eq!(font_info(name, "").state, state, "{name}");
        let f = TextFont::for_style(name, "");
        assert!(!f.is_shx());
        for (text, strokes, points, width) in baseline {
            let sh = shape_line(&f, text, 2.5, 0.8, 0.2);
            assert_eq!((sh.strokes.len(), sh.strokes.iter().map(Vec::len).sum::<usize>()), (strokes, points), "{name} {text}");
            assert!((sh.width - width).abs() < 1e-5 && (text_width(&f, text, 2.5, 0.8) - width).abs() < 1e-5, "{name} {text}");
            assert_eq!(sh, shape_line(&TextFont::STROKE, text, 2.5, 0.8, 0.2));
        }
    }
    assert_eq!(font_info("", "").state, FontState::Stroke);
    assert_eq!(font_info("txt", "").state, FontState::Stroke);
}

#[test]
fn bad_shx_add_says_not_readable() {
    let _s = serial();
    let before = ttf::user_fonts().len();
    let ok = test_font();
    let mut truncated = ok.clone();
    truncated.truncate(ok.len() / 2);
    for (name, bytes) in [("junk.shx", vec![1, 2, 3, 4]), ("empty.shx", Vec::new()), ("cut.shx", truncated.clone()), ("renamed.ttf", truncated)] {
        assert_eq!(ttf::add_user_font(name, bytes), Err("Not a readable SHX file".to_string()), "{name}");
    }
    // A TrueType file is still checked as TrueType.
    let err = ttf::add_user_font("junk.ttf", vec![1, 2, 3, 4]).unwrap_err();
    assert!(err.contains("TrueType"), "{err}");
    let mut big = BIGFONT.to_vec();
    big.extend_from_slice(&[0; 32]);
    let err = ttf::add_user_font("big.shx", big).unwrap_err();
    assert!(err.contains("bigfont is not supported yet"), "{err}");
    assert_eq!(ttf::user_fonts().len(), before, "a refused font is not kept");
    assert_eq!(font_info("cut.shx", "").state, FontState::Missing);
}

#[test]
fn an_shx_font_is_recognised_by_content_and_replaces_one_of_the_same_name() {
    let _s = serial();
    // A font file whose name hides what it is: still SHX, and found as an SHX font.
    let added = ttf::add_user_font("vhodd.TTF", test_font()).unwrap();
    assert_eq!(added.stem, "vhodd.shx");
    assert!(TextFont::for_style("vhodd.shx", "").is_shx());
    // Added again, the new bytes replace the old; there is one entry.
    let other = shapes_image("VHTEST", 10, &[(0x41, glyph("A", &[0xA0]))]);
    ttf::add_user_font("VHODD.SHX", other).unwrap();
    assert_eq!(ttf::user_fonts().iter().filter(|u| u.stem == "vhodd.shx").count(), 1);
    let sh = shape_line(&TextFont::for_style("vhodd.shx", ""), "A", 10.0, 1.0, 0.0);
    assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (10.0, 0.0)]]), "{:?}", sh.strokes);
    assert!(ttf::remove_user_font("vhodd.shx"));
}

#[test]
fn glyphs_are_built_once_per_code() {
    let font = ShxFont::parse(&test_font()).unwrap();
    let a = font.glyph_shared(0x41).unwrap();
    assert!(Arc::ptr_eq(&a, &font.glyph_shared(0x41).unwrap()));
    assert_eq!(*a, font.glyph(0x41).unwrap());
    assert!(font.glyph_shared(0x7A).is_none());
    // A clone starts without the original's cache but equals it.
    assert_eq!(font.clone(), font);
}

/// Height 1000 so coordinates stay small; each glyph zigzags through `points` vertices.
fn big_glyph_font(codes: u16, points: usize) -> Vec<u8> {
    let zigzag: Vec<u8> = (0..points).map(|i| if i % 2 == 0 { 0x14 } else { 0x1C }).collect();
    let glyphs: Vec<(u16, Vec<u8>)> = (1..=codes).map(|c| (0x100 + c, glyph("Z", &zigzag))).collect();
    shapes_image("BIG", 10, &glyphs)
}

#[test]
fn glyph_cache_is_bounded_by_total_points() {
    let codes = 150u16;
    let font = ShxFont::parse(&big_glyph_font(codes, 19_000)).unwrap();
    let one = font.glyph_shared(0x101).unwrap();
    let per_glyph: usize = one.strokes.iter().map(Vec::len).sum();
    assert!(per_glyph >= 15_000, "the synthetic glyph is large ({per_glyph} points)");
    assert!(per_glyph * usize::from(codes) > cadcraft_fonts::shx::MAX_CACHED_POINTS, "the font as a whole is over the cap");
    for c in 1..=codes {
        let g = font.glyph_shared(0x100 + u32::from(c)).unwrap();
        assert_eq!(g.strokes.iter().map(Vec::len).sum::<usize>(), per_glyph);
        assert!(font.cached_points() <= cadcraft_fonts::shx::MAX_CACHED_POINTS, "after code {c}: {}", font.cached_points());
    }
    // Dropping the cache only costs a rebuild: the glyph is still right and cached again.
    let again = font.glyph_shared(0x101).unwrap();
    assert_eq!(*again, *one);
    assert!(font.cached_points() >= per_glyph);
}

#[test]
fn oversized_shx_says_how_large() {
    let _s = serial();
    let mut big = SHAPES_10.to_vec();
    big.resize(cadcraft_fonts::shx::MAX_SHX_BYTES + 1, 0);
    let err = ttf::add_user_font("huge.shx", big).unwrap_err();
    assert_eq!(err, "huge.shx is larger than 8 MB");
    assert_eq!(font_info("huge.shx", "").state, FontState::Missing);
}

#[test]
fn debug_output_of_a_font_is_short() {
    let font = ShxFont::parse(&big_glyph_font(50, 2_000)).unwrap();
    let text = format!("{font:?}");
    assert!(text.len() < 200, "{} bytes: {text}", text.len());
    assert!(text.contains("BIG") && text.contains("50"), "{text}");
}

#[test]
fn cp1258_roundtrip_vhtime_like() {
    let _s = serial();
    // Ò has no single byte in Windows-1258; Ă distinguishes it from Windows-1252.
    let (bytes, _, unmappable) = encoding_rs::WINDOWS_1258.encode("Ă");
    assert!(!unmappable);
    assert_eq!(bytes.as_ref(), &[0xC3]);
    let code = u16::from(*bytes.first().unwrap());
    let image = shapes_image("F5ROUNDTRIP", 10, &[(code, glyph("ABREVE", &[0xA4, 0x02, 0x08, 8, 0xF6])), (0x3F, glyph("QUESTION", &[0x50]))]);
    ttf::add_user_font("f5-roundtrip.shx", image).unwrap();
    for enc in [None, Some(encoding_rs::WINDOWS_1258)] {
        let f = TextFont::for_style("f5-roundtrip.shx", "").with_codepage(enc);
        let sh = shape_line(&f, "Ă", 10.0, 1.0, 0.0);
        let (end, advance) = if enc.is_some() { ((0.0, 10.0), 8.0) } else { ((5.0, 0.0), 5.0) };
        assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), end]]), "{:?}", sh.strokes);
        assert!(near(sh.width, advance) && near(text_width(&f, "Ă", 10.0, 1.0), sh.width));
    }
    assert!(ttf::remove_user_font("f5-roundtrip.shx"));
}

#[test]
fn shapes_diameter_uses_legacy_slot_with_default_and_cp1258() {
    let _s = serial();
    let image = shapes_image("F5DIAMETER", 10, &[(178, glyph("DIAMETER", &[0x82])), (0x3F, glyph("QUESTION", &[0x50]))]);
    ttf::add_user_font("f5-diameter.shx", image).unwrap();
    for enc in [None, Some(encoding_rs::WINDOWS_1258)] {
        let f = TextFont::for_style("f5-diameter.shx", "").with_codepage(enc);
        for text in ["%%c", "\u{2300}"] {
            let sh = shape_line(&f, text, 10.0, 1.0, 0.0);
            assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (8.0, 8.0)]]), "{text}: {:?}", sh.strokes);
            assert!(near(sh.width, 8.0) && near(text_width(&f, text, 10.0, 1.0), sh.width));
        }
    }
    assert!(ttf::remove_user_font("f5-diameter.shx"));
}

#[test]
fn degree_and_plus_minus_keep_legacy_slots_when_unmappable() {
    let _s = serial();
    let image =
        shapes_image("FCSYMBOLS", 10, &[(176, glyph("DEGREE", &[0xA4])), (177, glyph("PLUSMINUS", &[0x82])), (0x3F, glyph("QUESTION", &[0x50]))]);
    let name = "fc-symbols.shx";
    ttf::add_user_font(name, image).unwrap();
    for enc in [encoding_rs::WINDOWS_874, encoding_rs::WINDOWS_1252] {
        let f = TextFont::for_style(name, "").with_codepage(Some(enc));
        for (text, symbol, slot, end, advance) in [("%%d", "°", 0xB0, (0.0, 10.0), 0.0), ("%%p", "±", 0xB1, (8.0, 8.0), 8.0)] {
            let (bytes, _, unmappable) = enc.encode(symbol);
            if enc == encoding_rs::WINDOWS_874 {
                assert!(unmappable, "{symbol}");
            } else {
                assert!(!unmappable, "{symbol}");
                assert_eq!(bytes.as_ref(), &[slot]);
            }
            let sh = shape_line(&f, text, 10.0, 1.0, 0.0);
            assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), end]]), "{text}: {:?}", sh.strokes);
            assert!(near(sh.width, advance) && near(text_width(&f, text, 10.0, 1.0), sh.width));
        }
    }
    assert!(ttf::remove_user_font(name));
}

#[test]
fn unencodable_char_falls_back() {
    let _s = serial();
    let question = glyph("QUESTION", &[0x50]);
    let ampersand = glyph("AMP", &[0xA4]);
    let with_question = shapes_image("F5FALLBACK", 10, &[(0x3F, question), (0x26, ampersand.clone())]);
    ttf::add_user_font("f5-fallback.shx", with_question).unwrap();
    let f = TextFont::for_style("f5-fallback.shx", "").with_codepage(Some(encoding_rs::WINDOWS_1258));
    for c in ['😀', 'Ò', 'Ắ'] {
        let mut buf = [0; 4];
        let text = c.encode_utf8(&mut buf);
        assert!(encoding_rs::WINDOWS_1258.encode(text).2, "{c}");
        let sh = shape_line(&f, text, 10.0, 1.0, 0.0);
        assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (5.0, 0.0)]]), "{c}: {:?}", sh.strokes);
        assert!(near(sh.width, 5.0) && near(text_width(&f, text, 10.0, 1.0), sh.width));
    }
    assert!(ttf::remove_user_font("f5-fallback.shx"));

    ttf::add_user_font("f5-no-question.shx", shapes_image("F5STROKE", 10, &[(0x26, ampersand)])).unwrap();
    let f = TextFont::for_style("f5-no-question.shx", "").with_codepage(Some(encoding_rs::WINDOWS_1258));
    for c in ['😀', 'Ò', 'Ắ'] {
        let mut buf = [0; 4];
        let text = c.encode_utf8(&mut buf);
        let sh = shape_line(&f, text, 10.0, 1.0, 0.0);
        let stroke = shape_stroke(text, 10.0, 1.0, 0.0);
        assert_eq!(sh.strokes, stroke.strokes, "{c}");
        // SHX text keeps the stroke font's trailing gap; a TTF fallback glyph (CRAFT_FONTS_DIR set)
        // has none, so it matches the stroke line width instead of char_advance.
        let advance = near(sh.width, char_advance(c) * 10.0) || near(sh.width, stroke.width);
        assert!(advance && near(text_width(&f, text, 10.0, 1.0), sh.width), "{c}: {} vs {}", sh.width, stroke.width);
    }
    assert!(ttf::remove_user_font("f5-no-question.shx"));
}

#[test]
fn multibyte_shx_codes_fall_back() {
    let _s = serial();
    let image = shapes_image("F5DBCS", 10, &[(0x3F, glyph("QUESTION", &[0x50])), (0x90, glyph("LEAD", &[0xA4]))]);
    ttf::add_user_font("f5-dbcs.shx", image).unwrap();
    let f = TextFont::for_style("f5-dbcs.shx", "").with_codepage(Some(encoding_rs::SHIFT_JIS));
    let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode("図");
    assert!(!unmappable && bytes.len() == 2);
    let sh = shape_line(&f, "図", 10.0, 1.0, 0.0);
    assert!(same_points(&sh.strokes, &[&[(0.0, 0.0), (5.0, 0.0)]]));
    assert!(near(sh.width, 5.0) && near(text_width(&f, "図", 10.0, 1.0), sh.width));
    assert!(ttf::remove_user_font("f5-dbcs.shx"));
}

#[test]
fn mtext_font_switch_retains_codepage() {
    let _s = serial();
    let image = shapes_image("F5MTEXT", 10, &[(0xC3, glyph("ABREVE", &[0xA4])), (0x3F, glyph("QUESTION", &[0x50]))]);
    ttf::add_user_font("f5-mtext.shx", image).unwrap();
    let base = cadcraft_fonts::MTextParams {
        font: TextFont::STROKE.with_codepage(Some(encoding_rs::WINDOWS_1258)),
        ..cadcraft_fonts::MTextParams::new(Vec2::ZERO, 10.0)
    };
    let switched = cadcraft_fonts::layout_mtext_with("{\\ff5-mtext.shx;Ă}", &base);
    let direct = cadcraft_fonts::layout_mtext_with(
        "Ă",
        &cadcraft_fonts::MTextParams { font: TextFont::for_style("f5-mtext.shx", "").with_codepage(Some(encoding_rs::WINDOWS_1258)), ..base },
    );
    assert!(!direct.strokes.is_empty());
    assert_eq!(switched.strokes, direct.strokes);
    assert_eq!(switched.bounds, direct.bounds);
    assert!(ttf::remove_user_font("f5-mtext.shx"));
}
