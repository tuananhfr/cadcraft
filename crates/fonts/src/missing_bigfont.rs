//! Characters of SHX bigfonts CADCraft can't read, for the codes where AutoCAD's output shows
//! what the font draws. Display only: the stored text never changes.
//!
//! A bigfont takes over two-byte codes from a lead byte on. Bigfonts aren't read at all (only
//! shapes fonts and unifonts are), so without this each byte is drawn as its own character.

use std::borrow::Cow;

use crate::stroke::TONES;

/// A missing bigfont with known codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingBigFont {
    /// The character starting each two-character code.
    lead: char,
    /// (second character of a code, what the code draws).
    codes: &'static [(char, Code)],
}

/// What a bigfont code draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Code {
    /// A character of its own.
    Char(char),
    /// A mark on the nearest vowel before it in the same word.
    Mark(Mark),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Circumflex,
    /// Breve on A, horn on O and U.
    BreveOrHorn,
    /// A tone, as an index into a [`TONES`] row: grave, acute, hook above, tilde, dot below.
    Tone(usize),
}

/// `vns.shx`, a Vietnamese bigfont with `/` as its lead byte (its text is typed like `THE/SP`):
/// letters `/D` Đ, `/}` Ư, `/{` Ơ; marks typed after their vowel, `/M` circumflex, `/W` breve or
/// horn, tones `/F` `/S` `/R` `/X` `/J`; `/g` and `/G` the diameter sign; `//` a slash.
/// Evidence: inferred from one drawing's notes and the PDF AutoCAD plotted from it (`/g12a150` is
/// `⌀12a150`, `BE/M TO/MNG` is `BÊ TÔNG`, `/DA/S` is `ĐÁ`), not from the vns.shx file itself.
/// Other codes are unproven: left as typed.
const VNS: MissingBigFont = MissingBigFont {
    lead: '/',
    codes: &[
        ('g', Code::Char('⌀')),
        ('G', Code::Char('⌀')),
        ('D', Code::Char('Đ')),
        ('}', Code::Char('Ư')),
        ('{', Code::Char('Ơ')),
        ('/', Code::Char('/')),
        ('M', Code::Mark(Mark::Circumflex)),
        ('W', Code::Mark(Mark::BreveOrHorn)),
        ('F', Code::Mark(Mark::Tone(0))),
        ('S', Code::Mark(Mark::Tone(1))),
        ('R', Code::Mark(Mark::Tone(2))),
        ('X', Code::Mark(Mark::Tone(3))),
        ('J', Code::Mark(Mark::Tone(4))),
    ],
};

impl MissingBigFont {
    /// The known bigfont a style's bigfont file name (`vns.shx`, any case or folder) points to.
    pub fn from_name(name: &str) -> Option<MissingBigFont> {
        let lower = name.trim().to_ascii_lowercase();
        let file = lower.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = file.strip_suffix(".shx").unwrap_or(file);
        (stem == "vns").then_some(VNS)
    }

    /// `s` with each known two-character code drawn as its character or mark. A lead character
    /// takes the next one with it whether the code is known or not (`//g` is the code `//`, then
    /// `g`); a mark with no vowel before it that can take it is left as typed.
    pub fn decode(self, s: &str) -> Cow<'_, str> {
        if !s.contains(self.lead) {
            return Cow::Borrowed(s);
        }
        let mut out: Vec<char> = Vec::with_capacity(s.len());
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
            let drawn = match self.codes.iter().find(|(k, _)| *k == second).map(|(_, code)| *code) {
                Some(Code::Char(d)) => {
                    out.push(d);
                    true
                }
                Some(Code::Mark(m)) => mark_last_vowel(&mut out, m),
                None => false,
            };
            if !drawn {
                out.push(c);
                out.push(second);
            }
        }
        let out: String = out.into_iter().collect();
        if out == s { Cow::Borrowed(s) } else { Cow::Owned(out) }
    }
}

