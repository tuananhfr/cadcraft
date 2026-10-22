use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::LOCKED_MESSAGE;
use cadcraft_ui_egui::wsprofile::schema::{Ribbon, RibbonMode};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use egui::{Id, Key, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

// App restarts share the engine's process-wide alias store.
fn test_lock() -> MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(store: MemoryStore) -> CadApp {
    let mut app = CadApp::new(Session::default(), Services::default());
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

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    scaled_harness(app, 1.0)
}

fn scaled_harness(app: CadApp, ppp: f32) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0) / ppp).with_pixels_per_point(ppp).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

#[test]
fn legacy_ribbon_flags_map_to_modes() {
    for (flags, expected) in [
        (json!({"visible": false}), RibbonMode::Hidden),
        (json!({"visible": false, "collapsed": true}), RibbonMode::Hidden),
        (json!({"visible": true, "collapsed": true}), RibbonMode::TabsOnly),
        (json!({"mode": null, "visible": true, "collapsed": true}), RibbonMode::TabsOnly),
        (json!({"visible": true, "collapsed": false}), RibbonMode::Expanded),
        (json!({"visible": true}), RibbonMode::Expanded),
        (json!({}), RibbonMode::Hidden),
    ] {
        let ribbon: Ribbon = serde_json::from_value(flags).unwrap();
        assert_eq!(ribbon.mode, None);
        assert_eq!(ribbon.effective_mode(), expected);
        assert!(serde_json::to_value(&ribbon).unwrap().get("mode").is_none());
    }
    for mode in [RibbonMode::Expanded, RibbonMode::Compact, RibbonMode::TabsOnly, RibbonMode::Hidden] {
        let ribbon = Ribbon { mode: Some(mode), visible: mode == RibbonMode::Hidden, collapsed: mode != RibbonMode::TabsOnly, ..Ribbon::default() };
        assert_eq!(ribbon.effective_mode(), mode, "explicit mode takes precedence over legacy flags");
    }
}

#[test]
fn set_mode_round_trips_and_keeps_legacy_flags() {
    let mut ribbon: Ribbon = serde_json::from_value(json!({"tabs": [{"id": "home", "label": "Home"}]})).unwrap();
    let tabs = ribbon.tabs.clone();
    for (mode, name, visible, collapsed) in [
        (RibbonMode::Expanded, "expanded", true, false),
        (RibbonMode::Compact, "compact", true, false),
        (RibbonMode::TabsOnly, "tabs_only", true, true),
        (RibbonMode::Hidden, "hidden", false, false),
    ] {
        ribbon.set_mode(mode);
        assert_eq!(ribbon.mode, Some(mode));
        assert_eq!(ribbon.visible, visible);
        assert_eq!(ribbon.collapsed, collapsed);
        assert_eq!(ribbon.tabs, tabs);
        let raw = serde_json::to_string(&ribbon).unwrap();
        let restored: Ribbon = serde_json::from_str(&raw).unwrap();
        assert_eq!(restored, ribbon);
        assert_eq!(restored.effective_mode(), mode);
        assert_eq!(serde_json::to_value(&restored).unwrap()["mode"], name);
    }
}

