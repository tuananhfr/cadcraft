use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_ui_egui::theme::{self, Tokens};
use cadcraft_ui_egui::wsprofile::LOCKED_MESSAGE;
use cadcraft_ui_egui::wsprofile::schema::{Density, WorkspaceLayout};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, ribbon};
use egui::{Color32, Id, Rect, Theme, accesskit::Role, vec2};
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
    for density in [Density::Compact, Density::Normal, Density::Comfortable] {
        let metrics = density.metrics();
        assert!(metrics.interact_height >= 18.0);
        assert!(metrics.ribbon_button >= 18.0 && metrics.toolbar_button >= 18.0 && metrics.tab_height >= 18.0);
    }
    let ctx = egui::Context::default();
    theme::apply(&ctx, Theme::Dark, false);
    theme::apply_density(&ctx, Density::Comfortable);
    theme::apply_density(&ctx, Density::Normal);
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
            let panels: Vec<_> = ["cc_title_toolbar", "cc_quick_access_row", "cc_ribbon", "cc_toolbars_top", "cc_file_tabs"]
                .into_iter()
                .filter_map(|id| egui::containers::panel::PanelState::load(&h.ctx, Id::new(id)).map(|state| state.outer_rect))
                .collect();
            let mut count = 0;
            for node in h.query_all_by_role(Role::Button) {
                let rect = node.rect();
                if panels.iter().any(|panel| panel.contains(rect.center())) {
                    count += 1;
                    assert!(rect.width() >= 32.0 && rect.height() >= 32.0, "{mode}/{position}: {node:?}: {rect:?}");
                    inside(h.ctx.content_rect(), rect, "Comfortable button");
                }
            }
            assert!(count >= 10, "{mode}/{position}: expected ribbon, QAT, toolbar and file-tab buttons");
            for label in ["New Drawing", "Line", "Circle", "Quick Plot..."] {
                assert!(h.query_all_by_label(label).any(|node| node.rect().width() >= 32.0 && node.rect().height() >= 32.0), "{label}");
            }
        }
    }
}

#[test]
fn every_density_keeps_controls_reachable() {
    let _guard = test_lock();
    for density in ["compact", "normal", "comfortable"] {
        for preset in ["SIMPLE", "AUTOCAD_FAMILIAR"] {
            for ppp in [1.0, 2.0] {
                let mut app = attached(MemoryStore::default());
                app.run("ui.workspace.select", json!({"id": preset})).unwrap();
                app.run("ui.toggle.menubar", json!({"on": true})).unwrap();
                app.run("ui.density", json!({"density": density})).unwrap();
                let mut h = harness(app, ppp);
                let screen = h.ctx.content_rect();
                let case = format!("{density}/{preset}/ppp={ppp}");
                for label in ["File", "Command line input", "Status bar"] {
                    inside(screen, h.get_by_label(label).rect(), &format!("{case}: {label}"));
                }
                let canvas = h.state().canvas.rect.unwrap();
                inside(screen, canvas, &case);
                assert!(canvas.width() >= 200.0 && canvas.height() >= 150.0, "{case}: {canvas:?}");
                if preset == "AUTOCAD_FAMILIAR" {
                    for label in ["Home", "Open (Ctrl+O)", "Search commands..."] {
                        inside(screen, h.query_all_by_label(label).next().unwrap().rect(), &format!("{case}: {label}"));
                    }
                    assert!(!h.state().ws.effective.layout.ribbon.collapsed);
                    h.state_mut().run("ui.ribbon.mode", json!({"mode": "tabs_only"})).unwrap();
                    h.run_steps(4);
                    let canvas = h.state().canvas.rect.unwrap();
                    let tabs = ribbon::visible_tabs(h.state());
                    for tab in tabs {
                        if h.query_all_by_role_and_label(Role::Tab, &tab.label).next().is_some() {
                            h.query_all_by_role_and_label(Role::Tab, &tab.label).next().unwrap().click();
                        } else {
                            h.get_by_label("»").click();
                            h.run_steps(3);
                            h.query_all_by_role_and_label(Role::Button, &tab.label).next().unwrap().click();
                        }
                        h.run_steps(4);
                        assert!(egui::Popup::is_id_open(&h.ctx, Id::new("cc_ribbon_flyout")), "{case}: {}", tab.id);
                        for id in tab.groups.iter().flat_map(|group| &group.commands) {
                            let label = cadcraft_engine::find_command(id)
                                .map(|spec| spec.label.to_string())
                                .unwrap_or_else(|| cadcraft_ui_egui::toolbars::resolve(id).label);
                            let count = h.query_all_by_role_and_label(Role::Button, &label).count();
                            assert!(count > 0, "{case}: {label}");
                            for nth in 0..count {
                                h.query_all_by_role_and_label(Role::Button, &label).nth(nth).unwrap().scroll_to_me();
                                h.run_steps(3);
                                inside(screen, h.query_all_by_role_and_label(Role::Button, &label).nth(nth).unwrap().rect(), &label);
                            }
                        }
                        assert_eq!(h.state().canvas.rect.unwrap(), canvas, "{case}: flyout preserves canvas");
                        h.key_press(egui::Key::Escape);
                        h.run_steps(3);
                    }
                } else {
                    assert!(h.state().ws.effective.layout.quick_access.is_empty());
                    assert!(!h.state().ws.effective.layout.ribbon.visible);
                }
                assert!(!h.state().session.state().unwrap().is_dirty());
            }
        }
    }
}

#[test]
fn theme_text_and_focus_contrast_meet_wcag() {
    assert!((theme::contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.001);
    assert_eq!(theme::contrast_ratio(Color32::WHITE, Color32::WHITE), 1.0);
    for theme in [Theme::Dark, Theme::Light] {
        let tokens = Tokens::of(theme);
        let ctx = egui::Context::default();
        theme::apply(&ctx, theme, false);
        let visuals = ctx.style().visuals.clone();
        assert_eq!(visuals.selection.stroke.color, tokens.accent);
        for bg in [tokens.panel, tokens.chrome, tokens.chrome_dark] {
            for text in [tokens.text, tokens.text_dim, tokens.text_faint, visuals.weak_text_color()] {
                let ratio = theme::contrast_ratio(text, bg);
                assert!(ratio >= 4.5, "{theme:?}: text {text:?} on {bg:?}: {ratio}");
                assert_eq!(ratio, theme::contrast_ratio(bg, text));
            }
            let ratio = theme::contrast_ratio(visuals.selection.stroke.color, bg);
            assert!(ratio >= 3.0, "{theme:?}: focus on {bg:?}: {ratio}");
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
    }
}

#[test]
fn density_command_validates_locks_and_undo_without_changing_the_drawing() {
    let _guard = test_lock();
    assert!(cadcraft_ui_egui::menus::UI_COMMANDS.iter().any(|entry| entry.0 == "ui.density"));
    let mut app = attached(MemoryStore::default());
    let drawing = app.session.state().unwrap().doc.clone();
    let camera = app.session.state().unwrap().view;
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
    assert_eq!(app.session.state().unwrap().view, camera);
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
    h.query_all_by_label("Normal").next().unwrap().click();
    h.run_steps(3);
    h.query_all_by_label("Comfortable").next().unwrap().click();
    h.run_steps(4);
    assert_eq!(h.state().ws.effective.layout.density, Density::Comfortable);
    assert_eq!(h.state().ws.history.undo_len(), steps + 1);
    assert!(!h.state().session.state().unwrap().is_dirty());
}
