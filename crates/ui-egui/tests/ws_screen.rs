//! Workspace persistence and control reachability across window sizes and interface zoom.

use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::toolbars;
use cadcraft_ui_egui::wsprofile::schema::Dock;
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Id, Key, Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

static LOCK: Mutex<()> = Mutex::new(());

fn test_lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app.run("ui.toggle.menubar", json!({"on": true})).unwrap();
    app
}

fn set(app: &mut CadApp, pointer: &str, value: Value) {
    app.run("ui.workspace.set", json!({"pointer": pointer, "value": value})).unwrap();
}

fn restarted(app: &mut CadApp) -> CadApp {
    app.commit_workspace_now().unwrap();
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    let mut next = attached(MemoryStore { current: old.read_current(), snapshot_map, ..MemoryStore::default() });
    next.load_profile(&app.profile_json());
    next
}

fn harness(app: CadApp, width: f32, height: f32, ppp: f32) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(width, height)).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(6);
    h
}

fn inside(screen: Rect, rect: Rect, label: &str) {
    assert!(rect.is_positive() && screen.expand(0.5).contains_rect(rect), "{label}: {rect:?} outside {screen:?}");
}

#[test]
fn command_line_dock_float_height() {
    let _guard = test_lock();
    for (name, dock, panel) in
        [("top", Dock::Top, Some("cc_cmdline_top")), ("bottom", Dock::Bottom, Some("cc_cmdline_bottom")), ("floating", Dock::Floating, None)]
    {
        let mut app = attached(MemoryStore::default());
        set(&mut app, "/command_line/dock", json!(name));
        set(&mut app, "/command_line/height_ratio", json!(0.15));
        let app = restarted(&mut app);
        assert_eq!(app.ws.effective.layout.command_line.dock, dock);
        assert_eq!(app.ws.effective.layout.command_line.height_ratio, 0.15);
        let mut h = harness(app, 1366.0, 768.0, 1.0);
        let input = h.get_by_label("Command line input").rect();
        inside(h.ctx.content_rect(), input, name);
        if let Some(panel) = panel {
            let row = egui::containers::panel::PanelState::load(&h.ctx, Id::new(panel)).unwrap().outer_rect;
            assert!((row.height() - 768.0 * 0.15).abs() <= 1.0, "{name}: {row:?}");
            let canvas = h.state().canvas.rect.unwrap();
            assert!(!row.intersect(canvas).is_positive(), "docked command line reserves its own row");
        } else {
            let canvas = h.state().canvas.rect.unwrap();
            let width = (canvas.width() * 0.48).clamp(360.0, 760.0).min((canvas.width() - 8.0).max(0.0));
            assert!((input.width() - width).abs() <= 1.0);
            assert!((input.center().x - canvas.center().x).abs() <= 1.0);
            assert!((canvas.bottom() - input.bottom() - 10.0).abs() <= 1.0);
        }
        h.state_mut().start("line");
        h.run_steps(2);
        assert_eq!(h.state().session.running.as_ref().unwrap().id, "line");
        h.get_by_label("Command line input").click();
        h.run_steps(2);
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert!(h.state().session.running.is_none(), "Escape cancels with dock {name}");
        assert!(!h.state().session.state().unwrap().is_dirty());
    }
}

