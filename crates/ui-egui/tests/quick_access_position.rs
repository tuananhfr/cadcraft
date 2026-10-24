use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::quick_access::{self, QuickAccessRow, quick_access_row};
use cadcraft_ui_egui::wsprofile::LOCKED_MESSAGE;
use cadcraft_ui_egui::wsprofile::schema::{Density, QuickAccessPosition, WorkspaceLayout};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Id, Key, Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store));
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app
}

fn restarted(app: &mut CadApp) -> CadApp {
    app.commit_workspace_now().unwrap();
    let old = app.ws.store.as_deref().unwrap();
    let snapshot_map = old.snapshots().into_iter().filter_map(|revision| old.read_snapshot(revision).map(|text| (revision, text))).collect();
    attached(MemoryStore { current: old.read_current(), policy: old.read_policy(), snapshot_map, ..MemoryStore::default() })
}

fn harness(app: CadApp, whole_app: bool) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        move |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            if whole_app {
                app.ui(ui);
            } else {
                ui.horizontal(|ui| quick_access::show(app, ui));
            }
        },
        app,
    );
    h.run_steps(5);
    h
}

fn pinned() -> CadApp {
    let mut app = attached(MemoryStore::default());
    for id in ["line", "circle", "ubplot"] {
        app.run("ui.quickaccess.pin", json!({"id": id})).unwrap();
    }
    app
}

fn qat_rect(h: &Harness<'_, CadApp>) -> Rect {
    h.query_all_by_label("Quick Plot...").next().unwrap().rect()
}

#[test]
fn qat_pins_from_toolbar_and_tool_sets() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.toolbar.hide", json!({"id": "standard"})).unwrap();
    app.run("ui.toolbar.show", json!({"id": "draw"})).unwrap();
    app.run("ui.workspace.set", json!({"pointer": "/toolbars/draw/commands", "value": ["circle"]})).unwrap();
    let mut h = harness(app, true);
    h.query_all_by_label("Circle").next().unwrap().click_secondary();
    h.run_steps(5);
    h.get_by_label("Add to Quick Access").click();
    h.run_steps(5);
    assert_eq!(h.state().ws.effective.layout.quick_access, ["circle"]);
    h.query_all_by_label("Circle").next().unwrap().click_secondary();
    h.run_steps(5);
    h.get_by_label("Remove from Quick Access").click();
    h.run_steps(5);
    assert!(h.state().ws.effective.layout.quick_access.is_empty());
    h.query_all_by_label("Line").next().unwrap().click_secondary();
    h.run_steps(5);
    h.get_by_label("Add to Quick Access").click();
    h.run_steps(5);
    assert_eq!(h.state().ws.effective.layout.quick_access, ["line"]);
    h.query_all_by_label("Line").nth(1).unwrap().click_secondary();
    h.run_steps(5);
    h.get_by_label("Remove from Quick Access").click();
    h.run_steps(5);
    assert!(h.state().ws.effective.layout.quick_access.is_empty());
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn qat_reorders_by_drag_in_the_bar() {
    let _guard = test_lock();
    let mut h = harness(pinned(), false);
    let source = qat_rect(&h).center();
    let target = h.query_all_by_label("Line").next().unwrap().rect().center();
    let steps = h.state().ws.history.undo_len();
    h.hover_at(source);
    h.run_steps(2);
    h.drag_at(source);
    h.run_steps(2);
    h.hover_at(source + (target - source) * 0.5);
    h.run_steps(2);
    h.hover_at(target);
    h.run_steps(3);
    assert!(egui::DragAndDrop::has_any_payload(&h.ctx));
    assert_eq!(h.state().ws.effective.layout.quick_access, ["line", "circle", "ubplot"]);
    assert_eq!(h.state().ws.history.undo_len(), steps);
    h.drop_at(target);
    h.run_steps(5);
    assert_eq!(h.state().ws.effective.layout.quick_access, ["ubplot", "line", "circle"]);
    assert_eq!(h.state().ws.history.undo_len(), steps + 1);
    let plot = qat_rect(&h);
    let line = h.query_all_by_label("Line").next().unwrap().rect();
    let circle = h.query_all_by_label("Circle").next().unwrap().rect();
    assert!(plot.right() <= line.left() && line.right() <= circle.left());
    let restored = restarted(h.state_mut());
    assert_eq!(restored.ws.effective.layout.quick_access, ["ubplot", "line", "circle"]);
    assert!(!restored.session.state().unwrap().is_dirty());
}