/// Put `m` on the nearest vowel of the word `out` ends with. False (nothing changed) when there
/// is none or that vowel can't take it.
fn mark_last_vowel(out: &mut [char], m: Mark) -> bool {
    let Some(v) = out.iter_mut().rev().take_while(|c| c.is_alphabetic()).find(|c| vowel(**c).is_some()) else {
        return false;
    };
    match marked(*v, m) {
        Some(n) => {
            *v = n;
            true
        }
        None => false,
    }
}

/// A Vietnamese vowel as (its letter with breve, circumflex or horn; its tone).
fn vowel(c: char) -> Option<(char, Option<usize>)> {
    TONES.iter().find_map(|(b, row)| if *b == c { Some((*b, None)) } else { row.chars().position(|t| t == c).map(|i| (*b, Some(i))) })
}

/// `c` with `m` added; `None` when `c` already has that kind of mark or can't take it.
fn marked(c: char, m: Mark) -> Option<char> {
    const CIRCUMFLEX: &[(char, char)] = &[('a', 'â'), ('e', 'ê'), ('o', 'ô'), ('A', 'Â'), ('E', 'Ê'), ('O', 'Ô')];
    const BREVE_OR_HORN: &[(char, char)] = &[('a', 'ă'), ('o', 'ơ'), ('u', 'ư'), ('A', 'Ă'), ('O', 'Ơ'), ('U', 'Ư')];
    let (letter, tone) = vowel(c)?;
    let shaped = |table: &[(char, char)]| table.iter().find(|(plain, _)| *plain == letter).map(|(_, s)| *s);
    let (letter, tone) = match m {
        Mark::Tone(i) if tone.is_none() => (letter, Some(i)),
        Mark::Tone(_) => return None,
        Mark::Circumflex => (shaped(CIRCUMFLEX)?, tone),
        Mark::BreveOrHorn => (shaped(BREVE_OR_HORN)?, tone),
    };
    match tone {
        None => Some(letter),
        Some(i) => TONES.iter().find(|(b, _)| *b == letter).and_then(|(_, row)| row.chars().nth(i)),
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
        let vns = vns();
        assert_eq!(vns.decode("/g12a150"), "⌀12a150");
        assert_eq!(vns.decode("12/g14a50 + 3/G16"), "12⌀14a50 + 3⌀16");
        assert_eq!(vns.decode("Ö/g/g"), "Ö⌀⌀");
        assert_eq!(vns.decode("//g"), "/g");
        // Unknown codes and plain text are left as typed.
        for s in ["1/2", "a/", "/", "", "ø", "plain"] {
            assert_eq!(vns.decode(s), Cow::Borrowed(s), "{s}");
        }
    }

    #[test]
    fn vns_marks_compose() {
        let vns = vns();
        for (typed, drawn) in [
            ("THE/SP", "THÉP"),
            ("TO/M/RNG", "TỔNG"),
            ("CHIE/M/FU", "CHIỀU"),
            ("HO/M/JP", "HỘP"),
            ("L/}/{/JNG", "LƯỢNG"),
            ("MA/W/JT", "MẶT"),
            ("/DA/S", "ĐÁ"),
            ("KY/S", "KÝ"),
            ("a//b", "a/b"),
            ("/g12", "⌀12"),
            ("/G12", "⌀12"),
            ("/Q", "/Q"),
            // A tone before the shape mark, lower-case vowels, the horn on O.
            ("A/S/M", "Ấ"),
            ("the/Sp", "thép"),
            ("TO/WI", "TƠI"),
            ("BE/M TO/MNG", "BÊ TÔNG"),
        ] {
            assert_eq!(vns.decode(typed), drawn, "{typed}");
        }
    }

    #[test]
    fn vns_mark_without_vowel_kept() {
        let vns = vns();
        // No vowel in the word, a vowel that can't take the mark, a word break before the mark.
        for s in ["X/S", "/S", "I/M", "A /S", "1/J"] {
            assert_eq!(vns.decode(s), Cow::Borrowed(s), "{s}");
        }
        // A second tone isn't guessed at.
        assert_eq!(vns.decode("A/S/F"), "Á/F");
    }

    fn vns() -> MissingBigFont {
        MissingBigFont::from_name("vns.shx").unwrap_or(VNS)
    }
}
