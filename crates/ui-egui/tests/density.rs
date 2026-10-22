use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::theme::{self, Tokens};
use cadcraft_ui_egui::wsprofile::LOCKED_MESSAGE;
use cadcraft_ui_egui::wsprofile::schema::{Density, RibbonTab, WorkspaceLayout};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, ribbon};
use egui::{Color32, Id, Rect, Theme, accesskit::Role, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use serde_json::json;

// App restarts share the engine's process-wide alias store.
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
    app.run("options", json!({"interfaceLanguage": "en", "animations": "off"})).unwrap();
    app
}

fn harness(app: CadApp, ppp: f32) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0) / ppp).with_pixels_per_point(ppp).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
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

fn panel(h: &Harness<'_, CadApp>, id: &str) -> Option<Rect> {
    egui::containers::panel::PanelState::load(&h.ctx, Id::new(id)).map(|state| state.outer_rect)
}

// Dock panel tabs are Role::Tab too and can share a ribbon tab's label ("Layers").
fn ribbon_tab<'h>(h: &'h Harness<'_, CadApp>, label: &'h str) -> Option<Node<'h>> {
    let ribbon = panel(h, "cc_ribbon")?;
    h.query_all_by_role_and_label(Role::Tab, label).find(|node| ribbon.y_range().contains(node.rect().center().y))
}

// The menu bar's "View" and "Insert" share labels with ribbon tabs; the overflow menu opens below the ribbon's top.
fn select_ribbon_tab(h: &mut Harness<'_, CadApp>, label: &str) {
    if let Some(tab) = ribbon_tab(h, label) {
        tab.click();
    } else {
        h.get_by_label("»").click();
        h.run_steps(3);
        let top = panel(h, "cc_ribbon").map_or(0.0, |ribbon| ribbon.top());
        h.query_all_by_role_and_label(Role::Button, label)
            .find(|node| node.rect().top() >= top)
            .unwrap_or_else(|| panic!("no overflow entry {label}"))
            .click();
    }
    h.run_steps(4);
}

fn button_nth<'h>(h: &'h Harness<'_, CadApp>, label: &'h str, nth: usize) -> Node<'h> {
    h.query_all_by_role_and_label(Role::Button, label).nth(nth).unwrap_or_else(|| panic!("no button {label} #{nth}"))
}

