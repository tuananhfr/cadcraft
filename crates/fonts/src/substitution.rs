//! What a text style's font becomes when it isn't there: the user's replacement table, then
//! FONTALT, then a Latin outline font for a TrueType font, then the stroke font. The table is
//! process-wide, like FONTALT, and never changes a drawing: it only decides what is drawn.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::shx::{self, ShxFont, is_standard_shx};
use crate::{BUILTIN_FONT, LegacyVi, ttf};

/// Entries and name length a replacement table keeps (it is read from untrusted saved settings).
const MAX_ENTRIES: usize = 256;
const MAX_NAME_CHARS: usize = 255;

fn table() -> MutexGuard<'static, BTreeMap<String, String>> {
    static T: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    T.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner)
}

/// The replacement table: font file name (lower case, no extension) → the font to draw instead.
/// The built-in stroke font's name stands for the stroke font.
pub fn font_map() -> Vec<(String, String)> {
    table().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Replace the whole table. Entries with an empty name or replacement are dropped.
pub fn set_font_map(entries: impl IntoIterator<Item = (String, String)>) {
    let mut t = table();
    t.clear();
    for (from, to) in entries {
        let (from, to) = (ttf::key(&from), to.trim().chars().take(MAX_NAME_CHARS).collect::<String>());
        if !from.is_empty() && !to.is_empty() && t.len() < MAX_ENTRIES {
            t.insert(from, to);
        }
    }
}

/// The font a user chose for the font `font`, if any.
pub fn replacement(font: &str) -> Option<String> {
    let k = ttf::key(font);
    if k.is_empty() { None } else { table().get(&k).cloned() }
}

/// Where a text style's font stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontState {
    /// The font (or its family) is installed, added or built in.
    Found,
    /// A stroke font by name (an SHX file or the built-in one): drawn with ours unless replaced.
    Stroke,
    /// Not available here: drawn with a replacement.
    Missing,
}

/// A text style's font and what is drawn for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontInfo {
    pub state: FontState,
    /// The font drawn instead (`None`: the stroke font); only for a font that isn't `Found`.
    pub with: Option<String>,
    /// `with` is the user's choice, not FONTALT or the Latin outline font.
    pub mapped: bool,
    /// Text typed for a legacy Vietnamese font is shown decoded.
    pub legacy: Option<LegacyVi>,
}

pub(crate) struct Resolved {
    pub face: Option<ttf::Face>,
    pub shx: Option<Arc<ShxFont>>,
    pub info: FontInfo,
}

/// A stroke-font name nobody ships (`vhtime.shx`) is reported missing; the stroke font still
/// draws it. The standard names, the built-in font and "no font" stay silent.
fn missing_state(k: &str) -> FontState {
    if ttf::is_stroke_name(k) && !(k.ends_with(".shx") && !is_standard_shx(k)) { FontState::Stroke } else { FontState::Missing }
}

/// Look a style's font up: its file, then its family, then a user's SHX font, then a replacement.
pub(crate) fn resolve(font: &str, family: &str) -> Resolved {
    let family = family.trim();
    if let Some(f) = ttf::find(font).or_else(|| if family.is_empty() { None } else { ttf::find(family) }) {
        return Resolved { face: Some(f), shx: None, info: FontInfo { state: FontState::Found, with: None, mapped: false, legacy: None } };
    }
    // Glyphs are looked up by the text's own character codes, so no legacy decoding applies.
    if let Some(shx) = shx::store::find(font) {
        return Resolved { face: None, shx: Some(shx), info: FontInfo { state: FontState::Found, with: None, mapped: false, legacy: None } };
    }
    let legacy = LegacyVi::from_name(family).or_else(|| LegacyVi::from_name(font));
    let state = missing_state(&ttf::key(font));
    let chosen = replacement(font).or_else(|| replacement(family));
    let (face, with, mapped) = match chosen {
        Some(r) if r.eq_ignore_ascii_case(BUILTIN_FONT) => (None, None, true),
        Some(r) => match ttf::find(&r) {
            Some(f) => (Some(f), Some(r), true),
            None => alternative(font),
        },
        None => alternative(font),
    };
    let (face, with) = if face.is_none() && with.is_none() && !mapped && is_truetype(font, family) { latin_outline() } else { (face, with) };
    Resolved { face, shx: None, info: FontInfo { state, with, mapped, legacy } }
}

/// Faces drawn for a missing TrueType font, first installed wins (the web embeds Noto Sans).
/// Its text was laid out in proportional outlines, which the wider stroke font overruns.
const LATIN_OUTLINE: &[&str] = &["Arial", "Helvetica", "Noto Sans", "DejaVuSans", "LiberationSans-Regular"];

