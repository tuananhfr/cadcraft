//! Vietnamese composition and search folding stay local to the UI; other scripts pass through.

use std::borrow::Cow;

const TONES: [(char, &str); 24] = [
    ('a', "àáảãạ"),
    ('ă', "ằắẳẵặ"),
    ('â', "ầấẩẫậ"),
    ('e', "èéẻẽẹ"),
    ('ê', "ềếểễệ"),
    ('i', "ìíỉĩị"),
    ('o', "òóỏõọ"),
    ('ô', "ồốổỗộ"),
    ('ơ', "ờớởỡợ"),
    ('u', "ùúủũụ"),
    ('ư', "ừứửữự"),
    ('y', "ỳýỷỹỵ"),
    ('A', "ÀÁẢÃẠ"),
    ('Ă', "ẰẮẲẴẶ"),
    ('Â', "ẦẤẨẪẬ"),
    ('E', "ÈÉẺẼẸ"),
    ('Ê', "ỀẾỂỄỆ"),
    ('I', "ÌÍỈĨỊ"),
    ('O', "ÒÓỎÕỌ"),
    ('Ô', "ỒỐỔỖỘ"),
    ('Ơ', "ỜỚỞỠỢ"),
    ('U', "ÙÚỦŨỤ"),
    ('Ư', "ỪỨỬỮỰ"),
    ('Y', "ỲÝỶỸỴ"),
];
const TONE_MARKS: [char; 5] = ['\u{0300}', '\u{0301}', '\u{0309}', '\u{0303}', '\u{0323}'];
const SHAPES: [(char, char, char); 12] = [
    ('a', '\u{0306}', 'ă'),
    ('a', '\u{0302}', 'â'),
    ('e', '\u{0302}', 'ê'),
    ('o', '\u{0302}', 'ô'),
    ('o', '\u{031b}', 'ơ'),
    ('u', '\u{031b}', 'ư'),
    ('A', '\u{0306}', 'Ă'),
    ('A', '\u{0302}', 'Â'),
    ('E', '\u{0302}', 'Ê'),
    ('O', '\u{0302}', 'Ô'),
    ('O', '\u{031b}', 'Ơ'),
    ('U', '\u{031b}', 'Ư'),
];

fn shape(letter: char, mark: char) -> Option<char> {
    SHAPES.iter().find_map(|&(base, accent, composed)| (base == letter && accent == mark).then_some(composed))
}

fn tone(letter: char, index: usize) -> Option<char> {
    TONES.iter().find(|(base, _)| *base == letter).and_then(|(_, row)| row.chars().nth(index))
}

fn compose(letter: char, mark: char) -> Option<char> {
    if let Some(index) = TONE_MARKS.iter().position(|&accent| accent == mark) {
        return tone(letter, index);
    }
    if let Some(composed) = shape(letter, mark) {
        return Some(composed);
    }
    // NFD puts a dot below before a breve/circumflex, and a horn before every tone.
    TONES.iter().find_map(|&(base, row)| {
        let index = row.chars().position(|c| c == letter)?;
        if index != 4 && mark != '\u{031b}' {
            return None;
        }
        tone(shape(base, mark)?, index)
    })
}

/// Compose Vietnamese vowels with up to two following marks; this is not general Unicode NFC.
/// Unchanged input is borrowed, and unsupported or excess combining marks remain intact.
pub fn nfc(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let mut output: Option<String> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((start, mut letter)) = chars.next() {
        let mut marks = [None; 2];
        for pending in &mut marks {
            let Some((_, mark)) = chars.peek().copied().filter(|(_, c)| matches!(*c, '\u{0300}'..='\u{036f}')) else { break };
            chars.next();
            *pending = Some(mark);
        }
        let mut changed = false;
        let mut blocked = false;
        for pending in &mut marks {
            if let Some(mark) = *pending {
                if !blocked && let Some(composed) = compose(letter, mark) {
                    letter = composed;
                    *pending = None;
                    changed = true;
                } else {
                    blocked = true;
                }
            }
        }
        if changed && output.is_none() {
            let mut composed = String::with_capacity(text.len());
            if let Some(prefix) = text.get(..start) {
                composed.push_str(prefix);
            }
            output = Some(composed);
        }
        if let Some(composed) = &mut output {
            composed.push(letter);
            composed.extend(marks.into_iter().flatten());
        }
    }
    output.map_or(Cow::Borrowed(text), Cow::Owned)
}

