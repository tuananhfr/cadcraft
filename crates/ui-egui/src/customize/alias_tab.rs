use std::collections::{BTreeMap, BTreeSet};

use cadcraft_engine::{command_specs, find_command};
use serde_json::{Value, json};

use super::{edit, run};
use crate::wsprofile::{self, schema::WorkspaceLayout};
use crate::{CadApp, FileKind};

const MAX_BYTES: usize = 256 * 1024;
const MAX_ALIASES: usize = cadcraft_engine::cmd::file::pgp::MAX_ALIASES;
const ALIAS_FILE: FileKind = FileKind { label: "Aliases (JSON)", extensions: &["json"] };

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasIssue {
    NotAscii,
    Empty,
    TooLong,
    IsCommandName { command: &'static str },
    OverridesBuiltin { command: &'static str },
    DuplicateUser { command: String },
}

impl AliasIssue {
    fn needs_confirmation(&self) -> bool {
        matches!(self, Self::OverridesBuiltin { .. } | Self::DuplicateUser { .. })
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotAscii => crate::tl!("Use letters A–Z, digits 0–9, _ or - for aliases").into(),
            Self::Empty => crate::tl!("Alias cannot be empty").into(),
            Self::TooLong => crate::tl!("Alias must be at most 16 characters").into(),
            Self::IsCommandName { command } => crate::tf!("{command} is a command name and cannot be an alias", command = command),
            Self::OverridesBuiltin { command } => crate::tf!("Alias already runs {command}; confirm to replace", command = command),
            Self::DuplicateUser { command } => crate::tf!("Alias already runs {command}; confirm to replace", command = command),
        }
    }
}

pub fn check_alias(layout: &WorkspaceLayout, alias: &str, command: &str) -> Vec<AliasIssue> {
    let mut issues = Vec::new();
    if alias.is_empty() {
        issues.push(AliasIssue::Empty);
    }
    if alias.len() > 16 {
        issues.push(AliasIssue::TooLong);
    }
    if !alias.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) {
        issues.push(AliasIssue::NotAscii);
    }
    if !issues.is_empty() {
        return issues;
    }
    if let Some(spec) = find_command(alias) {
        issues.push(AliasIssue::IsCommandName { command: spec.id });
    }
    if let Some((id, _, _, _)) = crate::menus::UI_COMMANDS.iter().find(|entry| entry.0.eq_ignore_ascii_case(alias)) {
        issues.push(AliasIssue::IsCommandName { command: id });
    }
    let target = find_command(command).map_or(command, |spec| spec.id);
    if let Some(spec) = command_specs().iter().find(|spec| spec.aliases.iter().any(|held| held.eq_ignore_ascii_case(alias)))
        && !spec.id.eq_ignore_ascii_case(target)
    {
        issues.push(AliasIssue::OverridesBuiltin { command: spec.id });
    }
    if let Some((_, held)) = layout.aliases.iter().find(|(held, _)| held.eq_ignore_ascii_case(alias))
        && !held.eq_ignore_ascii_case(target)
    {
        issues.push(AliasIssue::DuplicateUser { command: held.clone() });
    }
    issues
}

fn checked_command(layout: &WorkspaceLayout, alias: &str, command: &str, confirm: bool) -> Result<&'static str, String> {
    if let Some(issue) = check_alias(layout, alias, command).iter().find(|issue| !issue.needs_confirmation() || !confirm) {
        return Err(issue.message());
    }
    find_command(command).map(|spec| spec.id).ok_or_else(|| crate::tf!("Unknown command: {id}", id = command))
}

fn insert(layout: &mut WorkspaceLayout, alias: &str, command: &str) -> Result<(), String> {
    let exists = layout.aliases.keys().any(|held| held.eq_ignore_ascii_case(alias));
    if !exists && layout.aliases.len() >= MAX_ALIASES {
        return Err(crate::tl!("Too many aliases").into());
    }
    layout.aliases.retain(|held, _| !held.eq_ignore_ascii_case(alias));
    layout.aliases.insert(alias.to_ascii_uppercase(), command.into());
    Ok(())
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params.get(key).and_then(Value::as_str).ok_or_else(|| crate::tf!("{key} is required", key = key))
}

pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "ui.alias.set" => set(app, params),
        "ui.alias.remove" => remove(app, params),
        "ui.alias.reset" => wsprofile::edit_layout(app, |layout| {
            layout.aliases.clear();
            Ok(())
        })
        .map(|changed| json!({"changed": changed})),
        "ui.alias.export" => export(app, params),
        "ui.alias.import" => import(app, params),
        _ => return None,
    })
}

