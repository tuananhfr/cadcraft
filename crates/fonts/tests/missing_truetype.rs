//! A missing TrueType font's Latin outline face gives way to the user's choices: FONTALT and the
//! replacement table. In a process of its own, since FONTALT is process-wide.

use cadcraft_fonts::{FontState, TextFont, font_info, set_font_map, ttf};

#[test]
fn fontalt_and_the_replacement_table_win_over_the_latin_outline() {
    let Some(installed) = ["Segoe UI", "DejaVuSans", "Verdana", "Arial", "Helvetica"].into_iter().find(|n| ttf::find(n).is_some()) else {
        return; // No outline font here: nothing to choose between.
    };
    set_font_map([]);
    ttf::set_font_alt(installed);
    let alt = font_info("no-such-font-143.ttf", "");
    assert_eq!((alt.state, alt.with.as_deref(), alt.mapped), (FontState::Missing, Some(installed), false));
    ttf::set_font_alt("");

    set_font_map([("no-such-font-143.ttf".to_string(), installed.to_string())]);
    let mapped = font_info("no-such-font-143.ttf", "");
    assert_eq!((mapped.with.as_deref(), mapped.mapped), (Some(installed), true));
    set_font_map([]);

    // Neither set: the Latin outline font draws it, and an SHX font keeps the stroke font.
    let latin = font_info("no-such-font-143.ttf", "");
    assert!(!latin.mapped);
    assert_eq!(TextFont::for_style("no-such-font-143.ttf", "").is_outline(), latin.with.is_some());
    assert!(!TextFont::for_style("no-such-font-143.shx", "").is_outline());
}
