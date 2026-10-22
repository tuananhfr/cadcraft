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
    let dirty = app.session.state().unwrap().is_dirty();
    let drawing = app.session.state().unwrap().doc.clone();
    app.run("ui.workspace.set", json!({"pointer": pointer, "value": value})).unwrap();
    assert_eq!(app.session.state().unwrap().is_dirty(), dirty, "layout set {pointer}");
    assert!(std::sync::Arc::ptr_eq(&drawing, &app.session.state().unwrap().doc), "layout set {pointer} keeps drawing identity");
}

fn restarted(app: &mut CadApp) -> CadApp {
    let dirty = app.session.state().unwrap().is_dirty();
    let drawing = app.session.state().unwrap().doc.clone();
    app.commit_workspace_now().unwrap();
    assert_eq!(app.session.state().unwrap().is_dirty(), dirty, "workspace commit preserves drawing dirty state");
    assert!(std::sync::Arc::ptr_eq(&drawing, &app.session.state().unwrap().doc));
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    let mut next = attached(MemoryStore { current: old.read_current(), snapshot_map, ..MemoryStore::default() });
    next.load_profile(&app.profile_json());
    next
}

fn harness(app: CadApp, width: f32, height: f32, ppp: f32) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(width, height) / ppp).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).build_ui_state(
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
        let dirty = app.session.state().unwrap().is_dirty();
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
        assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
    }
}

#[test]
fn status_bar_items_persist() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("options", json!({"ORTHOMODE": 1})).unwrap();
    let dirty = app.session.state().unwrap().is_dirty();
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
    assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
    let restored = restarted(h.state_mut());
    let dirty = restored.session.state().unwrap().is_dirty();
    assert_eq!(restored.ws.effective.layout.status_bar.items.get("ortho"), Some(&false));
    let h = harness(restored, 1366.0, 768.0, 1.0);
    assert!(h.query_by_label("Ortho Mode (F8)").is_none());
    assert!(h.state().session.settings.orthomode, "ORTHOMODE still restores through PROFILE_VARS");
    assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
}