#[test]
fn ribbon_mode_command_persists_across_restart() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    for (command, expected_label, expected_path) in [
        ("ui.ribbon.mode", "Ribbon Display Mode", &[][..]),
        ("ui.ribbon.mode.expanded", "Expanded", &["View", "Ribbon", "Display Mode", "Expanded"][..]),
        ("ui.ribbon.mode.compact", "Compact", &["View", "Ribbon", "Display Mode", "Compact"][..]),
        ("ui.ribbon.mode.tabs_only", "Tabs Only", &["View", "Ribbon", "Display Mode", "Tabs Only"][..]),
        ("ui.ribbon.mode.hidden", "Hidden", &["View", "Ribbon", "Display Mode", "Hidden"][..]),
    ] {
        let rows: Vec<_> = cadcraft_ui_egui::menus::UI_COMMANDS.iter().filter(|(id, _, _, _)| *id == command).collect();
        assert_eq!(rows.len(), 1, "{command}");
        let (_, label, path, shortcut) = rows[0];
        assert_eq!(*label, expected_label, "{command}");
        assert_eq!(*path, expected_path, "{command}");
        assert!(shortcut.is_none(), "{command}");
    }
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    assert_eq!(app.run("ui.ribbon.mode", json!({"mode": "compact"})).unwrap()["changed"], true);
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    assert_eq!(app.ws.history.undo_len(), steps + 1);
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, before);
    app.run("ui.layout.redo", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    let steps = app.ws.history.undo_len();
    assert_eq!(app.run("ui.ribbon.mode", json!({"mode": "compact"})).unwrap()["changed"], false);
    assert_eq!(app.ws.history.undo_len(), steps);
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout.ribbon.mode, Some(RibbonMode::Compact));
    assert_eq!(restored.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    assert!(restored.ws.effective.layout.ribbon.visible);
    assert!(!restored.ws.effective.layout.ribbon.collapsed);
    assert!(!app.session.state().unwrap().is_dirty());
    let h = harness(restored);
    assert_eq!(h.state().ws.effective.layout.ribbon.effective_mode(), RibbonMode::Compact);
    assert!(h.query_by_label("Home").is_some());
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_mode_rejects_unknown_value_and_respects_lock() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    for params in [json!({"mode": "giant"}), json!({"mode": "Expanded"}), json!({"mode": null}), json!({"mode": 1}), json!({})] {
        assert_eq!(app.run("ui.ribbon.mode", params).unwrap_err(), "mode must be expanded, compact, tabs_only or hidden");
        assert_eq!(app.ws.user.layout, before);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    for pointer in ["/ribbon", "/ribbon/mode", "/ribbon/visible", "/ribbon/collapsed"] {
        let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": [pointer]}]});
        let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
        let before = app.ws.user.layout.clone();
        let effective = app.ws.effective.layout.ribbon.clone();
        let steps = app.ws.history.undo_len();
        for (command, params) in [("ui.ribbon.mode", json!({"mode": "tabs_only"})), ("ui.ribbon.collapse", json!({"collapsed": true}))] {
            assert_eq!(app.run(command, params).unwrap_err(), LOCKED_MESSAGE, "{command}: {pointer}");
            assert_eq!(app.ws.user.layout, before);
            assert_eq!(app.ws.effective.layout.ribbon, effective);
            assert_eq!(app.ws.history.undo_len(), steps);
            assert!(!app.session.state().unwrap().is_dirty());
        }
    }
}

#[test]
fn collapse_command_maps_to_tabs_only() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    for mode in ["expanded", "compact", "tabs_only", "hidden"] {
        app.run("ui.ribbon.mode", json!({"mode": mode})).unwrap();
        app.run("ui.ribbon.collapse", json!({"collapsed": true})).unwrap();
        let ribbon = &app.ws.effective.layout.ribbon;
        assert_eq!(ribbon.mode, Some(RibbonMode::TabsOnly));
        assert!(ribbon.visible && ribbon.collapsed);
        app.run("ui.ribbon.collapse", json!({"collapsed": false})).unwrap();
        let ribbon = &app.ws.effective.layout.ribbon;
        assert_eq!(ribbon.mode, Some(RibbonMode::Expanded));
        assert!(ribbon.visible && !ribbon.collapsed);
    }
    app.run("ui.ribbon.collapse", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::TabsOnly);
    app.run("ui.ribbon.collapse", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.ribbon.effective_mode(), RibbonMode::Expanded);
    assert!(!app.session.state().unwrap().is_dirty());
}

fn familiar(mode: &str) -> CadApp {
    let mut app = attached(MemoryStore::default());
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app.run("ui.ribbon.mode", json!({"mode": mode})).unwrap();
    app
}

fn popup_open(h: &Harness<'_, CadApp>) -> bool {
    egui::Popup::is_id_open(&h.ctx, Id::new("cc_ribbon_flyout"))
}

fn inside(screen: Rect, rect: Rect) {
    assert!(rect.is_positive() && screen.expand(0.5).contains_rect(rect), "{rect:?} outside {screen:?}");
}

