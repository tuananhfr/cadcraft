//! Text typed for the legacy 8-bit Vietnamese fonts (TCVN3 `.Vn*`, VNI `VNI-*`), shown in
//! Unicode when that font isn't installed. Display only: the stored text never changes.
//!
//! Drawings keep those bytes as Latin-1 characters (`MÆT` is `MẶT` in `.VnBlackH`); only the
//! font's name tells which encoding they are in.

use std::borrow::Cow;

use crate::stroke::TONES;

/// A legacy Vietnamese encoding, as a font name implies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyVi {
    /// TCVN3; `upper`: an H font (`.VnArialH`), which draws every letter as a capital.
    Tcvn3 {
        upper: bool,
    },
    Vni,
    /// Named like a Vietnamese font (`VNSIMLI.shx`, `VHBLACK.TTF`) without saying which
    /// encoding: decided per string.
    Guess {
        upper: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Plain,
    Circumflex,
    Breve,
}

/// TCVN3 (TCVN 5712:1993 VN3) bytes that are not ASCII.
const TCVN3: &[(u8, char)] = &[
    (0xA1, 'Ă'),
    (0xA2, 'Â'),
    (0xA3, 'Ê'),
    (0xA4, 'Ô'),
    (0xA5, 'Ơ'),
    (0xA6, 'Ư'),
    (0xA7, 'Đ'),
    (0xA8, 'ă'),
    (0xA9, 'â'),
    (0xAA, 'ê'),
    (0xAB, 'ô'),
    (0xAC, 'ơ'),
    (0xAD, 'ư'),
    (0xAE, 'đ'),
    (0xB5, 'à'),
    (0xB6, 'ả'),
    (0xB7, 'ã'),
    (0xB8, 'á'),
    (0xB9, 'ạ'),
    (0xBB, 'ằ'),
    (0xBC, 'ẳ'),
    (0xBD, 'ẵ'),
    (0xBE, 'ắ'),
    (0xC6, 'ặ'),
    (0xC7, 'ầ'),
    (0xC8, 'ẩ'),
    (0xC9, 'ẫ'),
    (0xCA, 'ấ'),
    (0xCB, 'ậ'),
    (0xCC, 'è'),
    (0xCE, 'ẻ'),
    (0xCF, 'ẽ'),
    (0xD0, 'é'),
    (0xD1, 'ẹ'),
    (0xD2, 'ề'),
    (0xD3, 'ể'),
    (0xD4, 'ễ'),
    (0xD5, 'ế'),
    (0xD6, 'ệ'),
    (0xD7, 'ì'),
    (0xD8, 'ỉ'),
    (0xDC, 'ĩ'),
    (0xDD, 'í'),
    (0xDE, 'ị'),
    (0xDF, 'ò'),
    (0xE1, 'ỏ'),
    (0xE2, 'õ'),
    (0xE3, 'ó'),
    (0xE4, 'ọ'),
    (0xE5, 'ồ'),
    (0xE6, 'ổ'),
    (0xE7, 'ỗ'),
    (0xE8, 'ố'),
    (0xE9, 'ộ'),
    (0xEA, 'ờ'),
    (0xEB, 'ở'),
    (0xEC, 'ỡ'),
    (0xED, 'ớ'),
    (0xEE, 'ợ'),
    (0xEF, 'ù'),
    (0xF1, 'ủ'),
    (0xF2, 'ũ'),
    (0xF3, 'ú'),
    (0xF4, 'ụ'),
    (0xF5, 'ừ'),
    (0xF6, 'ử'),
    (0xF7, 'ữ'),
    (0xF8, 'ứ'),
    (0xF9, 'ự'),
    (0xFA, 'ỳ'),
    (0xFB, 'ỷ'),
    (0xFC, 'ỹ'),
    (0xFD, 'ý'),
    (0xFE, 'ỵ'),
];