#[test]
fn controls_reachable_ux08() {
    let _guard = test_lock();
    for preset in ["SIMPLE", "AUTOCAD_FAMILIAR"] {
        for (width, height) in [(1366.0, 768.0), (3840.0, 2160.0)] {
            for ppp in [1.0, 1.5, 2.0] {
                for scale in [1.0, 2.0] {
                    // Below a 640x360 pt working area (limitations.md) the chrome alone cannot leave a usable canvas.
                    if width / (ppp * scale) < 640.0 || height / (ppp * scale) < 360.0 {
                        continue;
                    }
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
    assert!(!h.state().session.state().unwrap().is_dirty(), "floating layout changes preserve the clean drawing");
    let restored = restarted(h.state_mut());
    let dirty = restored.session.state().unwrap().is_dirty();
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
    assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
}

#[test]
fn interface_scale_tracks_egui_zoom_without_committing_each_frame() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("options", json!({"uiScale": 1.5})).unwrap();
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    assert_eq!(h.ctx.zoom_factor(), 1.5);
    h.state_mut().commit_workspace_now().unwrap();
    let revision = h.state().ws.user.revision;
    h.key_press_modifiers(Modifiers::COMMAND, Key::Equals);
    h.run_steps(3);
    assert_eq!(h.state().ws.device.ui_scale, 1.6);
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
    assert_eq!(restored.ws.device.ui_scale, 1.6);
    let h = harness(restored, 1366.0, 768.0, 1.0);
    assert_eq!(h.ctx.zoom_factor(), 1.6);
}

#[test]
fn floating_command_line_grip_moves_and_clamps() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    let undo = h.state().ws.history.undo_len();
    let saved_before = h.state().ws.user.layout.command_line.clone();
    let from = h.get_by_label("Command line grip").rect().center();
    let before = h.get_by_label("Command line input").rect();
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for delta in [vec2(-10.0, -15.0), vec2(-20.0, -30.0), vec2(-30.0, -45.0), vec2(-40.0, -60.0)] {
        h.hover_at(from + delta);
        h.run_steps(2);
        assert_eq!(h.state().ws.user.layout.command_line, saved_before, "drag preview stays local");
        assert_eq!(h.state().ws.history.undo_len(), undo);
    }
    h.drop_at(from + vec2(-40.0, -60.0));
    h.run_steps(3);
    assert_eq!(h.state().ws.history.undo_len(), undo + 1, "one grip drag adds one undo entry");
    let moved = h.get_by_label("Command line input").rect();
    assert!(moved.top() < before.top());
    inside(h.state().canvas.rect.unwrap(), moved, "moved command line");
    set(h.state_mut(), "/command_line/x_ratio", json!(1.0));
    set(h.state_mut(), "/command_line/y_ratio", json!(1.0));
    h.run_steps(3);
    inside(h.state().canvas.rect.unwrap(), h.get_by_label("Command line input").rect(), "clamped command line");
    assert!(!h.state().session.state().unwrap().is_dirty());
    let saved = h.state().ws.effective.layout.command_line.clone();
    let restored = restarted(h.state_mut());
    assert_eq!(restored.ws.effective.layout.command_line, saved);
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
        let undo = h.state().ws.history.undo_len();
        let saved_before = h.state().ws.user.layout.command_line.clone();
        let input = h.get_by_label("Command line input").rect();
        let from = egui::pos2(input.center().x, input.top() - 3.0 * 16.0 - 4.0);
        let direction = if dock == "top" { 1.0 } else { -1.0 };
        let to = from + vec2(0.0, direction * 32.0);
        h.hover_at(from);
        h.run_steps(2);
        h.drag_at(from);
        h.run_steps(2);
        for distance in [8.0, 16.0, 24.0, 32.0] {
            h.hover_at(from + vec2(0.0, direction * distance));
            h.run_steps(2);
            assert_eq!(h.state().ws.user.layout.command_line, saved_before, "resize preview stays local");
            assert_eq!(h.state().ws.history.undo_len(), undo);
        }
        h.drop_at(to);
        h.run_steps(3);
        assert_eq!(h.state().ws.history.undo_len(), undo + 1, "one resize adds one undo entry for {dock}");
        if dock == "top" {
            assert!(h.get_by_label("Command line input").rect().bottom() > input.bottom(), "the top dock grows toward the canvas");
        }
        assert_eq!(h.state().ui.history_lines, 5, "resizing {dock}");
        assert_eq!(h.state().ws.effective.layout.command_line.lines, 5);
        assert!(!h.state().session.state().unwrap().is_dirty());
        let restored = restarted(h.state_mut());
        assert_eq!(restored.ws.effective.layout.command_line.lines, 5);
    }
}

#[test]
fn docked_command_line_autocomplete_is_visible_and_clickable() {
    let _guard = test_lock();
    for dock in ["bottom", "top"] {
        let mut app = attached(MemoryStore::default());
        set(&mut app, "/command_line/dock", json!(dock));
        let mut h = harness(app, 1366.0, 768.0, 1.0);
        h.event(egui::Event::Text("li".into()));
        h.run_steps(4);
        let suggestion = h.get_by_role_and_label(egui::accesskit::Role::Button, "LINE");
        let rect = suggestion.rect();
        inside(h.ctx.content_rect(), rect, dock);
        let input = h.get_by_label("Command line input").rect();
        if dock == "top" {
            assert!(rect.top() >= input.bottom());
        } else {
            assert!(rect.bottom() <= input.top());
        }
        suggestion.click();
        h.run_steps(3);
        assert_eq!(h.state().session.running.as_ref().unwrap().id, "line");
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert!(!h.state().session.state().unwrap().is_dirty());
    }
}

#[test]
fn docked_history_window_is_a_visible_foreground_area() {
    let _guard = test_lock();
    for dock in ["bottom", "top"] {
        let mut app = attached(MemoryStore::default());
        set(&mut app, "/command_line/dock", json!(dock));
        for line in 0..40 {
            app.session.echo(format!("History {line}"));
        }
        let mut h = harness(app, 1366.0, 768.0, 1.0);
        h.key_press(Key::F2);
        h.run_steps(4);
        let area = egui::containers::AreaState::load(&h.ctx, Id::new("cmdline_history_window")).unwrap();
        inside(h.ctx.content_rect(), area.rect(), "F2 history");
        let input = h.get_by_label("Command line input").rect();
        if dock == "top" {
            assert!(area.rect().top() >= input.bottom());
        } else {
            assert!(area.rect().bottom() <= input.top());
        }
        assert!(h.state().cmd.expanded);
        h.key_press(Key::F2);
        h.run_steps(3);
        assert!(!h.state().cmd.expanded);
    }
}

#[test]
fn live_resize_remaps_floating_into_view() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app, 3840.0, 2160.0, 1.0);
    h.state_mut().run("ui.dock", json!({"operation": "float", "panel": "properties", "rect": [3600.0, 1950.0, 500.0, 350.0]})).unwrap();
    h.state_mut().run("ui.toolbar.show", json!({"id": "draw"})).unwrap();
    h.state_mut().run("ui.toolbar.float", json!({"id": "draw", "x_ratio": 0.97, "y_ratio": 0.97})).unwrap();
    h.run_steps(4);
    let dirty = h.state().session.state().unwrap().is_dirty();
    let old_area = h.state().ws.area;
    h.set_size(vec2(1366.0, 768.0));
    h.run_steps(6);
    let screen = h.ctx.content_rect();
    assert!(screen.width() < old_area.width());
    assert_eq!(h.state().ws.area, screen);
    let group =
        h.state().ui.docking.layout.floating.iter().find(|group| group.panels.contains(&cadcraft_ui_egui::docking::Panel::Properties)).unwrap();
    let [x, y, width, height] = group.rect;
    let panel = Rect::from_min_size(egui::pos2(x, y), vec2(width, height));
    inside(screen, panel, "live remapped panel");
    assert!(width >= 24.0 && height >= 24.0);
    let toolbar = h.state().ws.effective.layout.toolbars.iter().find(|bar| bar.id == "draw").unwrap();
    let rect = toolbars::render::floating_rect(toolbar, screen);
    inside(screen, rect, "live remapped toolbar");
    assert!(rect.width() >= 24.0 && rect.height() >= 24.0);
    inside(screen, h.get_by_label("Draw toolbar grip").rect(), "live remapped toolbar grip");
    assert_eq!(h.state().session.state().unwrap().is_dirty(), dirty);
}

