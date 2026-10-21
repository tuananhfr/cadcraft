//! Text in a user's SHX font: each character is looked up by its code, drawn from the font's own
//! strokes and spaced by the glyph's advance (no letter gap is added).
//!
//! Characters the font lacks, and right-to-left lines, are left to the built-in stroke font.
use cadcraft_geom::Vec2;
use encoding_rs::Encoding;

use crate::shx::{ShxFont, ShxKind};
use crate::{Shaped, bidi, decode_controls, decorations, line_width, shape_stroke, stroke_advance};

fn code(font: &ShxFont, enc: &'static Encoding, c: char) -> Option<u32> {
    if font.kind == ShxKind::Unifont {
        return u16::try_from(u32::from(c)).ok().map(u32::from);
    }
    let mut buf = [0; 4];
    let (bytes, _, had_unmappable) = enc.encode(c.encode_utf8(&mut buf));
    // Preserve legacy SHX symbol slots only when the codepage cannot encode decoded controls.
    const LEGACY_SLOTS: &[(char, u32)] = &[('\u{00B0}', 176), ('\u{00B1}', 177), ('\u{2300}', 178)];
    if had_unmappable && let Some((_, slot)) = LEGACY_SLOTS.iter().find(|(symbol, _)| *symbol == c) {
        return Some(*slot);
    }
    // Unmappable characters produce HTML references; neither those nor DBCS bytes are glyph codes.
    if had_unmappable || bytes.len() != 1 {
        return Some(u32::from(b'?'));
    }
    bytes.first().copied().map(u32::from)
}

fn sane(height: f64, width_factor: f64) -> (f64, f64) {
    let h = if height.is_finite() && height > 0.0 { height } else { 1.0 };
    let wf = if width_factor.is_finite() && width_factor.abs() > 1e-6 { width_factor } else { 1.0 };
    (h, wf)
}

pub(crate) fn shape(font: &ShxFont, enc: &'static Encoding, s: &str, height: f64, width_factor: f64, oblique: f64) -> Shaped {
    if bidi::has_rtl(s) {
        return shape_stroke(s, height, width_factor, oblique);
    }
    let (h, wf) = sane(height, width_factor);
    let shear = oblique.tan().clamp(-10.0, 10.0);
    let mut out = Shaped::default();
    let mut spans = Vec::new();
    let mut x = 0.0;
    for (c, under, over) in decode_controls(s) {
        if bidi::is_format_control(c) {
            continue;
        }
        let start = x;
        match code(font, enc, c).and_then(|k| font.glyph_shared(k)) {
            Some(g) => {
                for stroke in &g.strokes {
                    let pts = stroke.iter().map(|p| {
                        let (gx, gy) = (f64::from(p[0]), f64::from(p[1]));
                        Vec2::new(x + gx * h * wf + gy * h * shear, gy * h)
                    });
                    out.strokes.push(pts.collect());
                }
                x += f64::from(g.advance[0]) * h * wf;
            }
            None => {
                let mut piece = shape_stroke(c.encode_utf8(&mut [0; 4]), h, wf, oblique);
                piece.map(|p| p + Vec2::new(x, 0.0));
                out.extend(piece);
                x += stroke_advance(c, h, wf);
            }
        }
        spans.push((start, x, under, over));
    }
    out.strokes.extend(decorations(&spans, h));
    out.width = x.max(0.0);
    out
}

pub(crate) fn width(font: &ShxFont, enc: &'static Encoding, s: &str, height: f64, width_factor: f64) -> f64 {
    if bidi::has_rtl(s) {
        return line_width(s, height, width_factor);
    }
    let (h, wf) = sane(height, width_factor);
    let total: f64 = decode_controls(s)
        .into_iter()
        .filter(|(c, _, _)| !bidi::is_format_control(*c))
        .map(|(c, _, _)| match code(font, enc, c).and_then(|k| font.glyph_shared(k)) {
            Some(g) => f64::from(g.advance[0]) * h * wf,
            None => stroke_advance(c, h, wf),
        })
        .sum();
    total.max(0.0)
}