#[test]
fn qat_position_is_independent_of_menu_bar() {
    let _guard = test_lock();
    for integrated in [false, true] {
        for in_window_menu in [false, true] {
            for (position, expected) in [
                ("title_bar", QuickAccessPosition::TitleBar),
                ("above_ribbon", QuickAccessPosition::AboveRibbon),
                ("below_ribbon", QuickAccessPosition::BelowRibbon),
            ] {
                let mut app = attached(MemoryStore::default());
                app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
                app.run("ui.workspace.set", json!({"pointer": "/quick_access", "value": ["ubplot"]})).unwrap();
                app.run("ui.ribbon.mode", json!({"mode": "tabs_only"})).unwrap();
                app.run("ui.quickaccess.position", json!({"position": position})).unwrap();
                app.run("ui.toggle.menubar", json!({"on": in_window_menu})).unwrap();
                app.integrated_titlebar = integrated;
                let mut h = harness(app, true);
                let qat = qat_rect(&h);
                assert!(h.ctx.content_rect().contains_rect(qat));
                let ribbon = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
                if expected == QuickAccessPosition::BelowRibbon {
                    assert_eq!(quick_access_row(h.state()), QuickAccessRow::OwnRowBelowRibbon);
                    assert!(qat.min.y > ribbon.max.y, "{qat:?} below {ribbon:?}");
                } else {
                    let row = if expected == QuickAccessPosition::TitleBar { QuickAccessRow::TitleBar } else { QuickAccessRow::OwnRowAboveRibbon };
                    assert_eq!(quick_access_row(h.state()), row);
                    assert!(qat.max.y < ribbon.min.y, "{qat:?} above {ribbon:?}");
                }
                let mut restored = restarted(h.state_mut());
                assert_eq!(restored.ws.effective.layout.quick_access_position, expected);
                assert_eq!(restored.ui.in_window_menu, in_window_menu);
                restored.integrated_titlebar = integrated;
                let h = harness(restored, true);
                assert!(h.ctx.content_rect().contains_rect(qat_rect(&h)));
                assert!(!h.state().session.state().unwrap().is_dirty());
            }
        }
    }
}

#[test]
fn focus_mode_hides_and_restores_every_qat_position() {
    let _guard = test_lock();
    for in_window_menu in [false, true] {
        for position in ["title_bar", "above_ribbon", "below_ribbon"] {
            let mut app = attached(MemoryStore::default());
            app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
            app.run("ui.workspace.set", json!({"pointer": "/quick_access", "value": ["ubplot"]})).unwrap();
            app.run("ui.ribbon.mode", json!({"mode": "tabs_only"})).unwrap();
            app.run("ui.quickaccess.position", json!({"position": position})).unwrap();
            app.run("ui.toggle.menubar", json!({"on": in_window_menu})).unwrap();
            app.run("ui.density", json!({"density": "comfortable"})).unwrap();
            let mut h = harness(app, true);
            let before = qat_rect(&h);
            let layout = serde_json::to_value(&h.state().ws.effective.layout).unwrap();
            h.state_mut().run("ui.focusmode", json!({"on": true})).unwrap();
            h.run_steps(4);
            assert!(h.query_all_by_label("Quick Plot...").next().is_none(), "{position}, menu={in_window_menu}");
            let canvas = h.state().canvas.rect.unwrap();
            assert!(canvas.height() >= h.ctx.content_rect().height() * 0.85);
            assert_eq!(serde_json::to_value(&h.state().ws.effective.layout).unwrap(), layout);
            h.state_mut().run("ui.focusmode", json!({"on": false})).unwrap();
            h.run_steps(4);
            assert_eq!(qat_rect(&h), before, "{position}, menu={in_window_menu}");
            assert_eq!(serde_json::to_value(&h.state().ws.effective.layout).unwrap(), layout);
        }
    }
}