fn unique_labels(tab: &cadcraft_ui_egui::wsprofile::schema::RibbonTab, label: impl Fn(&str) -> String) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for id in tab.groups.iter().flat_map(|group| &group.commands) {
        let label = label(id);
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

fn ribbon_buttons<'h>(h: &'h Harness<'_, CadApp>, label: &'h str) -> impl Iterator<Item = egui_kittest::Node<'h>> + 'h {
    let popup = popup_open(h);
    let ribbon = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, label).filter(move |node| {
        if !popup {
            return ribbon.y_range().contains(node.rect().center().y);
        }
        // Docked palettes can overlap the popup and share labels; use ancestry rather than screen position.
        let mut ancestor = Some(*node);
        while let Some(node) = ancestor {
            if node.accesskit_node().locate().0 == Id::new("cc_ribbon_flyout").accesskit_id() {
                return true;
            }
            ancestor = node.parent();
        }
        false
    })
}

fn button_nth<'h>(h: &'h Harness<'_, CadApp>, label: &'h str, nth: usize) -> egui_kittest::Node<'h> {
    ribbon_buttons(h, label).nth(nth).unwrap_or_else(|| panic!("no ribbon button {label} #{nth}"))
}

// Dock panel tabs are Role::Tab too and can share a ribbon tab's label ("Layers").
fn ribbon_tab<'h>(h: &'h Harness<'_, CadApp>, label: &'h str) -> Option<egui_kittest::Node<'h>> {
    let ribbon = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
    h.query_all_by_role_and_label(egui::accesskit::Role::Tab, label).find(|node| ribbon.y_range().contains(node.rect().center().y))
}

fn command_label(id: &str) -> String {
    let label = cadcraft_engine::find_command(id)
        .map(|spec| spec.label)
        .or_else(|| cadcraft_ui_egui::menus::UI_COMMANDS.iter().find(|entry| entry.0 == id).map(|entry| entry.1))
        .unwrap();
    cadcraft_ui_egui::i18n::t(label).to_string()
}

#[test]
fn tabs_only_flyout_reaches_every_command_at_200_percent_tc11() {
    let _guard = test_lock();
    for language in ["en", "uk", "vi"] {
        let mut app = familiar("tabs_only");
        app.run("options", json!({"interfaceLanguage": language})).unwrap();
        let mut h = scaled_harness(app, 2.0);
        let tabs = cadcraft_ui_egui::ribbon::visible_tabs(h.state());
        let before = h.state().canvas.rect.unwrap();
        let layout = h.state().ws.user.layout.clone();
        for tab in tabs {
            let title = cadcraft_ui_egui::i18n::with_language(h.state().language(), || cadcraft_ui_egui::i18n::ribbon(&tab.label).to_string());
            select_ribbon_tab(&mut h, &title);
            assert!(popup_open(&h), "{language}: {}", tab.id);
            assert_eq!(h.state().canvas.rect.unwrap(), before, "flyout must not move the canvas");
            for label in unique_labels(&tab, |id| cadcraft_ui_egui::i18n::with_language(h.state().language(), || command_label(id))) {
                // Several commands can share a label (Modify "Copy" and Clipboard "Copy"); each must be reachable.
                for nth in 0..ribbon_buttons(&h, &label).count().max(1) {
                    button_nth(&h, &label, nth).scroll_to_me();
                    h.run_steps(3);
                    inside(h.ctx.content_rect(), button_nth(&h, &label, nth).rect());
                }
                let popup = h.ctx.read_response(Id::new("cc_ribbon_flyout")).unwrap();
                inside(h.ctx.content_rect(), popup.rect);
                assert_eq!(popup.layer_id.order, egui::Order::Foreground);
            }
            h.key_press(Key::Escape);
            h.run_steps(3);
            assert!(!popup_open(&h));
        }
        assert_eq!(h.state().ws.user.layout, layout);
        assert!(!h.state().session.state().unwrap().is_dirty());
    }
}