/// VNI mark bytes, typed after the vowel they go on: (byte, tone index into [`TONES`], shape).
/// These are the lower-case marks; a capital takes the byte − 0x20.
const VNI_MARKS: &[(u8, Option<usize>, Shape)] = &[
    (0xF8, Some(0), Shape::Plain),
    (0xF9, Some(1), Shape::Plain),
    (0xFB, Some(2), Shape::Plain),
    (0xF5, Some(3), Shape::Plain),
    (0xEF, Some(4), Shape::Plain),
    (0xE2, None, Shape::Circumflex),
    (0xE0, Some(0), Shape::Circumflex),
    (0xE1, Some(1), Shape::Circumflex),
    (0xE5, Some(2), Shape::Circumflex),
    (0xE3, Some(3), Shape::Circumflex),
    (0xE4, Some(4), Shape::Circumflex),
    (0xEA, None, Shape::Breve),
    (0xE8, Some(0), Shape::Breve),
    (0xE9, Some(1), Shape::Breve),
    (0xFA, Some(2), Shape::Breve),
    (0xFC, Some(3), Shape::Breve),
    (0xEB, Some(4), Shape::Breve),
];

/// SHX fonts, by lower-case file stem, that draw TCVN3 bytes as capitals though their names
/// don't say so.
const TCVN3_UPPER_SHX: &[&str] = &["chhbg"];

/// VNI letters that are one byte (lower case; a capital is the byte − 0x20).
const VNI_SINGLE: &[(u8, char)] = &[(0xF1, 'đ'), (0xF4, 'ơ'), (0xF6, 'ư'), (0xEC, 'ì'), (0xED, 'í'), (0xE6, 'ỉ'), (0xF3, 'ĩ'), (0xF2, 'ị')];

impl LegacyVi {
    /// The encoding a font family (`.VnTimeH`, `VNI-Times`) or file name (`vnsimli.shx`)
    /// implies, if any.
    pub fn from_name(name: &str) -> Option<LegacyVi> {
        let n = name.trim();
        let lower = n.to_ascii_lowercase();
        if lower.starts_with(".vn") {
            return Some(LegacyVi::Tcvn3 { upper: n.ends_with('H') });
        }
        if lower.starts_with("vni-") {
            return Some(LegacyVi::Vni);
        }
        let file = lower.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
        // "VNI Cambodia", "VNI Russia"…: VNI's fonts for other scripts.
        if stem.starts_with("vni ") {
            return None;
        }
        // AutoCAD plots `m¸c` in `chhbg.shx` as `MÁC` and `b25` as `B25`.
        if TCVN3_UPPER_SHX.contains(&stem) && (file == stem || file.ends_with(".shx")) {
            return Some(LegacyVi::Tcvn3 { upper: true });
        }
        (stem.starts_with("vn") || stem.starts_with("vh")).then_some(LegacyVi::Guess { upper: stem.starts_with("vh") })
    }

    /// `s` as the font would show it, in Unicode.
    pub fn decode(self, s: &str) -> Cow<'_, str> {
        match self {
            LegacyVi::Tcvn3 { upper } => tcvn3(s, upper),
            LegacyVi::Vni => vni(s),
            LegacyVi::Guess { upper } => match guess(s) {
                Some(LegacyVi::Vni) => vni(s),
                Some(_) => tcvn3(s, upper),
                None => Cow::Borrowed(s),
            },
        }
    }
}

fn byte(c: char) -> Option<u8> {
    u8::try_from(u32::from(c)).ok()
}

fn tcvn3_char(c: char) -> Option<char> {
    let b = byte(c)?;
    TCVN3.iter().find(|(k, _)| *k == b).map(|(_, v)| *v)
}

