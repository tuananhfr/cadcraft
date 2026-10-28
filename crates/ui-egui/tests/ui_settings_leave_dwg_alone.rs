//! Interface and workspace settings belong to the user, not the drawing (V4.2 QA-032): the same
//! drawing saved under another language, theme and workspace layout gives the same DWG bytes.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, WriterLosses, set_io, set_write_as, set_write_reported};
use cadcraft_ui_egui::{CadApp, Services};
use egui::vec2;
use egui_kittest::Harness;
use serde_json::{Value, json};

fn wire_real_io() {
    set_io(IoHooks { read: |b, n| cadcraft_io::read(b, n).map_err(|e| e.to_string()), write: |d, n| cadcraft_io::write(d, n).map_err(|e| e.to_string()), plot: None });
    set_write_as(|d, n, o| cadcraft_io::write_with(d, n, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary }).map_err(|e| e.to_string()));
    set_write_reported(|d, n, o| {
        cadcraft_io::write_reported(d, n, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary })
            .map(|(bytes, dropped)| (bytes, WriterLosses { entities: dropped.entities, objects: dropped.objects }))
            .map_err(|e| e.to_string())
    });
}

fn target(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("cadcraft-qa032-{tag}-{}.dwg", std::process::id()))
}

/// Draws the same plan, lets the interface run a few frames with `settings` applied, then saves
/// through the Save As dialog and returns the file's bytes.
fn draw_and_save(tag: &str, settings: impl FnOnce(&mut CadApp)) -> Vec<u8> {
    let path = target(tag);
    let picked = path.to_string_lossy().to_string();
    let services = Services { pick_save: Some(Box::new(move |_: &str| Some(picked.clone()))), ..Services::default() };
    let mut app = CadApp::new(Session::new(), services);
    settings(&mut app);
    app.run("line", json!({ "points": [[0, 0], [6000, 0], [6000, 4000], [0, 4000], [0, 0]] })).unwrap();
    app.run("line", json!({ "points": [[0, 2000], [6000, 2000]] })).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h.state_mut().run("ui.saveas", Value::Null).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    bytes
}

#[test]
fn language_theme_and_workspace_layout_do_not_change_the_saved_dwg() {
    wire_real_io();
    let plain = draw_and_save("plain", |_| {});
    let customised = draw_and_save("customised", |app| {
        app.run("ui.language", json!({ "lang": "vi" })).unwrap();
        assert_eq!(app.language(), "vi");
        app.run("ui.theme", json!({ "theme": "light" })).unwrap();
        app.set_session_layer(json!({
            "command_line": { "visible": false },
            "status_bar": { "visible": false },
            "panels": [{ "id": "layers", "visible": false, "mode": "hidden" }],
        }));
    });
    assert_eq!(plain.get(..6), Some(&b"AC1032"[..]), "saved as AutoCAD 2018 DWG");
    assert!(plain == customised, "the DWG differs ({} vs {} bytes)", plain.len(), customised.len());
}
