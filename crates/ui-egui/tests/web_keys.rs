use std::collections::BTreeMap;

use cadcraft_engine::Session;
use cadcraft_ui_egui::shortcuts::{self, BROWSER_RESERVED, BindError, CURRENT, KeyPlatform};
use cadcraft_ui_egui::{CadApp, Services};

#[test]
fn web_defaults_avoid_browser_reserved_keys() {
    let web = shortcuts::defaults_for(KeyPlatform::Web);
    for (command, keys) in &web {
        assert!(!shortcuts::is_browser_reserved(keys), "{command}: {keys}");
        for key in keys.split(',') {
            assert!(!shortcuts::is_browser_reserved(key), "{command}: {key}");
        }
    }
    assert!(!web.get("isoplane").is_some_and(|keys| keys.split(',').any(|key| matches!(key.trim(), "F5" | "Ctrl+E"))));
    assert_eq!(web.get("ortho").map(String::as_str), Some("F8"));
    assert_eq!(web.get("snap").map(String::as_str), Some("F9"));
    assert!(!shortcuts::is_browser_reserved("F8"));
    assert!(!shortcuts::is_browser_reserved("F9"));
    assert!(BROWSER_RESERVED.contains(&"Shift+F10"));
    assert!(BROWSER_RESERVED.contains(&"Ctrl+Shift+C"));
    assert!(shortcuts::is_browser_reserved("shift+f10"));
    assert!(shortcuts::is_browser_reserved(" shift + ctrl + c "));
    assert!(shortcuts::is_browser_reserved("Cmd+Shift+C"));
    assert!(shortcuts::is_browser_reserved("Ctrl+Shift+S, ctrl+r"));
    assert!(!shortcuts::is_browser_reserved("Ctrl+Shift+R"));
    assert_eq!(shortcuts::defaults(), shortcuts::defaults_for(CURRENT));
}

#[test]
fn web_defaults_keep_every_safe_desktop_binding() {
    let desktop = shortcuts::defaults_for(KeyPlatform::Desktop);
    let web = shortcuts::defaults_for(KeyPlatform::Web);
    // Web full screen needs a click and Ctrl+0 resets browser zoom, so these stay desktop-only.
    let desktop_only = |command: &str| matches!(command, "ui.fullscreen" | "ui.focusmode");
    let removed = desktop.iter().filter(|(command, keys)| desktop_only(command) || shortcuts::is_browser_reserved(keys)).count();
    for (command, keys) in &desktop {
        if !desktop_only(command) && !shortcuts::is_browser_reserved(keys) {
            assert_eq!(web.get(command), Some(keys), "{command}");
        }
    }
    let added =
        shortcuts::WEB_ALTERNATES.iter().filter(|(command, _)| desktop.get(*command).is_none_or(|keys| shortcuts::is_browser_reserved(keys))).count();
    assert_eq!(web.len(), desktop.len() - removed + added);
    assert_eq!(desktop.get("isoplane").map(String::as_str), Some("F5, Ctrl+E"));
    assert_eq!(desktop.get("copybase").map(String::as_str), Some("Ctrl+Shift+C"));
}

#[test]
fn web_binder_rejects_browser_keys() {
    let mut map = BTreeMap::from([("rectang".to_string(), "Ctrl+T".to_string())]);
    let before = map.clone();
    for key in BROWSER_RESERVED.iter().copied().chain(["ctrl+w", "shift+f10", "Shift+Ctrl+C", "Cmd+T"]) {
        assert!(shortcuts::is_browser_reserved(key), "{key}");
        for replace in [false, true] {
            assert_eq!(shortcuts::bind_for(KeyPlatform::Web, &mut map, "line", key, replace), Err(BindError::BrowserReserved), "{key}");
            assert_eq!(map, before);
        }
    }
    for key in ["Escape", "Esc", "Ctrl+Q", "Cmd+Q"] {
        assert_eq!(shortcuts::bind_for(KeyPlatform::Web, &mut map, "line", key, false), Err(BindError::Reserved));
        assert_eq!(map, before);
    }
    assert_eq!(shortcuts::bind_for(KeyPlatform::Web, &mut map, "line", "Ctrl+not-a-key", false), Err(BindError::Unparsable));
    assert_eq!(map, before);
    let mut desktop = shortcuts::defaults_for(KeyPlatform::Desktop);
    assert!(matches!(shortcuts::bind_for(KeyPlatform::Desktop, &mut desktop, "line", "F5", false), Ok(()) | Err(BindError::Conflict { .. })));
    for key in ["F8", "F9", "Ctrl+Shift+R"] {
        let mut safe = BTreeMap::new();
        assert_eq!(shortcuts::bind_for(KeyPlatform::Web, &mut safe, "line", key, false), Ok(()));
        assert_eq!(safe.get("line").map(String::as_str), Some(key));
    }
    assert_eq!(BindError::BrowserReserved.to_string(), "The browser uses this key");
}

#[test]
fn web_active_drops_browser_keys_from_profiles() {
    let mut app = CadApp::new(Session::default(), Services::default());
    app.ws.effective.layout.shortcuts.extend([
        ("rectang".to_string(), "Ctrl+R".to_string()),
        ("regen".to_string(), "Ctrl+Shift+R".to_string()),
        ("line".to_string(), "Shift+Ctrl+C".to_string()),
        ("circle".to_string(), "Shift+F10".to_string()),
        ("arc".to_string(), "Ctrl+W, Ctrl+T".to_string()),
        ("erase".to_string(), "Escape, Ctrl+Q".to_string()),
        ("qsave".to_string(), String::new()),
    ]);
    let web = shortcuts::active_for(KeyPlatform::Web, &app);
    for command in ["rectang", "line", "circle", "arc", "erase"] {
        assert!(!web.contains_key(command), "{command}");
    }
    assert_eq!(web.get("regen").map(String::as_str), Some("Ctrl+Shift+R"));
    assert_eq!(web.get("qsave").map(String::as_str), Some(""));
    assert!(web.values().all(|keys| !shortcuts::is_browser_reserved(keys)));
    assert_eq!(shortcuts::active_for(KeyPlatform::Desktop, &app).get("rectang").map(String::as_str), Some("Ctrl+R"));
    assert_eq!(app.ws.effective.layout.shortcuts.get("rectang").map(String::as_str), Some("Ctrl+R"));
    assert_eq!(shortcuts::active(&app), shortcuts::active_for(CURRENT, &app));
}

#[test]
fn web_menu_hides_browser_reserved_hints() {
    assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Web, "F5, Ctrl+E"), None);
    assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Web, "Ctrl+Shift+S"), Some("Ctrl+Shift+S".to_string()));
    assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Desktop, "F5"), Some("F5".to_string()));
    assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Desktop, "F5, Ctrl+E"), Some("F5, Ctrl+E".to_string()));
    for label in ["Shift+F10", "Ctrl+Shift+C", "Cmd+R", "Ctrl+Shift+S, Ctrl+R", "Shift+Ctrl+C"] {
        assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Web, label), None, "{label}");
    }
    for label in ["F8", "F9", "Ctrl+Shift+R", "Ctrl+Shift+Z, Ctrl+Y"] {
        assert_eq!(shortcuts::menu_shortcut_label(KeyPlatform::Web, label), Some(label.to_string()), "{label}");
    }
}