fn strip(letter: char) -> char {
    let base = TONES.iter().find_map(|&(base, row)| row.contains(letter).then_some(base)).unwrap_or(letter);
    match base {
        'ă' | 'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ơ' | 'ò'..='ö' | 'ø' => 'o',
        'ư' | 'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'đ' => 'd',
        other => other,
    }
}

/// Lowercase normalized text and remove Vietnamese and Latin-1 accents for command search.
pub fn fold(text: &str) -> String {
    nfc(text).chars().flat_map(char::to_lowercase).map(strip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_normalization::UnicodeNormalization;

    #[test]
    fn nfc_composes_every_vietnamese_letter() {
        let letters = concat!(
            "àáảãạằắẳẵặầấẩẫậèéẻẽẹềếểễệìíỉĩịòóỏõọồốổỗộờớởỡợùúủũụừứửữựỳýỷỹỵăâêôơưđ",
            "ÀÁẢÃẠẰẮẲẴẶẦẤẨẪẬÈÉẺẼẸỀẾỂỄỆÌÍỈĨỊÒÓỎÕỌỒỐỔỖỘỜỚỞỠỢÙÚỦŨỤỪỨỬỮỰỲÝỶỸỴĂÂÊÔƠƯĐ",
        );
        assert_eq!(letters.chars().count(), 134);
        assert_eq!(letters.chars().collect::<std::collections::BTreeSet<_>>().len(), 134);
        for letter in letters.chars() {
            let expected = letter.to_string();
            let decomposed = expected.nfd().collect::<String>();
            assert_eq!(nfc(&decomposed), expected, "U+{:04X}", u32::from(letter));
            assert!(matches!(nfc(&expected), Cow::Borrowed(_)), "{letter}");
        }
        let decomposed = letters.nfd().collect::<String>();
        assert_eq!(nfc(&decomposed), letters);
        assert_eq!(nfc("prefix a\u{0306}\u{0301} e\u{0323}\u{0302} suffix"), "prefix ắ ệ suffix");
    }

    #[test]
    fn nfc_leaves_other_text_unchanged() {
        for text in ["e\u{0308}", "Ŝ", "abc", "", "Українська 🦀", "Ω\u{0301}", "x\u{0301}", "\u{0301}\u{0306}"] {
            assert_eq!(nfc(text), text);
            assert!(matches!(nfc(text), Cow::Borrowed(_)), "{text:?}");
        }
        assert_eq!(nfc("a\u{0323}\u{0306}\u{0301}"), "ặ\u{0301}");
        assert_eq!(nfc("a\u{0308}\u{0301}"), "a\u{0308}\u{0301}");
        assert_eq!(nfc("e\u{0308} a\u{0301} 🦀 z\u{0308}"), "e\u{0308} á 🦀 z\u{0308}");
        let excess = format!("a{}", "\u{0301}".repeat(4096));
        assert_eq!(nfc(&excess), format!("á{}", "\u{0301}".repeat(4095)));
    }

    #[test]
    fn fold_strips_marks() {
        assert_eq!(fold("Lớp bản vẽ"), "lop ban ve");
        assert_eq!(fold("ĐƯỜNG"), "duong");
        assert_eq!(fold("Lo\u{031b}\u{0301}p"), "lop");
        assert_eq!(fold("Café"), "cafe");
        assert_eq!(fold("ÀÁÂÃÄÅ Ç ÈÉÊË ÌÍÎÏ Ñ ÒÓÔÕÖØ ÙÚÛÜ ÝŸ"), "aaaaaa c eeee iiii n oooooo uuuu yy");
        assert_eq!(fold("ăâêôơưđ ĂÂÊÔƠƯĐ"), "aaeooud aaeooud");
        assert_eq!(fold("Æ Ð Þ ß × ÷ Україна 🦀"), "æ ð þ ß × ÷ україна 🦀");
        assert_eq!(fold("e\u{0308}"), "e\u{0308}");
        assert_eq!(fold("a\u{0306}\u{0301}\u{0309}"), "a\u{0309}");
        assert_eq!(fold(""), "");
    }
}