fn set(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let alias = required(params, "alias")?;
    let command = required(params, "command")?;
    let confirm = params.get("confirm").and_then(Value::as_bool).unwrap_or(false);
    let command = checked_command(&app.ws.effective.layout, alias, command, confirm)?;
    let changed = wsprofile::edit_layout(app, |layout| insert(layout, alias, command))?;
    Ok(json!({"alias": alias.to_ascii_uppercase(), "command": command, "changed": changed}))
}

fn remove(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let alias = required(params, "alias")?;
    let changed = wsprofile::edit_layout(app, |layout| {
        layout.aliases.retain(|held, _| !held.eq_ignore_ascii_case(alias));
        Ok(())
    })?;
    Ok(json!({"changed": changed}))
}

fn export(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let aliases: BTreeMap<_, _> = app.ws.effective.layout.aliases.iter().map(|(alias, command)| (alias.to_ascii_uppercase(), command)).collect();
    let text = serde_json::to_string_pretty(&json!({"aliases": aliases})).map_err(|error| error.to_string())?;
    let path = match params.get("path").and_then(Value::as_str) {
        Some(path) => Some(path.to_string()),
        None if params.is_null() => app.services.pick_save.as_ref().and_then(|pick| pick("Aliases.json")),
        None => None,
    };
    if let Some(path) = &path {
        cadcraft_engine::cmd::file::store_file("ui.alias.export", path, text.as_bytes()).map_err(|error| error.to_string())?;
    }
    Ok(json!({"text": text, "path": path}))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut text = String::new();
    file.take(MAX_BYTES as u64 + 1).read_to_string(&mut text).map_err(|error| error.to_string())?;
    Ok(text)
}

#[cfg(target_arch = "wasm32")]
fn read_file(_path: &str) -> Result<String, String> {
    Err(crate::tl!("Paste alias JSON to import in the browser").into())
}

fn import(app: &mut CadApp, params: &Value) -> Result<Value, String> {
    let text = match (params.get("text").and_then(Value::as_str), params.get("path").and_then(Value::as_str)) {
        (Some(text), _) if text.len() <= MAX_BYTES => text.to_string(),
        (Some(_), _) => return Err(crate::tl!("Alias file is too large").into()),
        (None, Some(path)) => read_file(path)?,
        (None, None) if params.is_null() => match app.services.open_file(&ALIAS_FILE) {
            Some(path) => read_file(&path)?,
            None => return Ok(json!({"cancelled": true})),
        },
        _ => return Err(crate::tl!("Paste alias JSON to import").into()),
    };
    if text.len() > MAX_BYTES {
        return Err(crate::tl!("Alias file is too large").into());
    }
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let aliases = value.get("aliases").and_then(Value::as_object).ok_or_else(|| crate::tl!("Expected an aliases object").to_string())?;
    let confirm = params.get("confirm").and_then(Value::as_bool).unwrap_or(false);
    let mut next = app.ws.effective.layout.clone();
    let mut accepted = Vec::new();
    let mut skipped = 0usize;
    for (alias, command) in aliases {
        let result = command
            .as_str()
            .ok_or_else(|| crate::tl!("Expected a command name").to_string())
            .and_then(|command| checked_command(&next, alias, command, confirm))
            .and_then(|command| {
                insert(&mut next, alias, command)?;
                Ok(command)
            });
        match result {
            Ok(command) => accepted.push((alias.clone(), command)),
            Err(_) => skipped = skipped.saturating_add(1),
        }
    }
    let changed = wsprofile::edit_layout(app, |layout| {
        for (alias, command) in &accepted {
            insert(layout, alias, command)?;
        }
        Ok(())
    })?;
    let imported = accepted.len();
    let message = crate::tf!("{imported} imported, {skipped} skipped", imported = imported, skipped = skipped);
    app.set_status(message.clone());
    Ok(json!({"changed": changed, "imported": imported, "skipped": skipped, "message": message}))
}