#[test]
fn tabs_only_flyout_wraps_groups_without_horizontal_overflow() {
    let _guard = test_lock();
    for density in ["compact", "normal", "comfortable"] {
        let mut app = familiar("tabs_only");
        app.run("ui.density", json!({"density": density})).unwrap();
        let mut h = scaled_harness(app, 2.0);
        select_ribbon_tab(&mut h, "Home");
        assert!(popup_open(&h));
        let screen = h.ctx.content_rect();
        let home = cadcraft_ui_egui::ribbon::visible_tabs(h.state()).into_iter().find(|tab| tab.id == "home").unwrap();
        let mut group_tops = Vec::new();
        for group in &home.groups {
            let first = group.commands.first().unwrap();
            let first_label = command_label(first);
            group_tops.push(button_nth(&h, &first_label, 0).rect().top());
            for id in &group.commands {
                let label = command_label(id);
                let buttons: Vec<_> = ribbon_buttons(&h, &label).collect();
                assert!(!buttons.is_empty(), "{density}: {id}");
                for button in buttons {
                    let rect = button.rect();
                    assert!(rect.left() >= screen.left() && rect.right() <= screen.right(), "{density}: {id} {rect:?}");
                }
            }
        }
        assert!(group_tops.iter().any(|top| *top > group_tops[0] + 1.0), "{density}: groups must wrap to another row");
        inside(screen, h.ctx.read_response(Id::new("cc_ribbon_flyout")).unwrap().rect);
    }
}

#[test]
fn tabs_only_flyout_wraps_a_wide_custom_group_at_200_percent() {
    let _guard = test_lock();
    let mut app = familiar("tabs_only");
    let tab = app.run("ui.ribbon.tab.add", json!({"label": "Flyout wrapping"})).unwrap()["id"].as_str().unwrap().to_string();
    let group = app.run("ui.ribbon.group.add", json!({"tab": tab, "label": "Commands"})).unwrap()["id"].as_str().unwrap().to_string();
    let ids: Vec<_> = cadcraft_ui_egui::menus::UI_COMMANDS
        .iter()
        .filter(|entry| entry.0.starts_with("ui.") && cadcraft_ui_egui::toolbars::resolve(entry.0).icon.is_none())
        .take(36)
        .map(|entry| entry.0)
        .collect();
    assert_eq!(ids.len(), 36);
    for id in &ids {
        app.run("ui.ribbon.button.add", json!({"tab": tab, "group": group, "id": id})).unwrap();
    }
    let mut h = scaled_harness(app, 2.0);
    let canvas = h.state().canvas.rect.unwrap();
    let layout = h.state().ws.user.layout.clone();
    select_ribbon_tab(&mut h, "Flyout wrapping");
    assert!(popup_open(&h));
    let first = button_nth(&h, &command_label(ids[0]), 0).rect();
    let last = button_nth(&h, &command_label(ids[ids.len() - 1]), 0).rect();
    assert!(last.top() - first.top() > 80.0, "wide groups wrap into additional rows");
    for id in ids {
        let label = command_label(id);
        button_nth(&h, &label, 0).scroll_to_me();
        h.run_steps(3);
        inside(h.ctx.content_rect(), button_nth(&h, &label, 0).rect());
        inside(h.ctx.content_rect(), h.ctx.read_response(Id::new("cc_ribbon_flyout")).unwrap().rect);
    }
    assert_eq!(h.state().canvas.rect.unwrap(), canvas);
    assert_eq!(h.state().ws.user.layout, layout);
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_steps_down_when_height_is_short_h14() {
    let _guard = test_lock();
    let mut h = scaled_harness(familiar("expanded"), 2.0);
    assert_eq!(cadcraft_ui_egui::ribbon::render::rendered_mode(h.state(), h.ctx.content_rect().height()), RibbonMode::TabsOnly);
    h.get_by_label("Home").click();
    h.run_steps(4);
    assert!(popup_open(&h));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Line").click();
    h.run_steps(3);
    assert!(!popup_open(&h));
    assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("line"));
    assert!(!h.get_by_label("Home").is_focused());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().session.running.is_none());
    assert_eq!(h.state().ws.effective.layout.ribbon.effective_mode(), RibbonMode::Expanded);
    let app = familiar("expanded");
    assert_eq!(cadcraft_ui_egui::ribbon::render::rendered_mode(&app, 768.0), RibbonMode::Expanded);
    assert_eq!(cadcraft_ui_egui::ribbon::render::rendered_mode(&app, 560.0), RibbonMode::Compact);
    let h = harness(app);
    assert_eq!(cadcraft_ui_egui::ribbon::render::rendered_mode(h.state(), h.ctx.content_rect().height()), RibbonMode::Expanded);
    let row = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
    assert!(row.height() > cadcraft_ui_egui::ribbon::render::COMPACT_HEIGHT);
}

