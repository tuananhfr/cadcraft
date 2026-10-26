//! The External References palette (#525): XREF opens it; its rows and buttons act through the
//! engine's -XREF commands.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::geom::Vec3;
use cadcraft_engine::doc::{Drawing, Entity, EntityKind, Handle, Layer, Line, XrefStatus};
use cadcraft_ui_egui::{CadApp, Services, docking::Panel};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// Test drawings: any file holds one line on layer Walls.
fn read(_bytes: &[u8], _name: &str) -> Result<Drawing, String> {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer { name: "Walls".into(), ..Layer::default() });
    let mut e = Entity::new(Handle(0x60), EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(50.0, 20.0, 0.0) }));
    e.common.layer = "Walls".into();
    d.model.push(e);
    Ok(d)
}

fn site_file() -> String {
    let dir = std::env::temp_dir().join(format!("cadcraft-xref-palette-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("site.dxf");
    std::fs::write(&p, "site").unwrap();
    p.to_string_lossy().into_owned()
}

fn status(app: &CadApp) -> Option<XrefStatus> {
    app.session.doc().ok()?.block("site").map(|b| b.xref_status)
}

#[test]
fn xref_opens_the_palette_and_its_buttons_manage_the_xref() {
    set_io(IoHooks { read, write: |_, _| Err("no writer in this test".into()), plot: None });
    let mut app = CadApp::new(Session::default(), Services::default());
    app.run("xattach", json!({ "path": site_file(), "at": [10, 10] })).unwrap();
    assert_eq!(status(&app), Some(XrefStatus::Loaded));
    // Typed (by its alias) it opens the palette instead of listing.
    assert!(app.cmdline("XR").is_none());
    assert!(app.ui.docking.layout.contains(&Panel::ExternalRefs));
    app.ui.in_window_menu = false;
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("site");
    h.get_by_label("Loaded");
    h.get_by_label("Unload").click();
    h.run_steps(3);
    assert_eq!(status(h.state()), Some(XrefStatus::Unloaded));
    h.get_by_label("Unloaded");
    h.get_by_label("Reload").click();
    h.run_steps(3);
    assert_eq!(status(h.state()), Some(XrefStatus::Loaded));
    h.get_by_label("Overlay").click();
    h.run_steps(3);
    assert!(h.state().session.doc().unwrap().block("site").unwrap().xref_overlay);
    h.get_by_label("Detach").click();
    h.run_steps(3);
    assert_eq!(status(h.state()), None);
    h.get_by_label("No external references are attached.");
    // Undo brings it back, and the palette lists it again.
    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    h.get_by_label("site");
}

#[test]
fn json_xref_lists_and_never_opens_the_palette() {
    let mut app = CadApp::new(Session::default(), Services::default());
    let r = app.run("xref", json!({})).unwrap();
    assert_eq!(r["xrefs"], json!([]));
    assert!(!app.ui.docking.layout.contains(&Panel::ExternalRefs));
}

/// A palette over a drawing whose xref `frame` is missing, with or without the host's picker
/// (which records the xrefs it is asked for).
fn missing_xref_palette(can_pick: bool) -> (Harness<'static, CadApp>, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
    use cadcraft_engine::doc::{Block, Insert, Space};
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut services = Services::default();
    if can_pick {
        let log = asked.clone();
        services.pick_xref_file = Some(Box::new(move |_, name: &str| log.borrow_mut().push(name.to_string())));
    }
    let mut app = CadApp::new(Session::default(), services);
    let d = app.session.doc_mut().unwrap();
    let mut b = Block::new("frame");
    b.xref_path = Some("Khung ten A3.dwg".into());
    b.xref_status = XrefStatus::NotFound;
    d.blocks.insert("frame".into(), std::sync::Arc::new(b));
    let insert = Insert {
        block: "frame".into(),
        insert: Vec3::ZERO,
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs: Vec::new(),
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
    };
    d.add(&Space::Model, Default::default(), EntityKind::Insert(insert)).unwrap();
    assert!(app.cmdline("XREF").is_none());
    app.ui.in_window_menu = false;
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("frame");
    (h, asked)
}

#[test]
fn choose_file_shows_for_a_missing_xref_only_with_the_host_picker() {
    let (h, _) = missing_xref_palette(false);
    assert!(h.query_by_label("Choose file…").is_none(), "the desktop has no picker for it");

    let (mut h, asked) = missing_xref_palette(true);
    h.get_by_label("Choose file…").click();
    h.run_steps(3);
    assert_eq!(*asked.borrow(), ["frame"]);
}

#[test]
fn the_desktop_palette_keeps_its_disk_controls_with_or_without_the_picker() {
    for can_pick in [false, true] {
        let (h, _) = missing_xref_palette(can_pick);
        for label in ["Attach DWG...", "Reload All", "Reload", "Unload", "Detach", "Bind", "Insert", "Open", "Apply"] {
            assert!(h.query_by_label(label).is_some(), "{label} (picker: {can_pick})");
        }
        assert!(h.query_by_label("On the web, use Choose file… to load").is_none());
    }
}