#[test]
fn interface_scale_drag_commits_only_on_release() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.ui.dialog = Some("options:display".into());
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    h.state_mut().commit_workspace_now().unwrap();
    let revision = h.state().ws.user.revision;
    let slider = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Interface scale").rect();
    let from = egui::pos2(slider.left() + 10.0, slider.center().y);
    let to = egui::pos2(slider.right() - 10.0, slider.center().y);
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for fraction in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + (to - from) * fraction);
        h.run_steps(2);
        assert_eq!(h.ctx.zoom_factor(), 1.0, "dragging does not move the interface under the pointer");
        assert_eq!(h.state().ws.device.ui_scale, 1.0);
        assert_eq!(h.state().ws.user.revision, revision);
    }
    h.drop_at(to);
    h.run_steps(4);
    assert!(h.ctx.zoom_factor() > 1.0);
    assert_eq!(h.ctx.zoom_factor(), h.state().ws.device.ui_scale);
    let committed = h.ctx.zoom_factor();
    h.run_steps(4);
    assert_eq!(h.ctx.zoom_factor(), committed);
    assert!(!h.state().session.state().unwrap().is_dirty());
    h.run_steps(60);
    assert_eq!(h.state().ws.user.revision, revision + 1, "one slider gesture produces one saved change");
}

#[test]
fn options_ui_scale_uses_camel_case_accepts_legacy_input_and_resets() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let result = app.run("options", json!({"uiScale": 1.5})).unwrap();
    assert_eq!(result.get("uiScale"), Some(&json!(1.5)));
    assert!(result.get("ui_scale").is_none());
    let result = app.run("options", json!({"ui_scale": 1.75})).unwrap();
    assert_eq!(result.get("uiScale"), Some(&json!(1.75)));
    let result = app.run("options", json!({"reset": true})).unwrap();
    assert_eq!(result.get("uiScale"), Some(&json!(1.0)));
    assert_eq!(app.ws.device.ui_scale, 1.0);
    let result = app.run("options", json!({"reset": true, "uiScale": 1.25})).unwrap();
    assert_eq!(result.get("uiScale"), Some(&json!(1.25)));
}

#[test]
fn docked_default_height_keeps_all_history_lines() {
    let _guard = test_lock();
    for (dock, panel) in [("top", "cc_cmdline_top"), ("bottom", "cc_cmdline_bottom")] {
        let mut app = attached(MemoryStore::default());
        set(&mut app, "/command_line/dock", json!(dock));
        for line in 0..3 {
            app.session.echo(format!("History {line}"));
        }
        let h = harness(app, 1366.0, 768.0, 1.0);
        let row = egui::containers::panel::PanelState::load(&h.ctx, Id::new(panel)).unwrap().outer_rect;
        let input = h.get_by_label("Command line input").rect();
        assert!(input.top() - row.top() >= 3.0 * 16.0 + 8.0, "all three history lines fit with the input row");
        assert_eq!(h.state().ui.history_lines, 3);
        assert!(row.height() >= 3.0 * 16.0 + 24.0);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Command line input").is_none());
    }
}

#[test]
fn empty_status_bar_only_opens_visibility_menu_on_secondary_click() {
    let _guard = test_lock();
    let app = attached(MemoryStore::default());
    let mut h = harness(app, 1366.0, 768.0, 1.0);
    let bar = h.get_by_label("Status bar").rect();
    let empty = egui::pos2(bar.left() + 2.0, bar.center().y);
    h.hover_at(empty);
    h.run_steps(2);
    h.drag_at(empty);
    h.run_steps(2);
    h.drop_at(empty);
    h.run_steps(3);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Ortho Mode (F8)").is_none());
    h.event(egui::Event::PointerMoved(empty));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: empty, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Ortho Mode (F8)").is_some());
}

#[test]
fn compact_ribbon_double_click_keeps_saved_collapsed_preference() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let mut h = harness(app, 900.0, 420.0, 1.0);
    let row = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
    assert!(row.height() < 40.0, "the ribbon renders compactly");
    assert!(!h.state().ws.effective.layout.ribbon.collapsed);
    h.get_by_label("Home").click();
    h.run_steps(1);
    h.get_by_label("Home").click();
    h.run_steps(3);
    assert!(!h.state().ws.effective.layout.ribbon.collapsed);
    assert!(!restarted(h.state_mut()).ws.effective.layout.ribbon.collapsed);
}