#[derive(Default)]
pub(super) struct State {
    alias: String,
    command: String,
    query: String,
    text: String,
    pending: Option<(String, String)>,
    notice: Option<String>,
}

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, state: &mut super::State) {
    edit(ui, &mut state.aliases.query, crate::tl!("Filter aliases"));
    let index = crate::palette::index::SearchIndex::build(app.language(), &crate::shortcuts::active(app));
    let matching: BTreeSet<_> =
        index.search(&state.aliases.query, &[], &[], 50).iter().filter_map(|hit| index.entry(hit).map(|entry| entry.id)).collect();
    let query = state.aliases.query.to_ascii_uppercase();
    let mut rows: BTreeMap<String, (String, bool)> = command_specs()
        .iter()
        .flat_map(|spec| spec.aliases.iter().map(move |alias| (alias.to_ascii_uppercase(), (spec.id.to_string(), false))))
        .collect();
    for (alias, command) in app.ws.effective.layout.aliases.iter().take(MAX_ALIASES) {
        rows.insert(alias.to_ascii_uppercase(), (command.clone(), true));
    }
    egui::ScrollArea::vertical().id_salt("aliases_table").max_height(120.0).show(ui, |ui| {
        egui::Grid::new("aliases_grid").striped(true).show(ui, |ui| {
            ui.label(crate::tl!("Alias"));
            ui.label(crate::tl!("Command"));
            ui.label(crate::tl!("Source"));
            ui.end_row();
            for (alias, (command, user)) in rows {
                if !query.is_empty() && !alias.contains(&query) && !matching.contains(command.as_str()) {
                    continue;
                }
                if super::row(ui, state.aliases.alias == alias, &alias) {
                    state.aliases.alias = alias;
                    state.aliases.command = command.clone();
                }
                let label = find_command(&command).map_or(command.as_str(), |spec| spec.label);
                ui.label(format!("{} ({command})", crate::i18n::t(label)));
                ui.label(if user { crate::tl!("User") } else { crate::tl!("Built-in") });
                ui.end_row();
            }
        });
    });
    let alias = edit(ui, &mut state.aliases.alias, crate::tl!("Alias"));
    let command = edit(ui, &mut state.aliases.command, crate::tl!("Alias command"));
    let enter = (alias.has_focus() || command.has_focus() || alias.lost_focus() || command.lost_focus())
        && !state.ime_composing
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let add = ui.button(crate::tl!("Assign Alias")).clicked();
    if add || enter {
        let alias = state.aliases.alias.clone();
        let command = state.aliases.command.clone();
        let issues = check_alias(&app.ws.effective.layout, &alias, &command);
        if !issues.is_empty() && issues.iter().all(AliasIssue::needs_confirmation) {
            state.aliases.pending = Some((alias, command));
        } else {
            let _ = run(app, "ui.alias.set", json!({"alias": alias, "command": command}), state);
        }
    }
    ui.horizontal(|ui| {
        if ui.button(crate::tl!("Remove Alias")).clicked() {
            let _ = run(app, "ui.alias.remove", json!({"alias": state.aliases.alias}), state);
        }
        if ui.button(crate::tl!("Reset Aliases")).clicked() {
            let _ = run(app, "ui.alias.reset", json!({}), state);
        }
        if ui.button(crate::tl!("Export Aliases...")).clicked() {
            let _ = run(app, "ui.alias.export", Value::Null, state);
        }
    });
    if (app.services.pick_open.is_some() || app.services.pick_open_kind.is_some())
        && ui.button(crate::tl!("Import Alias File...")).clicked()
        && let Some(result) = run(app, "ui.alias.import", Value::Null, state)
    {
        state.aliases.notice = result.get("message").and_then(Value::as_str).map(str::to_string);
    }
    ui.collapsing(crate::tl!("Import Aliases..."), |ui| {
        ui.label(crate::tl!("Paste alias JSON to import"));
        ui.add(egui::TextEdit::multiline(&mut state.aliases.text).char_limit(MAX_BYTES).desired_rows(3));
        if ui.button(crate::tl!("Import Aliases")).clicked()
            && let Some(result) = run(app, "ui.alias.import", json!({"text": state.aliases.text}), state)
        {
            state.aliases.notice = result.get("message").and_then(Value::as_str).map(str::to_string);
        }
    });
    if let Some(notice) = &state.aliases.notice {
        ui.label(notice);
    }
    if let Some((alias, command)) = state.aliases.pending.clone() {
        egui::Modal::new(egui::Id::new("confirm_alias")).show(ui.ctx(), |ui| {
            ui.label(crate::tl!("Replace Alias?"));
            for issue in check_alias(&app.ws.effective.layout, &alias, &command) {
                ui.label(issue.message());
            }
            ui.horizontal(|ui| {
                if ui.button(crate::tl!("Replace Alias")).clicked() {
                    let _ = run(app, "ui.alias.set", json!({"alias": alias, "command": command, "confirm": true}), state);
                    state.aliases.pending = None;
                }
                if ui.button(crate::tl!("Cancel")).clicked() {
                    state.aliases.pending = None;
                }
            });
        });
    }
}
