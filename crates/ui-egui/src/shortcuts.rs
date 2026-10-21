//! Profile shortcut parsing, conflict checks and platform command-key dispatch.

use std::collections::BTreeMap;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use crate::CadApp;

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
    [
        ("pickstyle", "Ctrl+Shift+A"),
        ("redo", "Ctrl+Shift+Z, Ctrl+Y"),
        ("u", "Ctrl+Z"),
        ("new", "Ctrl+N"),
        ("ui.open", "Ctrl+O"),
        ("ui.saveas", "Ctrl+Shift+S"),
        ("qsave", "Ctrl+S"),
        ("plot", "Ctrl+P"),
        ("selectall", "Ctrl+A"),
        ("copybase", "Ctrl+Shift+C"),
        ("copyclip", "Ctrl+C"),
        ("cutclip", "Ctrl+X"),
        ("pasteblock", "Ctrl+Shift+V"),
        ("pasteclip", "Ctrl+V"),
        ("ui.panel.properties", "Ctrl+1"),
        ("ui.toggle.toolsets", "Ctrl+3"),
        ("ui.toggle.cmdline", "Ctrl+9"),
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
    .map(|(id, key)| (id.to_string(), key.to_string()))
    .collect()
}

pub const RESERVED: &[&str] = &["Escape", "Ctrl+Q"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindError {
    Reserved,
    Conflict { other: String },
    Unparsable,
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reserved => f.write_str(crate::tl!("Reserved key")),
            Self::Conflict { other } => f.write_str(&crate::tf!("Already used by {other}", other = other)),
            Self::Unparsable => f.write_str(crate::tl!("Not a key combination")),
        }
    }
}

fn combos(keys: &str) -> impl Iterator<Item = KeyCombo> + '_ {
    keys.split(',').filter_map(|key| parse(key.trim()))
}

pub fn bind(map: &mut BTreeMap<String, String>, command: &str, key: &str, replace: bool) -> Result<(), BindError> {
    let key = parse(key).ok_or(BindError::Unparsable)?;
    if RESERVED.iter().filter_map(|s| parse(s)).any(|reserved| reserved == key) {
        return Err(BindError::Reserved);
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
    let mut map = defaults();
    map.extend(app.ws.effective.layout.shortcuts.clone());
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