fn tcvn3(s: &str, upper: bool) -> Cow<'_, str> {
    if !s.chars().any(|c| tcvn3_char(c).is_some() || (upper && c.is_lowercase())) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + s.len() / 2);
    for c in s.chars() {
        let d = tcvn3_char(c).unwrap_or(c);
        if upper {
            out.extend(d.to_uppercase());
        } else {
            out.push(d);
        }
    }
    Cow::Owned(out)
}

/// A VNI mark: its tone, shape and whether it is the capital form.
fn vni_mark(c: char) -> Option<(Option<usize>, Shape, bool)> {
    let b = byte(c)?;
    VNI_MARKS.iter().find_map(|&(k, tone, shape)| if k == b { Some((tone, shape, false)) } else { (k - 0x20 == b).then_some((tone, shape, true)) })
}

fn vni_single(c: char) -> Option<char> {
    let b = byte(c)?;
    VNI_SINGLE.iter().find_map(|&(k, v)| {
        if k == b {
            Some(v)
        } else if k - 0x20 == b {
            v.to_uppercase().next()
        } else {
            None
        }
    })
}

/// `base` with a VNI mark on it, if the mark can go on that letter.
fn compose(base: char, tone: Option<usize>, shape: Shape) -> Option<char> {
    let shaped = match (shape, base) {
        (Shape::Plain, _) => base,
        (Shape::Circumflex, 'a') => 'â',
        (Shape::Circumflex, 'e') => 'ê',
        (Shape::Circumflex, 'o') => 'ô',
        (Shape::Circumflex, 'A') => 'Â',
        (Shape::Circumflex, 'E') => 'Ê',
        (Shape::Circumflex, 'O') => 'Ô',
        (Shape::Breve, 'a') => 'ă',
        (Shape::Breve, 'A') => 'Ă',
        _ => return None,
    };
    match tone {
        None => Some(shaped),
        Some(i) => TONES.iter().find(|(b, _)| *b == shaped).and_then(|(_, row)| row.chars().nth(i)),
    }
}

fn vni(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        return Cow::Borrowed(s);
    }
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        // The font draws a mark over whatever letter it follows, whichever case the mark is in.
        if let Some((tone, shape, _)) = vni_mark(c)
            && let Some(prev) = out.last_mut()
            && let Some(m) = compose(*prev, tone, shape)
        {
            *prev = m;
            continue;
        }
        out.push(vni_single(c).unwrap_or(c));
    }
    Cow::Owned(out.into_iter().collect())
}