#[test]
fn compact_mode_shows_icon_buttons_with_tooltips() {
    let _guard = test_lock();
    let mut app = familiar("compact");
    app.run("ui.ribbon.tab.add", json!({"label": "Compact test"})).unwrap();
    let tab = app.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.label == "Compact test").unwrap().id.clone();
    app.run("ui.ribbon.group.add", json!({"tab": tab, "label": "Commands"})).unwrap();
    let group = app.ws.effective.layout.ribbon.tabs.iter().find(|item| item.id == tab).unwrap().groups.first().unwrap().id.clone();
    let ids = ["line", "ui.ribbon.mode"];
    for id in ids {
        app.run("ui.ribbon.button.add", json!({"tab": tab, "group": group, "id": id})).unwrap();
    }
    let mut h = harness(app);
    for tab in cadcraft_ui_egui::ribbon::visible_tabs(h.state()) {
        ribbon_tab(&h, &tab.label).unwrap().scroll_to_me();
        h.run_steps(3);
        ribbon_tab(&h, &tab.label).unwrap().click();
        h.run_steps(3);
        let row = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
        assert!(row.height() <= cadcraft_ui_egui::ribbon::render::COMPACT_HEIGHT + 1.0);
        for label in unique_labels(&tab, command_label) {
            let count = ribbon_buttons(&h, &label).count().max(1);
            for nth in 0..count {
                button_nth(&h, &label, nth).scroll_to_me();
                h.run_steps(3);
                let item = button_nth(&h, &label, nth);
                if item.rect().width() > item.rect().height() + 1.0 {
                    assert!(item.rect().width() > item.rect().height(), "{label} has room for text");
                    assert!(button_paints_text(&h, &label, item.rect()), "{label} paints its label");
                } else {
                    assert!(item.rect().width() <= 25.0, "{label} uses a square icon button");
                    assert!((item.rect().width() - item.rect().height()).abs() <= 0.5, "{label} uses a square icon button");
                }
                let before_hover = h.query_all_by_label(&label).count();
                item.hover();
                h.run_steps(6);
                assert!(h.query_all_by_label(&label).count() > before_hover, "{label} has a labelled tooltip");
                h.hover_at(egui::pos2(1000.0, 650.0));
                h.run_steps(3);
            }
        }
    }
}

#[test]
fn hidden_ribbon_always_offers_show_ribbon() {
    let _guard = test_lock();
    for integrated in [false, true] {
        let mut app = familiar("hidden");
        app.integrated_titlebar = integrated;
        app.run("ui.toggle.menubar", json!({"on": false})).unwrap();
        let mut h = harness(app);
        let strip = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
        assert!((strip.height() - 6.0).abs() <= 0.5);
        inside(h.ctx.content_rect(), h.get_by_label("Show ribbon").rect());
        h.get_by_label("Show ribbon").click();
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon.effective_mode(), RibbonMode::Expanded);
        assert!(h.query_by_label("Show ribbon").is_none());
        assert!(!h.state().session.state().unwrap().is_dirty());
    }
}

#[test]
fn flyout_is_keyboard_operable() {
    let _guard = test_lock();
    let mut h = scaled_harness(familiar("tabs_only"), 2.0);
    h.get_by_label("Home").focus();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(popup_open(&h));
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Line").is_some());
    h.key_press(Key::ArrowRight);
    h.run_steps(3);
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Tab, "Insert").is_focused());
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, &command_label("wblock")).is_some());
    h.key_press(Key::ArrowLeft);
    h.run_steps(3);
    assert!(h.get_by_label("Home").is_focused());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Line").focus();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!popup_open(&h));
    assert!(h.get_by_label("Home").is_focused());
    h.key_press(Key::Space);
    h.run_steps(3);
    assert!(popup_open(&h));
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Insert").click();
    h.run_steps(3);
    assert!(popup_open(&h));
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, &command_label("wblock")).is_some());
    // Wrapped groups can cover the canvas at 200 %, so use the screen margin above the flyout.
    let screen = h.ctx.content_rect();
    let flyout = h.ctx.read_response(Id::new("cc_ribbon_flyout")).unwrap().rect;
    let pos = screen.min + vec2(1.0, 1.0);
    assert!(screen.contains(pos) && !flyout.contains(pos), "{pos:?} must be on screen outside {flyout:?}");
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(!popup_open(&h));
}

