//! Fonts the user adds, and the replacement table, in a process of their own: a font added here
//! changes which fonts the missing-glyph search finds, which other tests would see.

use cadcraft_fonts::{FontState, TextFont, font_info, set_font_map, ttf};

/// A real font from the craft-fonts checkout the build uses, if there is one.
fn sample_font() -> Option<Vec<u8>> {
    let dir = std::env::var("CRAFT_FONTS_DIR").ok()?;
    std::fs::read(format!("{dir}/fonts/noto-sans/NotoSans-Regular.ttf")).ok()
}

#[test]
fn a_user_font_is_found_by_file_name_and_family_and_can_be_removed() {
    assert!(ttf::add_user_font("C:\\Fonts\\Not-A-Font-140.ttf", vec![0, 1, 0, 0, 0, 0, 0, 0]).is_err());
    assert!(ttf::add_user_font("", vec![1]).is_err());
    let Some(bytes) = sample_font() else { return };
    let added = ttf::add_user_font("C:\\Fonts\\VHBLACK-140.TTF", bytes).unwrap();
    assert_eq!(added.stem, "vhblack-140");
    assert_eq!(added.families.first().map(String::as_str), Some("Noto Sans"));
    assert_eq!(ttf::user_fonts(), vec![added.clone()]);
    assert!(ttf::font_choices().iter().any(|c| c.name == "Noto Sans" && c.origin == ttf::FontOrigin::Added));

    // By the file name a drawing's style names, and by a family it states.
    assert_eq!(font_info("VHBLACK-140.TTF", ".VnBlackH").state, FontState::Found);
    assert_eq!(font_info("other-140.ttf", "Noto Sans").state, FontState::Found);
    assert!(TextFont::for_style("VHBLACK-140.TTF", ".VnBlackH").is_outline());

    // Adding it again replaces it rather than listing it twice.
    let again = ttf::add_user_font("vhblack-140.ttf", sample_font().unwrap_or_default()).unwrap();
    assert_eq!(ttf::user_fonts(), vec![again]);

    // A replacement is drawn for a missing font and the text is still decoded.
    set_font_map([("Missing-140.TTF".to_string(), "Noto Sans".to_string())]);
    let m = font_info("missing-140.ttf", ".VnMissingH");
    assert_eq!((m.state, m.with.as_deref(), m.mapped), (FontState::Missing, Some("Noto Sans"), true));
    assert!(m.legacy.is_some());
    assert!(TextFont::for_style("missing-140.ttf", ".VnMissingH").is_outline());
    set_font_map([]);
    assert!(!TextFont::for_style("missing-140.ttf", "").is_outline());

    assert!(ttf::remove_user_font("vhblack-140"));
    assert_eq!(font_info("VHBLACK-140.TTF", ".VnBlackH").state, FontState::Missing);
    assert!(!ttf::remove_user_font("vhblack-140"));
    assert!(ttf::user_fonts().is_empty());
}
