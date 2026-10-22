//! Ribbon layout commands use the same policy and history path as other workspace edits.

use serde_json::{Value, json};

use crate::CadApp;
use crate::wsprofile::schema::{RibbonGroup, RibbonMode, RibbonTab};

pub mod contextual;
pub mod render;

pub fn visible_tabs(app: &CadApp) -> Vec<RibbonTab> {
    let contexts = contextual::active_contexts(&app.session);
    let mut tabs: Vec<_> = app
        .ws
        .effective
        .layout
        .ribbon
        .tabs
        .iter()
        .filter(|tab| {
            tab.visible
                && tab.contextual.as_deref().is_none_or(|context| contexts.contains(&context))
                && (tab.contextual.is_none() || tab.groups.iter().any(|group| group.commands.iter().any(|id| known(id))))
        })
        .cloned()
        .collect();
    tabs.sort_by_key(|tab| tab.order);
    tabs
}

pub(crate) fn known(id: &str) -> bool {
    crate::wsprofile::apply::command_known(id)
}

fn text<'a>(params: &'a Value, field: &str) -> Result<std::borrow::Cow<'a, str>, String> {
    params
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty() && text.chars().count() <= 1024)
        .map(crate::i18n::fold::nfc)
        .ok_or_else(|| crate::tf!("Missing or invalid field: {field}", field = field))
}

fn destination(params: &Value) -> Result<usize, String> {
    params
        .get("to")
        .and_then(Value::as_u64)
        .and_then(|to| usize::try_from(to).ok())
        .ok_or_else(|| crate::tl!("to must be a whole number").to_string())
}

fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from < items.len() {
        let item = items.remove(from);
        items.insert(to.min(items.len()), item);
    }
}

fn move_button(tab: &mut RibbonTab, group_index: usize, params: &Value) -> Result<(), String> {
    let to = destination(params)?;
    let group = tab.groups.get(group_index).ok_or_else(|| crate::tl!("Ribbon group was not found").to_string())?;
    let (from, target) = if params.get("to_group").is_some() {
        let command = text(params, "command")?;
        let from = group.commands.iter().position(|id| id == command.as_ref());
        let target = text(params, "to_group")?;
        let target = tab.groups.iter().position(|group| group.id == target.as_ref());
        (from, target)
    } else {
        let from = params.get("from").and_then(Value::as_u64).and_then(|from| usize::try_from(from).ok());
        (from, Some(group_index))
    };
    let from = from.filter(|from| *from < group.commands.len()).ok_or_else(|| crate::tl!("Ribbon command was not found").to_string())?;
    let target = target.ok_or_else(|| crate::tl!("Ribbon group was not found").to_string())?;
    if target == group_index {
        if let Some(group) = tab.groups.get_mut(group_index) {
            move_item(&mut group.commands, from, to);
        }
    } else {
        let command = group.commands.get(from).ok_or_else(|| crate::tl!("Ribbon command was not found").to_string())?.clone();
        if tab.groups.get(target).is_some_and(|group| group.commands.contains(&command)) {
            return Err(crate::tl!("Ribbon command is already in the destination group").to_string());
        }
        if let Some(group) = tab.groups.get_mut(group_index) {
            group.commands.remove(from);
        }
        if let Some(group) = tab.groups.get_mut(target) {
            group.commands.insert(to.min(group.commands.len()), command);
        }
    }
    Ok(())
}

fn fresh_id<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let ids: std::collections::BTreeSet<_> = ids.collect();
    (1..=501).map(|number| format!("custom_{number}")).find(|id| !ids.contains(id.as_str())).unwrap_or_else(|| "custom".into())
}