fn select_ribbon_tab(h: &mut Harness<'_, CadApp>, title: &str) {
    if let Some(tab) = ribbon_tab(h, title) {
        tab.click();
    } else {
        h.get_by_label("»").click();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::Button, title).click();
    }
    h.run_steps(4);
}

#[test]
fn tab_overflow_keeps_every_tab_reachable() {
    let _guard = test_lock();
    let mut app = familiar("tabs_only");
    app.run("options", json!({"interfaceLanguage": "vi"})).unwrap();
    for number in 0..5 {
        app.run("ui.ribbon.tab.add", json!({"label": format!("Overflow test tab {number}")})).unwrap();
    }
    let mut h = harness(app);
    h.set_size(vec2(640.0, 360.0));
    h.run_steps(5);
    let tabs = cadcraft_ui_egui::ribbon::visible_tabs(h.state());
    let layout = h.state().ws.user.layout.clone();
    assert!(h.query_by_label("»").is_some());
    assert!(tabs.iter().any(|tab| {
        let title = cadcraft_ui_egui::i18n::with_language("vi", || cadcraft_ui_egui::i18n::ribbon(&tab.label).to_string());
        ribbon_tab(&h, &title).is_none()
    }));
    for tab in tabs {
        let title = cadcraft_ui_egui::i18n::with_language("vi", || cadcraft_ui_egui::i18n::ribbon(&tab.label).to_string());
        let overflowed = ribbon_tab(&h, &title).is_none();
        if overflowed {
            h.get_by_label("»").click();
            h.run_steps(3);
            let item = h.get_by_role_and_label(egui::accesskit::Role::Button, &title);
            inside(h.ctx.content_rect(), item.rect());
            item.click();
            h.run_steps(4);
        } else {
            select_ribbon_tab(&mut h, &title);
        }
        let active: String = h.ctx.data(|data| data.get_temp(Id::new("cc_ribbon_active"))).unwrap();
        assert_eq!(active, tab.id);
        let item = ribbon_tab(&h, &title).unwrap();
        inside(h.ctx.content_rect(), item.rect());
        assert_eq!(item.accesskit_node().is_selected(), Some(true));
        assert!(popup_open(&h));
        h.key_press(Key::Escape);
        h.run_steps(3);
    }
    assert_eq!(h.state().ws.user.layout, layout);
    assert!(!h.state().session.state().unwrap().is_dirty());
}

#[test]
fn context_menu_hides_and_restores_a_tab() {
    let _guard = test_lock();
    let mut h = harness(familiar("expanded"));
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Annotate").click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_label("Hide Tab").click();
    h.run_steps(3);
    assert!(!h.state().ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "annotate").unwrap().visible);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Tab, "Annotate").is_none());
    h.get_by_label("Home").click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_label("Show Tabs ⏵").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Annotate").click();
    h.run_steps(3);
    assert!(h.state().ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "annotate").unwrap().visible);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Tab, "Annotate").is_some());
    let restored = restarted(h.state_mut());
    assert!(restored.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "annotate").unwrap().visible);
    assert!(!restored.session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_reset_restores_only_the_ribbon() {
    let _guard = test_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {"ribbon": {"tabs": [{"id": "home", "label": "Company Home"}]}}, "locks": []}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let ribbon = app.ws.below_user().ribbon;
    assert_eq!(ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().label, "Company Home");
    app.run("ui.ribbon.tab.rename", json!({"tab": "home", "label": "My tab"})).unwrap();
    app.run("ui.ribbon.tab.visible", json!({"tab": "annotate", "visible": false})).unwrap();
    app.run("ui.ribbon.mode", json!({"mode": "compact"})).unwrap();
    app.run("aliasedit", json!({"set": {"LL": "line"}})).unwrap();
    app.run("ui.quickaccess.pin", json!({"id": "line"})).unwrap();
    app.run("ui.shortcut.bind", json!({"command": "line", "key": "Ctrl+Shift+L"})).unwrap();
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    let mut expected = before.clone();
    expected.ribbon = ribbon;
    assert_eq!(app.run("ui.ribbon.reset", json!({})).unwrap()["changed"], true);
    assert_eq!(app.ws.user.layout, expected);
    assert_eq!(app.ws.effective.layout, expected);
    assert_eq!(app.ws.history.undo_len(), steps + 1);
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, before);
    app.run("ui.layout.redo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, expected);
    assert_eq!(app.run("ui.ribbon.reset", json!({})).unwrap()["changed"], false);
    let restored = restarted(&mut app);
    assert_eq!(restored.ws.effective.layout, expected);
    assert_eq!(cadcraft_engine::cmd::resolve_alias("LL"), "line");
    assert!(!restored.session.state().unwrap().is_dirty());
}