/// A style font that names a TrueType file, or a font with its TrueType family; never a stroke font.
fn is_truetype(font: &str, family: &str) -> bool {
    let k = ttf::key(font);
    if ttf::is_stroke_name(&k) {
        return false;
    }
    let lower = font.trim().to_ascii_lowercase();
    [".ttf", ".ttc", ".otf", ".otc"].iter().any(|e| lower.ends_with(e)) || !family.is_empty()
}

fn latin_outline() -> (Option<ttf::Face>, Option<String>) {
    LATIN_OUTLINE.iter().find_map(|n| ttf::find(n).map(|f| (Some(f), Some((*n).to_string())))).unwrap_or((None, None))
}

/// FONTALT, for a font that isn't the built-in one.
fn alternative(font: &str) -> (Option<ttf::Face>, Option<String>, bool) {
    let n = font.trim();
    if n.is_empty() || n.eq_ignore_ascii_case(BUILTIN_FONT) {
        return (None, None, false);
    }
    let alt = ttf::font_alt();
    match ttf::find(&alt) {
        Some(f) => (Some(f), Some(alt), false),
        None => (None, None, false),
    }
}

/// Where the font of a text style stands, for a report.
pub fn font_info(font: &str, family: &str) -> FontInfo {
    resolve(font, family).info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_missing_here_is_reported_with_its_legacy_encoding() {
        let _serial = ttf::test_lock();
        set_font_map([]);
        ttf::set_font_alt("");
        let missing = font_info("no-such-font-140.ttf", ".VnNoSuchH");
        assert_eq!((missing.state, missing.with.clone(), missing.mapped), (FontState::Missing, latin(), false));
        assert_eq!(missing.legacy, Some(LegacyVi::Tcvn3 { upper: true }));
        assert_eq!(font_info("romans.shx", "").state, FontState::Stroke);
        assert_eq!(font_info("", "").state, FontState::Stroke);
        // A replacement that is the stroke font draws none; one that is missing counts as none,
        // so a missing TrueType font takes the Latin outline face.
        set_font_map([("shx-140.shx".to_string(), BUILTIN_FONT.to_string()), ("gone-140.ttf".to_string(), "no-such-font-141".to_string())]);
        let shx = resolve("shx-140.shx", "");
        assert!(shx.face.is_none() && shx.info.mapped && shx.info.with.is_none());
        let gone = font_info("gone-140.ttf", "");
        assert_eq!((gone.with, gone.mapped), (latin(), false));
        set_font_map([]);
    }

    /// The Latin outline face a missing TrueType font is drawn with here, if any.
    fn latin() -> Option<String> {
        LATIN_OUTLINE.iter().find(|n| ttf::find(n).is_some()).map(|n| n.to_string())
    }

    #[test]
    fn missing_ttf_uses_latin_outline() {
        let _serial = ttf::test_lock();
        set_font_map([]);
        ttf::set_font_alt("");
        if ttf::find("VHARIAL.TTF").is_some() {
            return; // Installed here: nothing is missing.
        }
        let latin = latin();
        let r = resolve("VHARIAL.TTF", "");
        assert_eq!(r.face.is_some(), latin.is_some());
        assert_eq!((r.info.state, r.info.with.clone(), r.info.mapped), (FontState::Missing, latin.clone(), false));
        assert_eq!(r.info.legacy, Some(LegacyVi::Guess { upper: true }));
        assert_eq!(crate::TextFont::for_style("VHARIAL.TTF", "").is_outline(), latin.is_some());
        // A font file without an extension is TrueType when the style names its family.
        assert_eq!(font_info("vharial", "VH Arial").with, latin);
        assert_eq!(font_info("vharial", "").with, None);
        // Missing SHX fonts keep the stroke font.
        for shx in ["chhbg.shx", "vhtime.shx", "romans.shx", "", "CADCraft Stroke"] {
            assert!(resolve(shx, "").face.is_none(), "{shx}");
            assert_eq!(font_info(shx, "").with, None, "{shx}");
        }
        // The user's choice of the stroke font stands. (An installed FONTALT winning is in
        // `tests/missing_truetype.rs`: setting it here would change unlocked tests' fonts.)
        set_font_map([("vharial.ttf".to_string(), BUILTIN_FONT.to_string())]);
        let stroke = resolve("VHARIAL.TTF", "");
        assert!(stroke.face.is_none() && stroke.info.with.is_none() && stroke.info.mapped);
        set_font_map([]);
    }

    #[test]
    fn the_table_drops_empty_entries_and_keeps_keys_lower_case() {
        let _serial = ttf::test_lock();
        set_font_map([("A.TTF".to_string(), "Arial".to_string()), (String::new(), "x".to_string()), ("b.ttf".to_string(), " ".to_string())]);
        assert_eq!(font_map(), vec![("a".to_string(), "Arial".to_string())]);
        assert_eq!(replacement("a.ttf").as_deref(), Some("Arial"));
        assert_eq!(replacement(""), None);
        set_font_map([]);
    }
}
