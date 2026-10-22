//! Profile shortcut parsing, conflict checks and platform command-key dispatch.

use std::collections::BTreeMap;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use crate::CadApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPlatform {
    Desktop,
    Web,
}

pub const CURRENT: KeyPlatform = if cfg!(target_arch = "wasm32") { KeyPlatform::Web } else { KeyPlatform::Desktop };

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

pub fn parse(s: &str) -> Option<KeyCombo> {
    let mut parts = s.split('+').map(str::trim).peekable();
    let (mut ctrl, mut shift, mut alt) = (false, false, false);
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let name = if part.eq_ignore_ascii_case("esc") || part.eq_ignore_ascii_case("escape") { "Escape".to_string() } else { part.to_string() };
            let key = Key::from_name(&name).or_else(|| Key::from_name(&name.to_ascii_uppercase()))?;
            return Some(KeyCombo { ctrl, shift, alt, key });
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" if !ctrl => ctrl = true,
            "shift" if !shift => shift = true,
            "alt" if !alt => alt = true,
            _ => return None,
        }
    }
    None
}

pub fn format(k: &KeyCombo) -> String {
    let mut out = String::new();
    if k.ctrl {
        out.push_str("Ctrl+");
    }
    if k.shift {
        out.push_str("Shift+");
    }
    if k.alt {
        out.push_str("Alt+");
    }
    out.push_str(k.key.name());
    out
}

/// Ctrl denotes the platform command key, as it did in the original dispatch table.
pub fn keyboard_shortcut(k: &KeyCombo) -> KeyboardShortcut {
    let mut modifiers = if k.ctrl { Modifiers::COMMAND } else { Modifiers::NONE };
    modifiers.shift = k.shift;
    modifiers.alt = k.alt;
    KeyboardShortcut::new(modifiers, k.key)
}

/// Commas preserve the original table's alternative keys for REDO and ISOPLANE.
pub fn defaults() -> BTreeMap<String, String> {
    defaults_for(CURRENT)
}

pub fn defaults_for(platform: KeyPlatform) -> BTreeMap<String, String> {
    [
        ("pickstyle", "Ctrl+Shift+A"),
        ("redo", "Ctrl+Shift+Z, Ctrl+Y"),
        ("u", "Ctrl+Z"),
        ("new", "Ctrl+N"),
        ("ui.open", "Ctrl+O"),
        ("ui.saveas", "Ctrl+Shift+S"),
        ("qsave", "Ctrl+S"),
        ("plot", "Ctrl+P"),
        ("ui.palette", "Ctrl+Shift+P"),
        ("selectall", "Ctrl+A"),
        ("copybase", "Ctrl+Shift+C"),
        ("copyclip", "Ctrl+C"),
        ("cutclip", "Ctrl+X"),
        ("pasteblock", "Ctrl+Shift+V"),
        ("pasteclip", "Ctrl+V"),
        ("ui.panel.properties", "Ctrl+1"),
        ("ui.toggle.toolsets", "Ctrl+3"),
        ("ui.toggle.cmdline", "Ctrl+9"),
        ("ui.fullscreen", "Ctrl+Shift+F"),
        ("ui.focusmode", "Ctrl+0"),
        ("ui.dialog.commands", "F1"),
        ("ui.toggle.history", "F2"),
        ("osnap", "F3"),
        ("isoplane", "F5, Ctrl+E"),
        ("grid", "F7"),
        ("ortho", "F8"),
        ("snap", "F9"),
        ("polar", "F10"),
        ("otrack", "F11"),
        ("dynmode", "F12"),
    ]
    .into_iter()
    .filter(|(id, keys)| platform == KeyPlatform::Desktop || (!matches!(*id, "ui.fullscreen" | "ui.focusmode") && !is_browser_reserved(keys)))
    .map(|(id, key)| (id.to_string(), key.to_string()))
    .collect()
}

pub const RESERVED: &[&str] = &["Escape", "Ctrl+Q"];

/// Keys the browser acts on before the page sees them (reload, new tab, address bar, help, devtools).
pub const BROWSER_RESERVED: &[&str] = &[
    "F1",
    "F3",
    "F5",
    "F6",
    "F7",
    "F10",
    "F11",
    "F12",
    "Ctrl+N",
    "Ctrl+1",
    "Ctrl+2",
    "Ctrl+3",
    "Ctrl+4",
    "Ctrl+5",
    "Ctrl+6",
    "Ctrl+7",
    "Ctrl+8",
    "Ctrl+9",
    "Ctrl+E",
    "Ctrl+L",
    "Ctrl+T",
    "Ctrl+W",
    "Ctrl+R",
    "Shift+F10",
    "Ctrl+Shift+C",
    "Ctrl+Shift+P",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindError {
    Reserved,
    BrowserReserved,
    Conflict { other: String },
    Unparsable,
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reserved => f.write_str(crate::tl!("Reserved key")),
            Self::BrowserReserved => f.write_str(crate::tl!("The browser uses this key")),
            Self::Conflict { other } => f.write_str(&crate::tf!("Already used by {other}", other = other)),
            Self::Unparsable => f.write_str(crate::tl!("Not a key combination")),
        }
    }
}

fn combos(keys: &str) -> impl Iterator<Item = KeyCombo> + '_ {
    keys.split(',').filter_map(|key| parse(key.trim()))
}

pub fn is_browser_reserved(key: &str) -> bool {
    combos(key).any(|key| is_browser_reserved_combo(&key))
}