#[test]
fn customize_ribbon_entry_opens_the_ribbon_page() {
    let _guard = test_lock();
    let mut h = harness(familiar("expanded"));
    h.state_mut().run("ui.customize.open", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Keyboard").click();
    h.run_steps(3);
    h.get_by_label("Close").click();
    h.run_steps(3);
    let polar = h.state().session.settings.polarmode;
    h.get_by_label("Home").focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::SHIFT, Key::F10);
    h.run_steps(3);
    assert!(h.get_by_label("Customize Ribbon…").is_focused());
    assert_eq!(h.state().session.settings.polarmode, polar);
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert_eq!(h.state().ui.dialog.as_deref(), Some(cadcraft_ui_egui::customize::DIALOG));
    assert!(h.query_by_label("Ribbon tabs").is_some());
    assert!(h.query_by_label("Ribbon groups").is_some());
}

#[test]
fn last_visible_tab_cannot_be_hidden() {
    let _guard = test_lock();
    let mut app = familiar("expanded");
    let tabs = app.ws.effective.layout.ribbon.tabs.clone();
    for tab in tabs.iter().filter(|tab| tab.visible && tab.id != "home") {
        app.run("ui.ribbon.tab.visible", json!({"tab": tab.id, "visible": false})).unwrap();
    }
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    assert_eq!(app.run("ui.ribbon.tab.visible", json!({"tab": "home", "visible": false})).unwrap_err(), "At least one ribbon tab must stay visible");
    assert_eq!(app.ws.user.layout, before);
    assert_eq!(app.ws.history.undo_len(), steps);
    assert!(!app.session.state().unwrap().is_dirty());
}

#[test]
fn ribbon_context_menu_selects_every_display_mode() {
    let _guard = test_lock();
    for (label, mode) in
        [("Expanded", RibbonMode::Expanded), ("Compact", RibbonMode::Compact), ("Tabs Only", RibbonMode::TabsOnly), ("Hidden", RibbonMode::Hidden)]
    {
        let mut h = harness(familiar("expanded"));
        if mode == RibbonMode::Compact {
            let ribbon = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
            let pos = egui::pos2(ribbon.right() - 4.0, ribbon.top() + 14.0);
            h.hover_at(pos);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(2);
            }
            assert!(h.get_by_label("Hide Tab").accesskit_node().is_disabled());
        } else {
            h.get_by_label("Home").click_button(egui::PointerButton::Secondary);
            h.run_steps(3);
        }
        h.get_by_label("Display Mode ⏵").click();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::RadioButton, label).click();
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon.effective_mode(), mode);
        assert!(!h.state().session.state().unwrap().is_dirty());
    }
}

