use std::collections::BTreeSet;

use serde_json::Value;

use super::merge::{KEYED_ARRAYS, escape_token};
use crate::CadApp;

#[derive(Clone, Debug, PartialEq)]
pub struct FieldConflict {
    pub pointer: String,
    pub mine: Value,
    pub theirs: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    KeepMine,
    UseServer,
    KeepBoth,
}

/// Keyed arrays use stable ids so moving a toolbar does not change its conflict pointer.
pub fn conflicts(base: &Value, mine: &Value, theirs: &Value) -> Vec<FieldConflict> {
    let mut out = Vec::new();
    collect(Some(base), Some(mine), Some(theirs), "", 0, &mut out);
    out
}

fn collect(base: Option<&Value>, mine: Option<&Value>, theirs: Option<&Value>, path: &str, depth: usize, out: &mut Vec<FieldConflict>) {
    if out.len() >= 500 || mine == theirs || mine == base || theirs == base {
        return;
    }
    if depth < 32 {
        if let (Some(Value::Object(mine)), Some(Value::Object(theirs))) = (mine, theirs)
            && base.is_none_or(Value::is_object)
        {
            let base = base.and_then(Value::as_object);
            let keys: BTreeSet<_> = mine.keys().chain(theirs.keys()).chain(base.into_iter().flat_map(|map| map.keys())).collect();
            for key in keys {
                collect(base.and_then(|map| map.get(key)), mine.get(key), theirs.get(key), &format!("{path}/{}", escape_token(key)), depth + 1, out);
            }
            return;
        }
        if KEYED_ARRAYS.contains(&path)
            && let (Some(Value::Array(mine)), Some(Value::Array(theirs))) = (mine, theirs)
            && base.is_none_or(Value::is_array)
        {
            let base = base.and_then(Value::as_array);
            let ids: BTreeSet<_> = mine
                .iter()
                .chain(theirs)
                .chain(base.into_iter().flatten())
                .take(1500)
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect();
            for id in ids {
                collect(
                    base.and_then(|items| find_item(items, id)),
                    find_item(mine, id),
                    find_item(theirs, id),
                    &format!("{path}/{}", escape_token(id)),
                    depth + 1,
                    out,
                );
            }
            // Reordering and a field edit commute; incompatible reorderings still need a choice.
            let order = |items: &[Value]| {
                items
                    .iter()
                    .take(500)
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .filter(|id| {
                        base.is_some_and(|items| find_item(items, id).is_some()) && find_item(mine, id).is_some() && find_item(theirs, id).is_some()
                    })
                    .map(|id| Value::String(id.to_string()))
                    .collect::<Vec<_>>()
            };
            let old = base.map(|items| order(items)).unwrap_or_default();
            let my_order = order(mine);
            let their_order = order(theirs);
            if my_order != old && their_order != old && my_order != their_order && out.len() < 500 {
                out.push(FieldConflict { pointer: path.into(), mine: Value::Array(my_order), theirs: Value::Array(their_order) });
            }
            return;
        }
    }
    out.push(FieldConflict { pointer: path.into(), mine: mine.cloned().unwrap_or(Value::Null), theirs: theirs.cloned().unwrap_or(Value::Null) });
}

fn find_item<'a>(items: &'a [Value], id: &str) -> Option<&'a Value> {
    items.iter().take(500).find(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

pub fn show_conflict_dialog(ctx: &egui::Context, app: &mut CadApp) {
    let Some(pending) = app.ws.sync.pending.as_ref() else { return };
    if pending.mine.profile_id != app.ws.user.profile_id {
        return;
    }
    let mut resolution = None;
    egui::Window::new(crate::tl!("Workspace sync conflict"))
        .id(egui::Id::new("workspace_sync_conflict"))
        .collapsible(false)
        .default_width(560.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                egui::Grid::new("workspace_conflicting_fields").striped(true).show(ui, |ui| {
                    ui.strong(crate::tl!("Field"));
                    ui.strong(crate::tl!("My value"));
                    ui.strong(crate::tl!("Server value"));
                    ui.end_row();
                    for field in &pending.fields {
                        ui.label(if field.pointer.is_empty() { crate::tl!("Whole workspace") } else { &field.pointer });
                        ui.label(field.mine.to_string());
                        ui.label(field.theirs.to_string());
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                if ui.button(crate::tl!("Keep mine")).clicked() {
                    resolution = Some(Resolution::KeepMine);
                }
                if ui.button(crate::tl!("Use server")).clicked() {
                    resolution = Some(Resolution::UseServer);
                }
                if ui.button(crate::tl!("Keep both")).clicked() {
                    resolution = Some(Resolution::KeepBoth);
                }
            });
        });
    if let Some(resolution) = resolution {
        app.resolve_conflict(resolution);
    }
}
