//! STYLESMANAGER's Import .ctb/.stb…: the host's picker asked for plot style tables, and the
//! picked table loaded into the drawing and selected.

use std::cell::RefCell;
use std::rc::Rc;

use cadcraft_engine::Session;
use cadcraft_engine::doc::{PlotStyle, PlotStyleTable};
use cadcraft_ui_egui::plotstyles::PLOT_STYLE_FILE;
use cadcraft_ui_egui::{CadApp, FileKind, Services};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

type Asked = Rc<RefCell<Vec<FileKind>>>;

fn app(services: Services) -> CadApp {
    let mut app = CadApp::new(Session::new(), services);
    app.ui.in_window_menu = false;
    app.ui.dialog = Some("stylesmanager".into());
    app
}

fn picking(path: Option<String>) -> (CadApp, Asked) {
    let asked: Asked = Rc::default();
    let log = asked.clone();
    let services = Services {
        pick_open_kind: Some(Box::new(move |kind: &FileKind| {
            log.borrow_mut().push(*kind);
            path.clone()
        })),
        ..Services::default()
    };
    (app(services), asked)
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
fn import_asks_for_a_plot_style_table_and_loads_it() {
    let black = |_| Some(PlotStyle { color: Some(cadcraft_engine::doc::color::Rgb(0, 0, 0)), ..PlotStyle::default() });
    let table = PlotStyleTable::color_dependent("office-pens.ctb", "Imported in a test", black);
    let path = std::env::temp_dir().join(format!("cadcraft-pstyle-import-{}.json", std::process::id()));
    std::fs::write(&path, table.to_file_text()).unwrap();
    let (app, asked) = picking(Some(path.to_string_lossy().into_owned()));
    let mut h = harness(app);
    h.get_by_label("Import .ctb/.stb…").click();
    h.run_steps(4);
    let _ = std::fs::remove_file(&path);
    assert_eq!(asked.borrow().iter().map(|k| k.extensions).collect::<Vec<_>>(), [PLOT_STYLE_FILE.extensions]);
    let app = h.state();
    assert!(app.session.doc().unwrap().plot_style_tables.iter().any(|t| t.name == "office-pens.ctb"));
    assert!(app.session.log.iter().any(|l| l == "Plot style table office-pens.ctb imported"), "{:?}", app.session.log);
    h.get_by_label("Imported in a test");
}

#[test]
fn a_cancelled_or_failed_import_changes_nothing() {
    let (app, asked) = picking(None);
    let mut h = harness(app);
    h.get_by_label("Import .ctb/.stb…").click();
    h.run_steps(3);
    assert_eq!(asked.borrow().len(), 1);
    assert!(h.state().session.doc().unwrap().plot_style_tables.is_empty());
    // A web upload that isn't a table reports why.
    cadcraft_engine::cmd::file::set_plot_style_reader(|name, bytes| {
        cadcraft_io::plot_style_file::read_plot_style_file(name, bytes).map(|r| (r.table, r.unsupported_linetypes))
    });
    let mut app = h.into_state();
    let r = app.import_plot_styles(json!({ "action": "load", "name": "bad.ctb", "data": "UElBRklMRVZFUlNJT05fMi4w" }));
    assert!(r.is_none());
    assert!(app.session.doc().unwrap().plot_style_tables.is_empty());
    // On the command line once, and on the status bar.
    let reason = "not an AutoCAD plot style table";
    assert_eq!(app.session.log.iter().filter(|l| l.contains(reason)).count(), 1, "{:?}", app.session.log);
    assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains(reason)), "{:?}", app.status);
}

#[test]
fn hosts_without_a_picker_show_no_import_button() {
    let h = harness(app(Services::default()));
    h.get_by_label("Plot Style Manager");
    assert!(h.query_by_label("Import .ctb/.stb…").is_none());
}

#[test]
fn the_import_button_reads_in_vietnamese() {
    let (mut app, _) = picking(None);
    app.run("ui.language", json!({ "lang": "vi" })).unwrap();
    let h = harness(app);
    h.get_by_label("Nhập .ctb/.stb…");
    assert_eq!(cadcraft_ui_egui::i18n::tr("vi", PLOT_STYLE_FILE.label), "Bảng kiểu in (CTB, STB)");
}
