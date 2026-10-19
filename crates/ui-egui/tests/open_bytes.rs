//! Opening a drawing from its contents, where the file has no path (the web's File > Open… picker
//! and files dropped on the page, #55).

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::Drawing;
use cadcraft_ui_egui::{CadApp, Services};

#[test]
fn open_bytes_opens_the_named_drawing_and_reports_failures() {
    // A reader that only accepts what a DXF starts with: proves the bytes arrive unchanged.
    set_io(IoHooks {
        read: |b, _| if b.starts_with(b"0\nSECTION\n") { Ok(Drawing::new_imperial()) } else { Err("not a drawing".into()) },
        write: |_, _| Ok(Vec::new()),
        plot: None,
    });
    let mut app = CadApp::new(Session::empty(), Services::default());
    app.ui.start_tab = true;

    app.open_bytes("Bracket.dwg", b"0\nSECTION\n2\nHEADER\n");
    assert_eq!(app.session.docs.len(), 1);
    assert_eq!(app.session.state().map(|s| s.title.clone()).ok().as_deref(), Some("Bracket.dwg"));
    assert!(!app.ui.start_tab && app.canvas.zoom_pending, "shown and zoomed like a drawing opened from a path");

    // A file the reader rejects: nothing opens; the error reaches the command line and status bar.
    app.open_bytes("junk.dxf", b"junk");
    assert_eq!(app.session.docs.len(), 1);
    assert!(app.session.log.last().is_some_and(|l| l.contains("not a drawing")), "{:?}", app.session.log.last());
    assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("not a drawing")));

    // Too large to hold in memory: refused before anything is read, with the limit named.
    assert!(CadApp::open_size_error("big.dxf", CadApp::MAX_OPEN_BYTES).is_none());
    let e = CadApp::open_size_error("huge.dxf", CadApp::MAX_OPEN_BYTES * 2);
    assert_eq!(e.as_deref(), Some("Open: huge.dxf is 512.0 MB, larger than the 256 MB CADCraft opens in the browser"));
    app.open_failed(e.unwrap_or_default());
    assert_eq!(app.session.log.last().map(String::as_str), Some("Open: huge.dxf is 512.0 MB, larger than the 256 MB CADCraft opens in the browser"));
    assert_eq!(app.session.docs.len(), 1);
}

#[test]
fn a_dwg_too_large_for_wasm_memory_is_refused_by_name_or_contents() {
    // Converting a DWG peaks near 40 times its size; the cap keeps it under wasm32's 4 GB.
    assert!(CadApp::open_size_error("site.dwg", CadApp::MAX_OPEN_DWG_BYTES).is_none());
    assert_eq!(
        CadApp::open_size_error("site.DWG", CadApp::MAX_OPEN_DWG_BYTES + (10 << 20)).as_deref(),
        Some("Open: site.DWG is 100.0 MB; the browser opens DWG files up to 90 MB. Open it in the desktop app.")
    );
    assert!(CadApp::open_size_error("template.dwt", CadApp::MAX_OPEN_DWG_BYTES + 1).is_some());
    assert!(CadApp::open_size_error("site.dxf", CadApp::MAX_OPEN_DWG_BYTES + 1).is_none(), "a DXF keeps the general limit");

    // A DWG under another name is caught by its AC10xx magic before it is read (so no reader is
    // installed: the hooks are global, shared with the test above).
    let mut app = CadApp::new(Session::empty(), Services::default());
    let mut dwg = vec![0u8; usize::try_from(CadApp::MAX_OPEN_DWG_BYTES + 1).unwrap_or_default()];
    if let Some(magic) = dwg.get_mut(..6) {
        magic.copy_from_slice(b"AC1021");
    }
    app.open_bytes("Drawing.dxf", &dwg);
    assert!(app.session.docs.is_empty());
    assert!(app.session.log.last().is_some_and(|l| l.contains("DWG files up to 90 MB")), "{:?}", app.session.log.last());
}