#[test]
fn ribbon_tab_visibility_rejects_invalid_params_and_respects_lock() {
    let _guard = test_lock();
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/ribbon/tabs/home/visible"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    for params in [
        json!({}),
        json!({"tab": "home"}),
        json!({"tab": "home", "visible": "false"}),
        json!({"tab": "home", "visible": null}),
        json!({"tab": "unknown", "visible": false}),
    ] {
        assert!(app.run("ui.ribbon.tab.visible", params).is_err());
        assert_eq!(app.ws.user.layout, before);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    assert_eq!(app.run("ui.ribbon.tab.visible", json!({"tab": "home", "visible": false})).unwrap_err(), LOCKED_MESSAGE);
    assert_eq!(app.ws.user.layout, before);
    assert_eq!(app.ws.history.undo_len(), steps);
}

fn button_paints_text(h: &Harness<'_, CadApp>, label: &str, rect: Rect) -> bool {
    fn contains(shape: &egui::epaint::Shape, label: &str, rect: Rect) -> bool {
        match shape {
            egui::epaint::Shape::Text(text) => text.galley.job.text == label && rect.contains(text.pos),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|shape| contains(shape, label, rect)),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|shape| contains(&shape.shape, label, rect))
}

#[test]
fn view_menu_selects_each_ribbon_mode_without_parameters() {
    let _guard = test_lock();
    for (id, label, mode) in [
        ("ui.ribbon.mode.expanded", "Expanded", RibbonMode::Expanded),
        ("ui.ribbon.mode.compact", "Compact", RibbonMode::Compact),
        ("ui.ribbon.mode.tabs_only", "Tabs Only", RibbonMode::TabsOnly),
        ("ui.ribbon.mode.hidden", "Hidden", RibbonMode::Hidden),
    ] {
        let mut h = harness(familiar("expanded"));
        h.get_by_role_and_label(egui::accesskit::Role::Button, "View").click();
        h.run_steps(3);
        h.get_by_label("Ribbon ⏵").click();
        h.run_steps(3);
        h.get_by_label("Display Mode ⏵").click();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
        h.run_steps(3);
        assert_eq!(h.state().ws.effective.layout.ribbon.effective_mode(), mode);
        assert!(!h.state().session.state().unwrap().is_dirty());
        let restored = restarted(h.state_mut());
        assert_eq!(restored.ws.effective.layout.ribbon.effective_mode(), mode);
        for language in ["uk", "vi"] {
            assert_ne!(cadcraft_ui_egui::i18n::tr(language, label), label, "{id}: {language}");
        }
    }
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/ribbon/mode"]}]});
    let mut app = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    let before = app.ws.user.layout.clone();
    assert_eq!(app.run("ui.ribbon.mode.compact", json!({})).unwrap_err(), LOCKED_MESSAGE);
    assert_eq!(app.ws.user.layout, before);
}

#[test]
fn compact_iconless_commands_paint_readable_text_with_hover_labels() {
    let _guard = test_lock();
    for density in ["normal", "comfortable"] {
        let mut app = familiar("compact");
        app.run("ui.density", json!({"density": density})).unwrap();
        let tab = app.run("ui.ribbon.tab.add", json!({"label": "Text buttons"})).unwrap()["id"].as_str().unwrap().to_string();
        let group = app.run("ui.ribbon.group.add", json!({"tab": tab, "label": "Text"})).unwrap()["id"].as_str().unwrap().to_string();
        for id in ["attsync", "battman", "blockreplace", "ui.ribbon.mode"] {
            assert!(cadcraft_ui_egui::toolbars::resolve(id).icon.is_none());
            app.run("ui.ribbon.button.add", json!({"tab": tab, "group": group, "id": id})).unwrap();
        }
        let mut h = harness(app);
        select_ribbon_tab(&mut h, "Text buttons");
        for id in ["attsync", "battman", "blockreplace", "ui.ribbon.mode"] {
            let label = command_label(id);
            let item = button_nth(&h, &label, 0);
            inside(h.ctx.content_rect(), item.rect());
            assert!(item.rect().width() > item.rect().height(), "{density}: {id}");
            assert!(button_paints_text(&h, &label, item.rect()), "{density}: {id}");
            let before = h.query_all_by_label(&label).count();
            item.hover();
            h.run_steps(6);
            assert!(h.query_all_by_label(&label).count() > before, "{density}: {id} has a full tooltip");
            h.hover_at(egui::pos2(1000.0, 650.0));
            h.run_steps(3);
        }
    }
}
