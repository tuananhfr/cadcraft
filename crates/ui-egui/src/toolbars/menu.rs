//! The tool bar context menu, the Window ▸ Toolbars items, and the key that opens the menu.

use egui::{Id, Key, Modifiers};
use serde_json::{Value, json};

use super::{BUILTIN, HIDE_ITEM, SHOW_ITEM, render};
use crate::CadApp;
use crate::menus::Entry;
use crate::wsprofile::merge::{check, escape_token};
use crate::wsprofile::schema::{Dock, ToolbarState};

const MENU_KEY: &str = "cc_toolbar_menu_key";

/// Takes Shift+F10 while any control of a bar has the keyboard focus, so it opens that bar's menu
/// instead of reaching the F10 shortcut (Polar Tracking), which egui matches whatever Shift does.
pub(crate) fn claim_menu_key(ctx: &egui::Context) {
    let Some(focused) = ctx.memory(|memory| memory.focused()) else { return };
    let Some(owner) = render::last(ctx).stops.iter().find(|(stop, _)| *stop == focused).map(|(_, owner)| *owner) else { return };
    if ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::F10)) {
        ctx.data_mut(|data| data.insert_temp(Id::new(MENU_KEY), Some(owner)));
    }
}

/// Whether the menu key was pressed for the bar `owner`.
pub(super) fn take_menu_key(ctx: &egui::Context, owner: Id) -> bool {
    ctx.data_mut(|data| {
        let pressed = data.get_temp::<Option<Id>>(Id::new(MENU_KEY)).flatten() == Some(owner);
        if pressed {
            data.insert_temp(Id::new(MENU_KEY), None::<Id>);
        }
        pressed
    })
}

/// The right-click / grip menu of `bar`; a chosen item runs as the matching `ui.toolbar.*` command.
/// `focus_first` moves the keyboard focus onto the first item, for a menu that was opened by keyboard.
pub(super) fn context_menu(
    app: &mut CadApp,
    bar: &ToolbarState,
    policy_locked: bool,
    anchor: &egui::Response,
    open: Option<egui::SetOpenCommand>,
    focus_first: bool,
) {
    let mut chosen: Option<(&'static str, Value)> = None;
    let movable = !policy_locked && !bar.locked;
    let id = bar.id.as_str();
    egui::Popup::menu(anchor).id(anchor.id.with("menu")).open_memory(open).show(|ui| {
        let hide = ui.add_enabled(!policy_locked, egui::Button::new(crate::tl!("Hide")));
        if focus_first && !policy_locked {
            hide.request_focus();
        }
        if hide.clicked() {
            chosen = Some(("ui.toolbar.hide", json!({"id": id})));
            ui.close();
        }
        if policy_locked {
            hide.on_disabled_hover_text(crate::tl!("Locked by company policy"));
        }
        let (label, command) = if bar.locked { (crate::tl!("Unlock"), "ui.toolbar.unlock") } else { (crate::tl!("Lock"), "ui.toolbar.lock") };
        let toggle = ui.add_enabled(!policy_locked, egui::Button::new(label));
        if toggle.clicked() {
            chosen = Some((command, json!({"id": id})));
            ui.close();
        }
        let dock_menu = ui
            .add_enabled_ui(movable, |ui| {
                ui.menu_button(crate::tl!("Dock"), |ui| {
                    let edges = [
                        (crate::tl!("Top"), Dock::Top, "top"),
                        (crate::tl!("Bottom"), Dock::Bottom, "bottom"),
                        (crate::tl!("Left"), Dock::Left, "left"),
                        (crate::tl!("Right"), Dock::Right, "right"),
                    ];
                    for (label, dock, name) in edges {
                        if ui.add_enabled(bar.dock != dock, egui::Button::new(label)).clicked() {
                            chosen = Some(("ui.toolbar.dock", json!({"id": id, "dock": name})));
                            ui.close();
                        }
                    }
                })
                .response
            })
            .inner;
        let float = ui.add_enabled(movable && bar.dock != Dock::Floating, egui::Button::new(crate::tl!("Float")));
        if float.clicked() {
            chosen = Some(("ui.toolbar.float", json!({"id": id})));
            ui.close();
        }
        if policy_locked {
            for item in [toggle, dock_menu, float] {
                item.on_disabled_hover_text(crate::tl!("Locked by company policy"));
            }
        }
    });
    if let Some((command, params)) = chosen
        && let Err(error) = app.run(command, params)
    {
        app.set_status(error);
    }
}

/// Window ▸ Toolbars: one item per tool bar, shipped ones the profile does not hold included.
pub(crate) fn menu_entries(app: &CadApp) -> Vec<Entry> {
    let held = &app.ws.applied.toolbars;
    let item = |id: &str, label: &str, shown: bool| {
        let editable = check(&app.ws.effective, &format!("/toolbars/{}", escape_token(id))).is_ok();
        let prefix = if shown { HIDE_ITEM } else { SHOW_ITEM };
        Entry::Item { label: label.to_string(), id: format!("{prefix}{id}"), shortcut: None, enabled: editable }
    };
    let mut entries: Vec<Entry> = held
        .iter()
        .map(|bar| {
            let shown = if bar.id == "standard" { app.ui.show_toolbar } else { bar.visible };
            item(&bar.id, if bar.label.is_empty() { &bar.id } else { &bar.label }, shown)
        })
        .collect();
    entries.extend(BUILTIN.iter().filter(|(id, _)| !held.iter().any(|bar| bar.id == *id)).map(|(id, label)| item(id, label, false)));
    entries
}

/// `Some(checked)` for a Window ▸ Toolbars item.
pub(crate) fn menu_check_state(command: &str) -> Option<bool> {
    if command.starts_with(HIDE_ITEM) {
        Some(true)
    } else if command.starts_with(SHOW_ITEM) {
        Some(false)
    } else {
        None
    }
}
