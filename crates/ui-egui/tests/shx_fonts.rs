//! SHX fonts in the Fonts dialog and in the notice after a drawing opens. In a test binary of its
//! own: the fonts a user adds are process-wide, and other binaries must not see them.

use std::sync::{Mutex, MutexGuard, PoisonError};

use cadcraft_engine::Session;
use cadcraft_fonts::{CRAFT_FONTS, ttf};
use cadcraft_ui_egui::fontmgr::FontChange;
use cadcraft_ui_egui::fontstore::FontStore;
use cadcraft_ui_egui::{CadApp, Services, dialogs};
use serde_json::json;

fn serial() -> MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A tiny shapes font: one glyph, 'A', a vertical bar.
fn shx_bytes() -> Vec<u8> {
    let header: Vec<u8> = b"VHTIME\0\x0a\x02\0\0\0".to_vec();
    let a: Vec<u8> = b"A\0\xA4\x02\x08\x08\xF6\0".to_vec();
    let mut out = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0x41u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    for (code, body) in [(0u16, &header), (0x41, &a)] {
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    }
    out.extend_from_slice(&header);
    out.extend_from_slice(&a);
    out.extend_from_slice(b"EOF");
    out
}

/// A drawing with one text per style, each style set in the given font.
fn app_with_styles(fonts: &[(&str, &str)]) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    for (name, font) in fonts {
        app.run("style", json!({ "name": name, "font": font })).unwrap();
        app.run("text", json!({ "at": [0, 0], "text": "ABC", "height": 2.5 })).unwrap();
    }
    app
}

fn style_state(app: &CadApp, style: &str) -> String {
    let report = cadcraft_engine::cmd::fonts::report(&app.session);
    let row = report["styles"].as_array().and_then(|rows| rows.iter().find(|r| r["style"] == style));
    row.and_then(|r| r["state"].as_str()).unwrap_or_default().to_string()
}

#[test]
fn fonts_dialog_lists_shx_found() {
    let _s = serial();
    let mut app = app_with_styles(&[("VhTime", "vhtime.shx")]);
    assert_eq!(style_state(&app, "VhTime"), "missing");

    let added = app.add_font("C:\\Fonts\\VHTIME.SHX", shx_bytes()).unwrap();
    assert_eq!((added.stem.as_str(), added.is_shx()), ("vhtime.shx", true));
    assert_eq!(style_state(&app, "VhTime"), "found");
    let report = cadcraft_engine::cmd::fonts::report(&app.session);
    assert_eq!(report["missing"], json!(0));

    // The dialog draws with the font listed under "Fonts you added" and the style marked found.
    app.ui.dialog = Some("fonts".into());
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| dialogs::show(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert_eq!(app.ui.dialog.as_deref(), Some("fonts"));
    assert!(ttf::user_fonts().iter().any(|f| f.stem == "vhtime.shx"));
    assert_eq!(app.take_font_changes(), vec![FontChange::Added { file_name: "C:\\Fonts\\VHTIME.SHX".into(), bytes: shx_bytes() }]);

    // A file that is not SHX is refused with the reader's message, and nothing is kept.
    let err = app.add_font("junk.shx", vec![1, 2, 3]).unwrap_err();
    assert_eq!(err, "Not a readable SHX file");
    assert!(app.take_font_changes().is_empty());

    app.remove_font("vhtime.shx");
    assert_eq!(app.take_font_changes(), vec![FontChange::Removed { stem: "vhtime.shx".into() }]);
    assert_eq!(style_state(&app, "VhTime"), "missing");
}

#[test]
fn open_notice_skips_standard_shx() {
    let _s = serial();
    let mut app = app_with_styles(&[("Romans", "romans.shx"), ("VhTime", "vhtime.shx")]);
    assert_eq!(style_state(&app, "Romans"), "stroke");
    app.notify_missing_fonts();
    let notice = app.session.log.last().cloned().unwrap_or_default();
    assert!(notice.starts_with("1 font missing: vhtime.shx."), "{notice}");
    assert!(!notice.contains("romans"), "{notice}");

    // Only standard names: nothing to say.
    let mut app = app_with_styles(&[("Romans", "romans.shx"), ("Iso", "ISOCP3.SHX")]);
    let before = app.session.log.len();
    app.notify_missing_fonts();
    assert_eq!(app.session.log.len(), before);
}

#[test]
fn misnamed_shx_is_kept_and_removed_under_its_own_key() {
    let _s = serial();
    let dir = std::env::temp_dir().join(format!("cadcraft-shx-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FontStore::new(dir.join("fonts"));
    let mut app = CadApp::new(Session::new(), Services::default());

    // A TrueType file `x.ttf` kept first (when a font is embedded), then SHX bytes named `x.ttf`.
    let ttf_bytes = CRAFT_FONTS.iter().find(|f| f.style == "Regular").map(|f| f.bytes.to_vec());
    if let Some(bytes) = &ttf_bytes {
        app.add_font("x.ttf", bytes.clone()).unwrap();
    }
    let added = app.add_font("x.ttf", shx_bytes()).unwrap();
    assert_eq!(added.stem, "x.shx");
    let changes = app.take_font_changes();
    assert_eq!(changes.last(), Some(&FontChange::Added { file_name: "x.shx".into(), bytes: shx_bytes() }));
    for change in &changes {
        match change {
            FontChange::Added { file_name, bytes } => {
                std::fs::create_dir_all(dir.join("fonts")).unwrap();
                std::fs::write(dir.join("fonts").join(file_name), bytes).unwrap();
            }
            FontChange::Removed { .. } => {}
        }
    }
    assert!(dir.join("fonts").join("x.shx").is_file());

    app.remove_font("x.shx");
    store.sync(&mut app);
    assert!(!dir.join("fonts").join("x.shx").exists(), "the kept SHX copy is gone");
    if ttf_bytes.is_some() {
        assert!(dir.join("fonts").join("x.ttf").is_file(), "the TrueType font with the same stem is untouched");
        app.remove_font("x");
        store.sync(&mut app);
        assert!(!dir.join("fonts").join("x.ttf").exists());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