fn command_labels(tab: &RibbonTab) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for id in tab.groups.iter().flat_map(|group| &group.commands).map(String::as_str) {
        let label = cadcraft_engine::find_command(id)
            .map(|spec| spec.label)
            .or_else(|| cadcraft_ui_egui::menus::UI_COMMANDS.iter().find(|entry| entry.0 == id).map(|entry| entry.1))
            .unwrap_or_else(|| panic!("{id} isn't a command"));
        let label = cadcraft_ui_egui::i18n::t(label).to_string();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

#[test]
fn normal_density_matches_todays_metrics() {
    let metrics = Density::Normal.metrics();
    assert_eq!(metrics.interact_height, 20.0);
    assert_eq!(metrics.button_padding, vec2(6.0, 3.0));
    assert_eq!(metrics.item_spacing, vec2(6.0, 4.0));
    assert_eq!(metrics.ribbon_button, 24.0);
    assert_eq!(metrics.toolbar_button, 24.0);
    assert_eq!(metrics.tab_height, 28.0);
    assert_eq!(Density::default(), Density::Normal);
    let legacy: WorkspaceLayout = serde_json::from_value(json!({"quick_access": ["line"]})).unwrap();
    assert_eq!(legacy.density, Density::Normal);
    assert_eq!(WorkspaceLayout::default().density, Density::Normal);
    for density in [Density::Compact, Density::Normal, Density::Comfortable] {
        let sizes = density.metrics();
        assert!(sizes.interact_height >= 18.0, "{density:?}");
        assert!(sizes.ribbon_button >= 18.0 && sizes.toolbar_button >= 18.0 && sizes.tab_height >= 18.0, "{density:?}");
    }
    let comfortable = Density::Comfortable.metrics();
    assert!(comfortable.interact_height >= 32.0 && comfortable.ribbon_button >= 32.0 && comfortable.toolbar_button >= 32.0);
    assert!(comfortable.tab_height >= 32.0);
    // Spacing belongs to the density: a theme change must keep it, and Normal must restore today's values.
    let ctx = egui::Context::default();
    theme::apply(&ctx, Theme::Dark, false);
    theme::apply_density(&ctx, Density::Comfortable);
    assert_eq!(ctx.style().spacing.interact_size.y, comfortable.interact_height);
    theme::apply_density(&ctx, Density::Normal);
    theme::apply(&ctx, Theme::Light, false);
    let style = ctx.style();
    assert_eq!(style.spacing.interact_size.y, metrics.interact_height);
    assert_eq!(style.spacing.button_padding, metrics.button_padding);
    assert_eq!(style.spacing.item_spacing, metrics.item_spacing);
}

#[test]
fn comfortable_targets_are_at_least_32_points() {
    let _guard = test_lock();
    for mode in ["expanded", "compact", "tabs_only"] {
        for position in ["title_bar", "above_ribbon", "below_ribbon"] {
            let mut app = attached(MemoryStore::default());
            app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
            app.run("ui.workspace.set", json!({"pointer": "/quick_access", "value": ["line", "circle", "ubplot"]})).unwrap();
            app.run("ui.quickaccess.position", json!({"position": position})).unwrap();
            app.run("ui.toggle.menubar", json!({"on": false})).unwrap();
            app.run("ui.toolbar.show", json!({"id": "standard"})).unwrap();
            app.run("ui.ribbon.mode", json!({"mode": mode})).unwrap();
            app.run("ui.density", json!({"density": "comfortable"})).unwrap();
            let h = harness(app, 1.0);
            let case = format!("{mode}/{position}");
            let panels: Vec<Rect> = ["cc_title_toolbar", "cc_quick_access_row", "cc_ribbon", "cc_toolbars_top", "cc_file_tabs"]
                .into_iter()
                .filter_map(|id| panel(&h, id))
                .collect();
            let mut count = 0;
            for node in h.query_all_by_role(Role::Button) {
                let rect = node.rect();
                // Buttons scrolled out of a strip keep their full size, so only the size is checked for each.
                if panels.iter().any(|panel| panel.contains(rect.center())) {
                    count += 1;
                    assert!(rect.width() >= 32.0 && rect.height() >= 32.0, "{case}: {:?} {rect:?}", node.accesskit_node().label());
                }
            }
            assert!(count >= 10, "{case}: expected ribbon, Quick Access, toolbar and file-tab buttons, found {count}");
            for label in ["New Drawing", "Line", "Circle", "Quick Plot..."] {
                let reachable = h.query_all_by_label(label).any(|node| {
                    let rect = node.rect();
                    rect.width() >= 32.0 && rect.height() >= 32.0 && h.ctx.content_rect().expand(0.5).contains_rect(rect)
                });
                assert!(reachable, "{case}: {label}");
            }
        }
    }
}

#[test]
fn every_density_keeps_controls_reachable() {
    let _guard = test_lock();
    for density in ["compact", "normal", "comfortable"] {
        for preset in ["SIMPLE", "AUTOCAD_FAMILIAR"] {
            // 1366x768 and 683x384 points.
            for ppp in [1.0, 2.0] {
                let mut app = attached(MemoryStore::default());
                app.run("ui.workspace.select", json!({"id": preset})).unwrap();
                app.run("ui.toggle.menubar", json!({"on": true})).unwrap();
                app.run("ui.density", json!({"density": density})).unwrap();
                let mut h = harness(app, ppp);
                let screen = h.ctx.content_rect();
                let case = format!("{density}/{preset}/{}x{}", screen.width(), screen.height());
                for label in ["File", "Command line input", "Status bar"] {
                    inside(screen, h.get_by_label(label).rect(), &format!("{case}: {label}"));
                }
                inside(screen, panel(&h, "cc_menubar").unwrap(), &format!("{case}: menu bar"));
                let canvas = h.state().canvas.rect.unwrap();
                inside(screen, canvas, &format!("{case}: canvas"));
                assert!(canvas.width() >= 200.0 && canvas.height() >= 150.0, "{case}: canvas {canvas:?}");
                assert!(!h.state().session.state().unwrap().is_dirty(), "{case}");
                if preset != "AUTOCAD_FAMILIAR" {
                    assert!(h.state().ws.effective.layout.quick_access.is_empty());
                    assert!(!h.state().ws.effective.layout.ribbon.visible);
                    continue;
                }
                for label in ["Home", "Open (Ctrl+O)", "Search commands..."] {
                    inside(screen, h.query_all_by_label(label).next().unwrap().rect(), &format!("{case}: {label}"));
                }
                assert!(!h.state().ws.effective.layout.ribbon.collapsed, "{case}: compact rendering preserves the preference");
                h.state_mut().run("ui.ribbon.mode", json!({"mode": "tabs_only"})).unwrap();
                h.run_steps(4);
                let canvas = h.state().canvas.rect.unwrap();
                for tab in ribbon::visible_tabs(h.state()) {
                    select_ribbon_tab(&mut h, &tab.label);
                    assert!(egui::Popup::is_id_open(&h.ctx, Id::new("cc_ribbon_flyout")), "{case}: {}", tab.id);
                    for label in command_labels(&tab) {
                        let count = h.query_all_by_role_and_label(Role::Button, &label).count();
                        assert!(count > 0, "{case}: {label}");
                        // Several commands can share a label (Modify "Copy" and Clipboard "Copy"); each must be reachable.
                        for nth in 0..count {
                            button_nth(&h, &label, nth).scroll_to_me();
                            h.run_steps(3);
                            inside(screen, button_nth(&h, &label, nth).rect(), &format!("{case}: {label} #{nth}"));
                        }
                    }
                    assert_eq!(h.state().canvas.rect.unwrap(), canvas, "{case}: the flyout keeps the canvas");
                    h.key_press(egui::Key::Escape);
                    h.run_steps(3);
                }
                assert!(!h.state().session.state().unwrap().is_dirty(), "{case}");
            }
        }
    }
}

#[test]
fn theme_text_and_focus_contrast_meet_wcag() {
    assert!((theme::contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.001);
    assert_eq!(theme::contrast_ratio(Color32::WHITE, Color32::WHITE), 1.0);
    for shown in [Theme::Dark, Theme::Light] {
        let tokens = Tokens::of(shown);
        let ctx = egui::Context::default();
        theme::apply(&ctx, shown, false);
        let visuals = ctx.style().visuals.clone();
        // Panels, the ribbon and tool bars (chrome) and the file tabs (chrome_dark).
        for bg in [tokens.panel, tokens.chrome, tokens.chrome_dark] {
            for text in [tokens.text, tokens.text_dim, tokens.text_faint, visuals.weak_text_color()] {
                let ratio = theme::contrast_ratio(text, bg);
                assert!(ratio >= 4.5, "{shown:?}: text {text:?} on {bg:?}: {ratio}");
                assert_eq!(ratio, theme::contrast_ratio(bg, text));
            }
            // egui outlines focused text fields with selection.stroke; the app's own controls with the accent.
            for focus in [visuals.selection.stroke.color, tokens.accent] {
                let ratio = theme::contrast_ratio(focus, bg);
                assert!(ratio >= 3.0, "{shown:?}: focus {focus:?} on {bg:?}: {ratio}");
            }
        }
    }
}

#[test]
fn density_persists_across_restart() {
    let _guard = test_lock();
    for (name, expected) in [("compact", Density::Compact), ("normal", Density::Normal), ("comfortable", Density::Comfortable)] {
        let mut app = attached(MemoryStore::default());
        app.run("ui.density", json!({"density": name})).unwrap();
        app.commit_workspace_now().unwrap();
        let store = app.ws.store.as_deref().unwrap();
        let snapshot_map = store.snapshots().into_iter().filter_map(|revision| store.read_snapshot(revision).map(|text| (revision, text))).collect();
        let restored = attached(MemoryStore { current: store.read_current(), policy: store.read_policy(), snapshot_map, ..MemoryStore::default() });
        assert_eq!(restored.ws.user.layout.density, expected);
        assert_eq!(restored.ws.effective.layout.density, expected);
        assert_eq!(serde_json::to_value(&restored.ws.user.layout).unwrap()["density"], name);
        let h = harness(restored, 1.0);
        assert_eq!(h.ctx.style().spacing.interact_size.y, expected.metrics().interact_height);
        assert_eq!(h.ctx.style().spacing.button_padding, expected.metrics().button_padding);
    }
}

#[test]
fn density_command_validates_locks_and_undo_without_changing_the_drawing() {
    let _guard = test_lock();
    assert!(cadcraft_ui_egui::menus::UI_COMMANDS.iter().any(|entry| entry.0 == "ui.density"));
    let mut app = attached(MemoryStore::default());
    let drawing = app.session.state().unwrap().doc.clone();
    let camera = app.session.state().unwrap().view();
    let selection = app.session.state().unwrap().selection.clone();
    let before = app.ws.user.layout.clone();
    let steps = app.ws.history.undo_len();
    for params in [json!({}), json!({"density": null}), json!({"density": 1}), json!({"density": "Comfortable"})] {
        assert_eq!(app.run("ui.density", params).unwrap_err(), "density must be compact, normal or comfortable");
        assert_eq!(app.ws.user.layout, before);
        assert_eq!(app.ws.history.undo_len(), steps);
    }
    assert_eq!(app.run("ui.density", json!({"density": "comfortable"})).unwrap()["changed"], true);
    assert_eq!(app.ws.history.undo_len(), steps + 1);
    assert_eq!(app.run("ui.density", json!({"density": "comfortable"})).unwrap()["changed"], false);
    assert_eq!(app.ws.history.undo_len(), steps + 1);
    app.run("ui.layout.undo", json!({})).unwrap();
    assert_eq!(app.ws.user.layout, before);
    app.run("ui.layout.redo", json!({})).unwrap();
    assert_eq!(app.ws.effective.layout.density, Density::Comfortable);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.state().unwrap().view(), camera);
    assert_eq!(app.session.state().unwrap().selection, selection);
    assert!(std::sync::Arc::ptr_eq(&app.session.state().unwrap().doc, &drawing));
    assert!(!app.session.state().unwrap().is_dirty());
    let policy = json!({"layers": [{"level": "company", "patch": {}, "locks": ["/density"]}]});
    let mut locked = attached(MemoryStore { policy: Some(policy.to_string()), ..MemoryStore::default() });
    let before = locked.ws.user.layout.clone();
    assert_eq!(locked.run("ui.density", json!({"density": "comfortable"})).unwrap_err(), LOCKED_MESSAGE);
    assert_eq!(locked.ws.user.layout, before);
}

#[test]
fn options_display_changes_density_through_the_command() {
    let _guard = test_lock();
    let mut app = attached(MemoryStore::default());
    app.ui.dialog = Some("options:display".into());
    let mut h = harness(app, 1.0);
    let steps = h.state().ws.history.undo_len();
    // A combo box carries its choice as its value, not its label.
    h.query_all_by_role(Role::ComboBox)
        .find(|node| node.accesskit_node().value().as_deref() == Some("Normal"))
        .expect("density combo showing Normal")
        .click();
    h.run_steps(3);
    h.query_all_by_label("Comfortable").next().expect("Comfortable choice").click();
    h.run_steps(4);
    assert_eq!(h.state().ws.effective.layout.density, Density::Comfortable);
    assert_eq!(h.state().ws.history.undo_len(), steps + 1);
    assert_eq!(h.ctx.style().spacing.interact_size.y, Density::Comfortable.metrics().interact_height);
    assert_eq!(h.state().ui.dialog.as_deref(), Some("options:display"));
    assert!(!h.state().session.state().unwrap().is_dirty());
}