/// Which encoding a string from a font that doesn't say is in. TCVN3 text nearly always has a
/// byte VNI never uses (â ê ô ơ ư đ, the toned a's: 0xA1–0xBE); VNI text has a vowel followed by
/// a mark of its own case that fits it. Text with characters past Latin-1 is already Unicode.
fn guess(s: &str) -> Option<LegacyVi> {
    if s.chars().any(|c| byte(c).is_none()) {
        return None;
    }
    if s.chars().any(|c| byte(c).is_some_and(|b| (0xA1..=0xBE).contains(&b)) && tcvn3_char(c).is_some()) {
        return Some(LegacyVi::Tcvn3 { upper: false });
    }
    let vni_pair = s.chars().zip(s.chars().skip(1)).any(|(a, b)| {
        let base = vni_single(a).unwrap_or(a);
        vni_mark(b).is_some_and(|(tone, shape, upper)| base.is_uppercase() == upper && compose(base, tone, shape).is_some())
    });
    if vni_pair {
        return Some(LegacyVi::Vni);
    }
    s.chars().any(|c| tcvn3_char(c).is_some()).then_some(LegacyVi::Tcvn3 { upper: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text as a drawing stores it: each byte as the Latin-1 character.
    fn bytes(b: &[u8]) -> String {
        b.iter().map(|&b| char::from(b)).collect()
    }

    #[test]
    fn names_say_the_encoding() {
        assert_eq!(LegacyVi::from_name(".VnBlackH"), Some(LegacyVi::Tcvn3 { upper: true }));
        assert_eq!(LegacyVi::from_name(".VnArial"), Some(LegacyVi::Tcvn3 { upper: false }));
        assert_eq!(LegacyVi::from_name("VNI-Helve-Condense"), Some(LegacyVi::Vni));
        assert_eq!(LegacyVi::from_name("VNSIMLI.shx"), Some(LegacyVi::Guess { upper: false }));
        assert_eq!(LegacyVi::from_name("C:\\Fonts\\VHBLACK.TTF"), Some(LegacyVi::Guess { upper: true }));
        // Not Vietnamese: VNI's other scripts, and plain fonts.
        assert_eq!(LegacyVi::from_name("VNI Cambodia"), None);
        assert_eq!(LegacyVi::from_name("arial.ttf"), None);
        assert_eq!(LegacyVi::from_name(""), None);
    }

    #[test]
    fn chhbg_is_tcvn3_upper() {
        let upper = Some(LegacyVi::Tcvn3 { upper: true });
        for name in ["chhbg.shx", "CHHBG.SHX", "C:\\Fonts\\chhbg.shx", "chhbg"] {
            assert_eq!(LegacyVi::from_name(name), upper, "{name}");
        }
        assert_eq!(LegacyVi::from_name("chhbgx.shx"), None);
        let Some(l) = LegacyVi::from_name("chhbg.shx") else { panic!("chhbg.shx is known") };
        assert_eq!(l.decode(&bytes(b"m\xb8c b25")), "MÁC B25");
        assert_eq!(l.decode(&bytes(b"\xae\xe9 s\xf4t")), "ĐỘ SỤT");
    }

    #[test]
    fn tcvn3_text_reads_in_unicode() {
        let t = LegacyVi::Tcvn3 { upper: false };
        assert_eq!(t.decode(&bytes(b"M\xc6t b\xbbng t\xadng \xae\xd3 x\xa9y")), "Mặt bằng tưng để xây");
        assert_eq!(t.decode("plain text"), Cow::Borrowed("plain text"));
        // An H font draws every letter as a capital, ASCII included.
        let h = LegacyVi::Tcvn3 { upper: true };
        assert_eq!(h.decode(&bytes(b"M\xc6T TI\xd2N \xa7I\xd3N H\xd7NH")), "MẶT TIỀN ĐIỂN HÌNH");
        assert_eq!(h.decode(&bytes(b"m\xb8i t\xabn")), "MÁI TÔN");
    }

    #[test]
    fn vni_marks_join_their_vowels() {
        let v = LegacyVi::Vni;
        // ơ (F4) takes a tone after it: "ôù" is ớ.
        assert_eq!(v.decode(&bytes(b"Ma\xefi t\xf4\xf9i \xf1a\xe2u")), "Mại tới đâu");
        assert_eq!(v.decode(&bytes(b"MA\xc9T BA\xc8NG")), "MẮT BẰNG");
        assert_eq!(v.decode(&bytes(b"\xd1A\xc4N")), "ĐẬN");
        assert_eq!(v.decode(&bytes(b"tr\xf6\xf4\xf8ng h\xf6\xf4\xf9ng")), "trường hướng");
        // A mark that can't go on the letter before it stays as it is.
        assert_eq!(v.decode(&bytes(b"n\xe2")), bytes(b"n\xe2"));
    }

    #[test]
    fn a_font_named_vn_guesses_from_the_text() {
        let g = LegacyVi::Guess { upper: false };
        assert_eq!(g.decode(&bytes(b"\xae\xd3 x\xa9y")), "để xây");
        assert_eq!(g.decode(&bytes(b"tr\xf6\xf4\xf8ng")), "trường");
        assert_eq!(LegacyVi::Guess { upper: true }.decode(&bytes(b"M\xc6t")), "MẶT");
        // Already Unicode: left alone.
        assert_eq!(g.decode("Mặt bằng"), Cow::Borrowed("Mặt bằng"));
    }
}
