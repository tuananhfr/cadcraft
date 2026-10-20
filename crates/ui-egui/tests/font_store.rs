//! The desktop keeps the fonts a user added in a folder and adds them back at the next start. In a
//! test binary of its own: an added font changes which fonts every other text lookup finds.

use cadcraft_engine::Session;
use cadcraft_fonts::{CRAFT_FONTS, ttf};
use cadcraft_ui_egui::fontstore::FontStore;
use cadcraft_ui_egui::{CadApp, Services};

fn app() -> CadApp {
    CadApp::new(Session::new(), Services::default())
}

#[test]
fn added_fonts_are_kept_in_a_folder_and_come_back() {
    let Some(font) = CRAFT_FONTS.iter().find(|f| f.style == "Regular") else { return };
    let dir = std::env::temp_dir().join(format!("cadcraft-font-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FontStore::new(dir.join("fonts"));

    let mut first = app();
    first.add_font(r"C:\Fonts\KEPT-160.TTF", font.bytes.to_vec()).unwrap();
    store.sync(&mut first);
    assert!(dir.join("fonts").join("KEPT-160.TTF").is_file(), "{:?}", std::fs::read_dir(dir.join("fonts")).map(|d| d.count()));
    assert!(first.take_font_changes().is_empty(), "a kept change is not reported again");

    // A new run: forget the font in the process, then load the folder.
    assert!(ttf::remove_user_font("kept-160"));
    let mut second = app();
    store.load(&mut second);
    assert_eq!(ttf::user_fonts().iter().map(|f| f.stem.as_str()).collect::<Vec<_>>(), ["kept-160"]);
    assert!(second.take_font_changes().is_empty(), "restored fonts are not new changes");

    second.remove_font("kept-160");
    store.sync(&mut second);
    assert!(!dir.join("fonts").join("KEPT-160.TTF").exists());
    assert!(ttf::user_fonts().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
