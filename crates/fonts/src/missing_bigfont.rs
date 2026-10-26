//! Characters of SHX bigfonts CADCraft can't read, for the few codes where AutoCAD's output
//! proves what the font draws. Display only: the stored text never changes.
//!
//! A bigfont takes over two-byte codes from a lead byte on. Bigfonts aren't read at all (only
//! shapes fonts and unifonts are), so without this each byte is drawn as its own character.

use std::borrow::Cow;

/// A missing bigfont with known characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingBigFont {
    /// The character starting each two-character code.
    lead: char,
    /// (second character of a code, what the code draws).
    codes: &'static [(char, char)],
}

/// `vns.shx`, a Vietnamese bigfont with `/` as its lead byte (its text is typed like `THE/SP`).
/// Evidence per code: AutoCAD plots the rebar label `/g12a150` in a style using it as one
/// diameter sign, one character wide, before `12a150`. Other codes are unproven: left as typed.
const VNS: MissingBigFont = MissingBigFont { lead: '/', codes: &[('g', '⌀')] };

impl MissingBigFont {
    /// The known bigfont a style's bigfont file name (`vns.shx`, any case or folder) points to.
    pub fn from_name(name: &str) -> Option<MissingBigFont> {
        let lower = name.trim().to_ascii_lowercase();
        let file = lower.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = file.strip_suffix(".shx").unwrap_or(file);
        (stem == "vns").then_some(VNS)
    }

    /// `s` with each known two-character code drawn as its character. A lead character takes the
    /// next one with it whether the code is known or not (`//g` is the code `//`, then `g`).
    pub fn decode(self, s: &str) -> Cow<'_, str> {
        if !s.contains(self.lead) {
            return Cow::Borrowed(s);
        }
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != self.lead {
                out.push(c);
                continue;
            }
            let Some(second) = chars.next() else {
                out.push(c);
                break;
            };
            match self.codes.iter().find(|(k, _)| *k == second) {
                Some((_, drawn)) => out.push(*drawn),
                None => {
                    out.push(c);
                    out.push(second);
                }
            }
        }
        if out == s { Cow::Borrowed(s) } else { Cow::Owned(out) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_vns_is_known() {
        for name in ["vns.shx", "VNS.SHX", "vns", " C:\\Fonts\\Vns.shx ", "fonts/vns.shx"] {
            assert!(MissingBigFont::from_name(name).is_some(), "{name}");
        }
        for name in ["", "vnsimli.shx", "chineset.shx", "vns.ttf", "avns.shx"] {
            assert!(MissingBigFont::from_name(name).is_none(), "{name}");
        }
    }

    #[test]
    fn slash_g_is_the_diameter_sign() {
        let Some(vns) = MissingBigFont::from_name("vns.shx") else { panic!("vns.shx is known") };
        assert_eq!(vns.decode("/g12a150"), "⌀12a150");
        assert_eq!(vns.decode("12/g14a50 + 3/g16"), "12⌀14a50 + 3⌀16");
        assert_eq!(vns.decode("Ö/g/g"), "Ö⌀⌀");
        assert_eq!(vns.decode("//g"), "//g");
        // Unproven codes and plain text are left as typed.
        for s in ["/G12A150", "CA/M/SU", "1/2", "a/", "/", "", "ø"] {
            assert_eq!(vns.decode(s), Cow::Borrowed(s), "{s}");
        }
    }
}