#[test]
fn status_bar_items_persist() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("options", json!({"ORTHOMODE": 1})).unwrap();
    assert!(cadcraft_engine::cmd::options::PROFILE_VARS.contains(&"ORTHOMODE"));
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    assert!(h.query_by_label("Ortho Mode (F8)").is_some());
    h.get_by_label("Status bar").click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Ortho Mode (F8)").click();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Ortho Mode (F8)").is_none());
    assert!(h.state().session.settings.orthomode);
    let restored = restarted(h.state_mut());
    assert_eq!(restored.ws.effective.layout.status_bar.items.get("ortho"), Some(&false));
    let h = harness(restored, 1366.0, 768.0, 1.0);
    assert!(h.query_by_label("Ortho Mode (F8)").is_none());
    assert!(h.state().session.settings.orthomode, "ORTHOMODE still restores through PROFILE_VARS");
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn controls_reachable_ux08() {
    let _guard = test_lock();
    for preset in ["SIMPLE", "AUTOCAD_FAMILIAR"] {
        for (width, height) in [(1366.0, 768.0), (3840.0, 2160.0)] {
            for ppp in [1.0, 1.5, 2.0] {
                for scale in [1.0, 2.0] {
                    let mut app = attached(MemoryStore::default());
                    app.run("ui.workspace.select", json!({"id": preset})).unwrap();
                    app.run("ui.toggle.menubar", json!({"on": true})).unwrap();
                    set(&mut app, "/ui_scale", json!(scale));
                    let h = harness(app, width, height, ppp);
                    let screen = h.ctx.content_rect();
                    let case = format!("{preset} {width}x{height} ppp={ppp} scale={scale}");
                    assert_eq!(h.ctx.zoom_factor(), scale, "{case}");
                    for label in ["File", "Command line input", "Status bar"] {
                        inside(screen, h.get_by_label(label).rect(), &format!("{case}: {label}"));
                    }
                    let menu = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_menubar")).unwrap().outer_rect;
                    inside(screen, menu, &format!("{case}: menu bar"));
                    if preset == "AUTOCAD_FAMILIAR" {
                        inside(screen, h.get_by_label("Home").rect(), &format!("{case}: first ribbon tab"));
                        inside(screen, h.get_by_label("Open (Ctrl+O)").rect(), &format!("{case}: Quick Access command"));
                        inside(screen, h.get_by_label("Search commands...").rect(), &format!("{case}: Quick Access"));
                        assert!(!h.state().ws.effective.layout.ribbon.collapsed, "compact rendering preserves the profile preference");
                    } else {
                        assert!(h.state().ws.effective.layout.quick_access.is_empty());
                        assert!(!h.state().ws.effective.layout.ribbon.visible);
                    }
                    let canvas = h.state().canvas.rect.unwrap();
                    inside(screen, canvas, &format!("{case}: canvas"));
                    assert!(canvas.width() >= 200.0 && canvas.height() >= 150.0, "{case}: canvas {canvas:?}");
                }
            }
        }
    }
}

#[test]
fn screen_shrink_pulls_floating_back_ux07() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app, 3840.0, 2160.0, 1.0);
    h.state_mut().run("ui.dock", json!({"operation": "float", "panel": "properties", "rect": [3600.0, 1950.0, 500.0, 350.0]})).unwrap();
    h.state_mut().run("ui.toolbar.show", json!({"id": "draw"})).unwrap();
    h.state_mut().run("ui.toolbar.float", json!({"id": "draw", "x_ratio": 0.97, "y_ratio": 0.97})).unwrap();
    h.run_steps(4);
    let restored = restarted(h.state_mut());
    let h = harness(restored, 1366.0, 768.0, 1.0);
    let screen = h.ctx.content_rect();
    let group =
        h.state().ui.docking.layout.floating.iter().find(|group| group.panels.contains(&cadcraft_ui_egui::docking::Panel::Properties)).unwrap();
    let [x, y, width, height] = group.rect;
    let panel = Rect::from_min_size(egui::pos2(x, y), vec2(width, height));
    inside(screen, panel, "floating panel after restart");
    assert!(panel.width() >= 24.0 && panel.height() >= 24.0);
    let toolbar = h.state().ws.effective.layout.toolbars.iter().find(|bar| bar.id == "draw").unwrap();
    let toolbar_rect = toolbars::render::floating_rect(toolbar, screen);
    inside(screen, toolbar_rect, "floating toolbar after restart");
    assert!(toolbar_rect.width() >= 24.0 && toolbar_rect.height() >= 24.0);
    inside(screen, h.get_by_label("Draw toolbar grip").rect(), "floating toolbar grip after restart");
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn interface_scale_tracks_egui_zoom_without_committing_each_frame() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("options", json!({"ui_scale": 1.5})).unwrap();
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    assert_eq!(h.ctx.zoom_factor(), 1.5);
    h.state_mut().commit_workspace_now().unwrap();
    let revision = h.state().ws.user.revision;
    h.key_press_modifiers(Modifiers::COMMAND, Key::Equals);
    h.run_steps(3);
    assert_eq!(h.state().ws.user.ui_scale, 1.6);
    assert_eq!(h.state().ws.user.revision, revision, "zoom uses the normal debounce");
    assert!(h.state().ws.autosave.pending());
    h.run_steps(3);
    assert_eq!(h.state().ws.user.revision, revision);
    assert!(!h.state().session.state().unwrap().is_dirty());
    h.run_steps(60);
    assert_eq!(h.state().ws.user.revision, revision + 1, "zoom saves after the quiet spell");
    assert!(!h.state().ws.autosave.pending());
    h.run_steps(60);
    assert_eq!(h.state().ws.user.revision, revision + 1, "unchanged frames do not save again");
    let restored = restarted(h.state_mut());
    assert_eq!(restored.ws.user.ui_scale, 1.6);
    let h = harness(restored, 1366.0, 768.0, 1.0);
    assert_eq!(h.ctx.zoom_factor(), 1.6);
}

