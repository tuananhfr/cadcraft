//! Pseudo-localization exercises display paths without becoming a saved language.

use cadcraft_engine::prompt::{Accept, PromptArg};
use cadcraft_engine::{Prompt, Session};
use cadcraft_ui_egui::i18n::{self, Preference};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services, menus};
use egui::accesskit::Role;
use egui::{Id, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

#[test]
fn pseudo_keeps_placeholders_and_expands() {
    let source = "Version {version}";
    let translated = i18n::pseudo(source);
    assert!(translated.starts_with('[') && translated.ends_with(']'), "{translated}");
    assert!(translated.contains("{version}"), "{translated}");
    assert!(translated.chars().count() * 5 >= source.chars().count() * 7, "{translated}");
    assert_ne!(translated, format!("[{source}]"));
    let copy = source.to_owned();
    assert!(std::ptr::eq(translated, i18n::pseudo(&copy)), "cached strings reuse their allocation");

    let spaced = i18n::pseudo(" \tOpen {file}... …\r\n");
    assert!(spaced.starts_with(" \t["), "{spaced:?}");
    assert!(spaced.ends_with("]\r\n"), "{spaced:?}");
    assert!(spaced.contains("{file}... …"), "{spaced:?}");
    assert_eq!(i18n::pseudo(" \t\n"), " \t\n");
    let hostile = "a".repeat(100_000);
    assert!(std::ptr::eq(i18n::pseudo(&hostile), hostile.as_str()), "oversized inputs are borrowed unchanged");
    for text in ["{unclosed", "{{nested}}", "é中", "..."] {
        assert!(!i18n::pseudo(text).is_empty());
    }
    assert_eq!(i18n::tr("qps", source), translated);
    assert_eq!(i18n::tr_menu("qps", source), translated);
    assert_eq!(i18n::tr_ribbon("qps", source), translated);
    assert_eq!(i18n::tr("en", source), source);

    i18n::with_language("qps", || {
        let user_value = "Drawing {n}";
        let expected = translated.replace("{version}", user_value);
        assert_eq!(i18n::format(source, &[("version", user_value.into())]), expected);
        for n in [1, 2, 0] {
            let form = if n == 1 { "{n} object" } else { "{n} objects" };
            assert_eq!(i18n::count(n, "{n} object", "{n} objects"), i18n::pseudo(form).replace("{n}", &n.to_string()));
        }
        let prompt = Prompt::tf(source, vec![("version", PromptArg::Text(user_value.into()))], Accept::TEXT).kw(&["Undo", "Close"]);
        let english = prompt.message.clone();
        assert_eq!(i18n::prompt_message(&prompt), expected);
        assert_eq!(prompt.message, english);
        assert_eq!(prompt.keywords, ["Undo", "Close"]);
        let fixed = Prompt::new("Specify first point", Accept::POINT);
        assert_eq!(i18n::prompt_message(&fixed), i18n::pseudo(&fixed.message));
        let keyed =
            Prompt::tf("Specify {which} {unit}", vec![("which", PromptArg::Key("radius")), ("unit", PromptArg::Text("mm".into()))], Accept::NUMBER);
        let expected = i18n::pseudo("Specify {which} {unit}").replace("{which}", i18n::pseudo("radius")).replace("{unit}", "mm");
        assert_eq!(i18n::prompt_message(&keyed), expected);
    });
    assert_eq!(i18n::t(source), source, "the scoped language is restored");
}

#[test]
fn pseudo_is_not_saved_and_hides_from_picker() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.run("ui.language", json!({"lang": "en"})).unwrap();
    let prefs = app.prefs_json();
    let profile = app.profile_json();
    let result = app.run("ui.language", json!({"lang": "qps"})).unwrap();
    assert_eq!(result, json!({"interfaceLanguage": "qps", "shown": "qps"}));
    assert_eq!(app.language(), "qps");
    assert_eq!(app.ui.interface_language, Preference::English);
    assert_eq!(app.prefs_json(), prefs);
    assert_eq!(app.profile_json(), profile);
    assert!(!app.prefs_json().contains("qps"));
    assert!(Preference::ALL.iter().all(|preference| preference.code() != "qps"));
    assert!(Preference::parse("qps").is_none());
    assert!(i18n::LANGUAGES.iter().all(|language| language.code != "qps"));

    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.language(), "en");
    assert!(app.run("ui.language", json!({"lang": "bad"})).is_err());
    assert_eq!(app.language(), "qps", "invalid language changes are atomic");
    app.run("ui.language", json!({"lang": "en"})).unwrap();
    assert_eq!(app.language(), "en");
    app.run("ui.language", json!({"lang": "qps"})).unwrap();
    app.run("options", json!({"interfaceLanguage": "en"})).unwrap();
    assert_eq!(app.language(), "en", "the Options language picker also leaves qps");
}

fn harness() -> Harness<'static, CadApp> {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    app.ui.start_tab = false;
    app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
    app.run("ui.language", json!({"lang": "qps"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn inside(screen: Rect, rect: Rect, label: &str) {
    assert!(rect.is_positive() && screen.contains_rect(rect), "{label}: {rect:?} outside {screen:?}");
}

#[test]
fn pseudo_ribbon_and_dialogs_fit_1366x768() {
    let mut h = harness();
    let tabs: Vec<_> = h.state().ws.effective.layout.ribbon.tabs.iter().filter(|tab| tab.contextual.is_none()).map(|tab| tab.id.clone()).collect();
    for tab in tabs {
        h.ctx.data_mut(|data| data.insert_temp(Id::new("cc_ribbon_active"), tab.clone()));
        h.run_steps(3);
        let panel = egui::containers::panel::PanelState::load(&h.ctx, Id::new("cc_ribbon")).unwrap().outer_rect;
        let screen = h.ctx.content_rect();
        inside(screen, panel, "ribbon panel");
        let buttons: Vec<_> = h
            .query_all_by_role(Role::Button)
            .filter(|node| node.rect().center().y >= panel.top() && node.rect().center().y < panel.bottom())
            .map(|node| (node.accesskit_node().label().unwrap_or_default().to_string(), node.rect()))
            .collect();
        assert!(!buttons.is_empty(), "ribbon tab {tab} must expose its buttons");
        for (index, (label, rect)) in buttons.iter().enumerate() {
            inside(screen, *rect, &format!("{tab}: {label}"));
            for (other_label, other_rect) in buttons.iter().skip(index + 1) {
                assert!(!rect.intersects(*other_rect), "{tab}: {label} {rect:?} overlaps {other_label} {other_rect:?}");
            }
        }
    }

    for (command, label) in [("ui.customize.open", "Customize Workspace..."), ("options", "Options")] {
        menus::activate(h.state_mut(), command);
        h.run_steps(5);
        let title = i18n::pseudo(label);
        let window = h.get_by_role_and_label(Role::Window, title);
        inside(h.ctx.content_rect(), window.rect(), title);
        h.state_mut().ui.dialog = None;
        h.run_steps(3);
    }
}