pub(crate) fn route(app: &mut CadApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if !matches!(
        command,
        "ui.ribbon.tab.add"
            | "ui.ribbon.tab.rename"
            | "ui.ribbon.tab.remove"
            | "ui.ribbon.tab.move"
            | "ui.ribbon.group.move"
            | "ui.ribbon.group.add"
            | "ui.ribbon.group.rename"
            | "ui.ribbon.group.remove"
            | "ui.ribbon.button.add"
            | "ui.ribbon.button.remove"
            | "ui.ribbon.button.move"
            | "ui.ribbon.collapse"
            | "ui.ribbon.mode"
            | "ui.ribbon.tab.visible"
            | "ui.ribbon.reset"
    ) {
        return None;
    }
    if command == "ui.ribbon.reset" {
        return Some(crate::wsprofile::reset_ribbon(app).map(|changed| json!({"changed": changed})));
    }
    Some((|| {
        let mut ribbon = app.ws.effective.layout.ribbon.clone();
        let mut created = None;
        if command == "ui.ribbon.mode" {
            let mode = match params.get("mode").and_then(Value::as_str) {
                Some("expanded") => RibbonMode::Expanded,
                Some("compact") => RibbonMode::Compact,
                Some("tabs_only") => RibbonMode::TabsOnly,
                Some("hidden") => RibbonMode::Hidden,
                _ => return Err(crate::tl!("mode must be expanded, compact, tabs_only or hidden").to_string()),
            };
            ribbon.set_mode(mode);
        } else if command == "ui.ribbon.collapse" {
            let collapsed = params.get("collapsed").and_then(Value::as_bool).unwrap_or(ribbon.effective_mode() != RibbonMode::TabsOnly);
            ribbon.set_mode(if collapsed { RibbonMode::TabsOnly } else { RibbonMode::Expanded });
        } else if command == "ui.ribbon.tab.add" {
            let label = text(params, "label")?;
            let id = fresh_id(ribbon.tabs.iter().map(|tab| tab.id.as_str()));
            let order = ribbon
                .tabs
                .iter()
                .filter(|tab| tab.contextual.is_none())
                .map(|tab| tab.order)
                .max()
                .map_or(Some(0), |order| order.checked_add(1))
                .ok_or_else(|| crate::tl!("Ribbon tab order is full").to_string())?;
            // Display order uses `order`; reordering the stored array would touch unrelated policy locks.
            ribbon.tabs.push(RibbonTab { id: id.clone(), label: label.into(), order, ..RibbonTab::default() });
            created = Some(id);
        } else {
            let tab_id = text(params, "tab")?;
            let from =
                ribbon.tabs.iter().position(|tab| tab.id == tab_id.as_ref()).ok_or_else(|| crate::tl!("Ribbon tab was not found").to_string())?;
            if command == "ui.ribbon.tab.visible" {
                let visible = params.get("visible").and_then(Value::as_bool).ok_or_else(|| crate::tl!("visible must be a boolean").to_string())?;
                // Contextual tabs can disappear with selection, so keep a persistent tab available.
                if !visible && !ribbon.tabs.iter().any(|tab| tab.id != tab_id.as_ref() && tab.visible && tab.contextual.is_none()) {
                    return Err(crate::tl!("At least one ribbon tab must stay visible").to_string());
                }
                if let Some(tab) = ribbon.tabs.get_mut(from) {
                    tab.visible = visible;
                }
            } else if command == "ui.ribbon.tab.remove" {
                ribbon.tabs.remove(from);
            } else if command == "ui.ribbon.tab.move" {
                let mut tabs: Vec<_> = ribbon.tabs.iter().filter(|tab| tab.contextual.is_none()).cloned().collect();
                tabs.sort_by_key(|tab| tab.order);
                let orders: Vec<_> = tabs.iter().map(|tab| tab.order).collect();
                if let Some(from) = tabs.iter().position(|tab| tab.id == tab_id.as_ref()) {
                    move_item(&mut tabs, from, destination(params)?);
                    for (moved, order) in tabs.iter().zip(orders) {
                        if let Some(tab) = ribbon.tabs.iter_mut().find(|tab| tab.id == moved.id) {
                            tab.order = order;
                        }
                    }
                }
            } else {
                let tab = ribbon.tabs.get_mut(from).ok_or_else(|| crate::tl!("Ribbon tab was not found").to_string())?;
                match command {
                    "ui.ribbon.tab.rename" => tab.label = text(params, "label")?.into(),
                    "ui.ribbon.group.add" => {
                        let id = fresh_id(tab.groups.iter().map(|group| group.id.as_str()));
                        tab.groups.push(RibbonGroup { id: id.clone(), label: text(params, "label")?.into(), commands: Vec::new() });
                        created = Some(id);
                    }
                    _ => {
                        let group_id = text(params, "group")?;
                        let from = tab
                            .groups
                            .iter()
                            .position(|group| group.id == group_id.as_ref())
                            .ok_or_else(|| crate::tl!("Ribbon group was not found").to_string())?;
                        match command {
                            "ui.ribbon.button.move" => move_button(tab, from, params)?,
                            "ui.ribbon.group.move" => move_item(&mut tab.groups, from, destination(params)?),
                            "ui.ribbon.group.remove" => {
                                tab.groups.remove(from);
                            }
                            _ => {
                                let group = tab.groups.get_mut(from).ok_or_else(|| crate::tl!("Ribbon group was not found").to_string())?;
                                match command {
                                    "ui.ribbon.group.rename" => group.label = text(params, "label")?.into(),
                                    "ui.ribbon.button.add" => {
                                        let id = text(params, "id")?;
                                        if !known(id.as_ref()) {
                                            return Err(crate::tf!("Unknown command: {id}", id = id));
                                        }
                                        if !group.commands.iter().any(|held| held == id.as_ref()) {
                                            group.commands.push(id.into());
                                        }
                                    }
                                    _ => {
                                        let id = text(params, "id")?;
                                        group.commands.retain(|held| held != id.as_ref());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let changed = crate::wsprofile::edit_layout(app, |layout| {
            layout.ribbon = ribbon;
            Ok(())
        })?;
        Ok(json!({"changed": changed, "id": created}))
    })())
}