#[test]
fn qat_survives_theme_and_density_changes_tc18() {
    let _guard = test_lock();
    let mut h = harness(pinned(), true);
    let before = h.state().ws.effective.layout.quick_access.clone();
    for density in ["normal", "comfortable"] {
        h.state_mut().run("ui.density", json!({"density": density})).unwrap();
        for theme in ["light", "dark", "light"] {
            h.state_mut().run("ui.theme", json!({"theme": theme})).unwrap();
            h.run_steps(5);
            assert_eq!(h.state().ws.effective.layout.quick_access, before);
            assert_eq!(h.state().shown_theme(), if theme == "light" { egui::Theme::Light } else { egui::Theme::Dark });
            let line = h.query_all_by_label("Line").next().unwrap().rect();
            let circle = h.query_all_by_label("Circle").next().unwrap().rect();
            let plot = qat_rect(&h);
            assert!(line.right() <= circle.left() && circle.right() <= plot.left());
            if density == "comfortable" {
                assert_eq!(h.state().ws.effective.layout.density, Density::Comfortable);
                for rect in [line, circle, plot] {
                    assert!(rect.width() >= 32.0 && rect.height() >= 32.0);
                    assert!(h.ctx.content_rect().contains_rect(rect));
                }
            }
        }
    }
}

#[test]
fn qat_alt_arrows_keep_focus_and_match_drag_order() {
    let _guard = test_lock();
    let mut h = harness(pinned(), false);
    h.query_all_by_label("Quick Plot...").next().unwrap().focus();
    h.run_steps(3);
    for _ in 0..2 {
        h.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
        h.run_steps(5);
        assert!(h.query_all_by_label("Quick Plot...").next().unwrap().is_focused());
    }
    assert_eq!(h.state().ws.effective.layout.quick_access, ["ubplot", "line", "circle"]);
    let steps = h.state().ws.history.undo_len();
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
    h.run_steps(5);
    assert_eq!(h.state().ws.history.undo_len(), steps);
    assert!(h.query_all_by_label("Quick Plot...").next().unwrap().is_focused());
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    h.run_steps(5);
    assert_eq!(h.state().ws.effective.layout.quick_access, ["line", "ubplot", "circle"]);
    assert!(h.query_all_by_label("Quick Plot...").next().unwrap().is_focused());
}

#[test]
fn qat_context_menu_changes_position() {
    let _guard = test_lock();
    let mut h = harness(pinned(), true);
    for (label, expected) in
        [("Show Below the Ribbon", QuickAccessPosition::BelowRibbon), ("Show Above the Ribbon", QuickAccessPosition::AboveRibbon)]
    {
        h.query_all_by_label("Quick Plot...").next().unwrap().click_secondary();
        h.run_steps(5);
        h.get_by_label(label).click();
        h.run_steps(5);
        assert_eq!(h.state().ws.effective.layout.quick_access_position, expected);
    }
}

#[test]
fn qat_position_defaults_round_trips_and_rejects_invalid_or_locked_edits() {
    let _guard = test_lock();
    let legacy: WorkspaceLayout = serde_json::from_value(json!({"quick_access": ["line"]})).unwrap();
    assert_eq!(legacy.quick_access_position, QuickAccessPosition::TitleBar);
    assert_eq!(WorkspaceLayout::default().quick_access_position, QuickAccessPosition::TitleBar);
    assert!(cadcraft_ui_egui::menus::UI_COMMANDS.iter().any(|entry| entry.0 == "ui.quickaccess.position"));
    let mut app = pinned();
    for (position, expected) in [
        ("title_bar", QuickAccessPosition::TitleBar),
        ("above_ribbon", QuickAccessPosition::AboveRibbon),
        ("below_ribbon", QuickAccessPosition::BelowRibbon),
    ] {
        app.run("ui.quickaccess.position", json!({"position": position})).unwrap();
        assert_eq!(app.ws.effective.layout.quick_access_position, expected);
        let serialized = serde_json::to_value(&app.ws.user.layout).unwrap();
        assert_eq!(serialized["quick_access_position"], position);
        let restored: WorkspaceLayout = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored.quick_access_position, expected);
        let steps = app.ws.history.undo_len();
        assert_eq!(app.run("ui.quickaccess.position", json!({"position": position})).unwrap()["changed"], false);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    for params in [json!({}), json!({"position": null}), json!({"position": 2}), json!({"position": "sideways"})] {
        assert_eq!(app.run("ui.quickaccess.position", params).unwrap_err(), "position must be title_bar, above_ribbon or below_ribbon");
        assert_eq!(app.ws.user.layout, before);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/quick_access_position", "/quick_access"]}]});
    let mut locked = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    let before = locked.ws.user.layout.clone();
    for (command, params) in [("ui.quickaccess.position", json!({"position": "below_ribbon"})), ("ui.quickaccess.pin", json!({"id": "line"}))] {
        assert_eq!(locked.run(command, params).unwrap_err(), LOCKED_MESSAGE);
        assert_eq!(locked.ws.user.layout, before);
    }
}