fn is_browser_reserved_combo(key: &KeyCombo) -> bool {
    BROWSER_RESERVED.iter().filter_map(|s| parse(s)).any(|reserved| reserved == *key)
}

pub fn menu_shortcut_label(platform: KeyPlatform, label: &str) -> Option<String> {
    if platform == KeyPlatform::Web && is_browser_reserved(label) { None } else { Some(label.to_string()) }
}

fn is_reserved(key: &KeyCombo) -> bool {
    RESERVED.iter().filter_map(|s| parse(s)).any(|reserved| reserved == *key)
}

pub(crate) fn remove_reserved(map: &mut BTreeMap<String, String>) {
    map.retain(|_, keys| {
        if !combos(keys).any(|key| is_reserved(&key)) {
            return true;
        }
        *keys = keys.split(',').map(str::trim).filter(|key| !parse(key).is_some_and(|key| is_reserved(&key))).collect::<Vec<_>>().join(", ");
        !keys.trim().is_empty()
    });
}

pub fn bind(map: &mut BTreeMap<String, String>, command: &str, key: &str, replace: bool) -> Result<(), BindError> {
    bind_for(CURRENT, map, command, key, replace)
}

pub fn bind_for(platform: KeyPlatform, map: &mut BTreeMap<String, String>, command: &str, key: &str, replace: bool) -> Result<(), BindError> {
    let key = parse(key).ok_or(BindError::Unparsable)?;
    if is_reserved(&key) {
        return Err(BindError::Reserved);
    }
    if platform == KeyPlatform::Web && is_browser_reserved_combo(&key) {
        return Err(BindError::BrowserReserved);
    }
    if !key.ctrl
        && !key.shift
        && !key.alt
        && !matches!(
            key.key,
            Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12
        )
    {
        return Err(BindError::Unparsable);
    }
    let conflicts: Vec<String> =
        map.iter().filter(|(id, keys)| id.as_str() != command && combos(keys).any(|held| held == key)).map(|(id, _)| id.clone()).collect();
    if !replace && let Some(other) = conflicts.first() {
        return Err(BindError::Conflict { other: other.clone() });
    }
    for id in conflicts {
        let remaining = map
            .get(&id)
            .map(|keys| combos(keys).filter(|held| *held != key).map(|held| format(&held)).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        if remaining.is_empty() {
            map.remove(&id);
        } else {
            map.insert(id, remaining);
        }
    }
    map.insert(command.to_string(), format(&key));
    Ok(())
}

pub fn active(app: &CadApp) -> BTreeMap<String, String> {
    active_for(CURRENT, app)
}

pub fn active_for(platform: KeyPlatform, app: &CadApp) -> BTreeMap<String, String> {
    let mut map = defaults_for(platform);
    map.extend(app.ws.effective.layout.shortcuts.clone());
    remove_reserved(&mut map);
    if platform == KeyPlatform::Web {
        map.retain(|_, keys| !is_browser_reserved(keys));
    }
    map
}

/// UI overrides take precedence over defaults; empty strings explicitly disable a default.
pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !matches!(id, "ui.shortcut.bind" | "ui.shortcut.unbind") {
        return None;
    }
    Some((|| {
        let command =
            params.get("command").and_then(Value::as_str).filter(|id| !id.is_empty()).ok_or_else(|| crate::tl!("command is required").to_string())?;
        let before = active(app);
        let mut map = before.clone();
        if id == "ui.shortcut.bind" {
            if !crate::wsprofile::apply::command_known(command) {
                return Err(crate::tf!("Unknown command: {id}", id = command));
            }
            let key = params.get("key").and_then(Value::as_str).ok_or_else(|| crate::tl!("key is required").to_string())?;
            bind(&mut map, command, key, params.get("replace").and_then(Value::as_bool).unwrap_or(false)).map_err(|error| error.to_string())?;
        } else {
            map.remove(command);
        }
        let changed = crate::wsprofile::edit_layout(app, |layout| {
            for command in before.keys().chain(map.keys()) {
                if before.get(command) != map.get(command) {
                    layout.shortcuts.insert(command.clone(), map.get(command).cloned().unwrap_or_default());
                }
            }
            Ok(())
        })?;
        Ok(json!({"changed": changed}))
    })())
}

/// Clipboard events substitute for key events on native platforms and the web.
pub(crate) fn pressed(app: &mut CadApp, ctx: &egui::Context) -> Vec<String> {
    let map = active(app);
    let clipboard = crate::clipboard::shortcut_commands(app, ctx);
    let defaults = defaults();
    let mut fire: Vec<String> = Vec::new();
    for original in clipboard {
        if let Some(keys) = defaults.get(original) {
            for key in combos(keys) {
                for (id, keys) in &map {
                    if combos(keys).any(|held| held == key) && !fire.contains(id) {
                        fire.push(id.clone());
                    }
                }
            }
        }
    }
    let mut pairs: Vec<_> = map.iter().flat_map(|(id, keys)| combos(keys).map(move |key| (keyboard_shortcut(&key), id.clone()))).collect();
    // egui matches extra modifiers; more specific bindings must consume the event first.
    pairs.sort_by_key(|(key, _)| std::cmp::Reverse(u8::from(key.modifiers.shift) + u8::from(key.modifiers.alt) + u8::from(key.modifiers.command)));
    ctx.input_mut(|input| {
        for (key, id) in pairs {
            if input.consume_shortcut(&key) && !fire.contains(&id) {
                fire.push(id);
            }
        }
    });
    fire
}