#[test]
fn floating_command_line_grip_moves_and_clamps() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    let from = h.get_by_label("Command line grip").rect().center();
    let before = h.get_by_label("Command line input").rect();
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    h.hover_at(from + vec2(-40.0, -60.0));
    h.run_steps(3);
    h.drop_at(from + vec2(-40.0, -60.0));
    h.run_steps(3);
    let moved = h.get_by_label("Command line input").rect();
    assert!(moved.top() < before.top());
    inside(h.state().canvas.rect.unwrap(), moved, "moved command line");
    set(h.state_mut(), "/command_line/x_ratio", json!(1.0));
    set(h.state_mut(), "/command_line/y_ratio", json!(1.0));
    h.run_steps(3);
    inside(h.state().canvas.rect.unwrap(), h.get_by_label("Command line input").rect(), "clamped command line");
    let saved = h.state().ws.effective.layout.command_line.clone();
    let restored = restarted(h.state_mut());
    assert_eq!(restored.ws.effective.layout.command_line, saved);
    assert!(!restored.session.state().unwrap().is_dirty());
}

#[test]
fn status_bar_schema_ids_control_existing_widgets() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    for (id, label) in [
        ("annotation", "Annotation Visibility"),
        ("dynmode", "Dynamic Input (F12)"),
        ("grid", "Grid (F7)"),
        ("isodraft", "Isometric Drafting"),
        ("layouts", "Model"),
        ("lineweight", "Show/Hide Lineweight"),
        ("ortho", "Ortho Mode (F8)"),
        ("osnap", "Object Snap (F3)"),
        ("otrack", "Object Snap Tracking (F11)"),
        ("polar", "Polar Tracking (F10)"),
        ("quickprops", "Quick Properties"),
        ("settings", "Drafting Settings"),
        ("snap", "Snap Mode (F9)"),
        ("transparency", "Show/Hide Transparency"),
        ("workspace", "Workspace Switching"),
    ] {
        assert!(h.query_by_label(label).is_some(), "existing status item {id}");
        set(h.state_mut(), &format!("/status_bar/items/{id}"), json!(false));
        h.run_steps(3);
        assert!(h.query_by_label(label).is_none(), "hidden status item {id}");
    }
    set(h.state_mut(), "/status_bar/items/future_control", json!(true));
    h.run_steps(3);
    assert!(h.query_by_label("future_control").is_none());
    assert_eq!(restarted(h.state_mut()).ws.effective.layout.status_bar.items.get("future_control"), Some(&true));
}

#[test]
fn command_line_top_edge_resizes_lines_in_all_docks() {
    let _guard = test_lock();
    for dock in ["top", "bottom", "floating"] {
        let mut app = attached(MemoryStore::default());
        set(&mut app, "/command_line/dock", json!(dock));
        set(&mut app, "/command_line/height_ratio", json!(0.15));
        for line in 0..20 {
            app.session.echo(format!("History {line}"));
        }
        let mut h = harness(app, 1366.0, 768.0, 1.0);
        let input = h.get_by_label("Command line input").rect();
        let from = egui::pos2(input.center().x, input.top() - 3.0 * 16.0 - 4.0);
        let to = from - vec2(0.0, 32.0);
        h.hover_at(from);
        h.run_steps(2);
        h.drag_at(from);
        h.run_steps(2);
        h.hover_at(to);
        h.run_steps(3);
        h.drop_at(to);
        h.run_steps(3);
        assert_eq!(h.state().ui.history_lines, 5, "resizing {dock}");
        assert_eq!(h.state().ws.effective.layout.command_line.lines, 5);
        let restored = restarted(h.state_mut());
        assert_eq!(restored.ws.effective.layout.command_line.lines, 5);
        assert!(!restored.session.state().unwrap().is_dirty());
    }
}
