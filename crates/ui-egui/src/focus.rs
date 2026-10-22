use egui::{Align2, Context, Id, Order, vec2};
use serde_json::{Value, json};

use crate::CadApp;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandLineInFocus {
    #[default]
    Floating,
    Hidden,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    active: bool,
    pub command_line: CommandLineInFocus,
}

impl State {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn enter(&mut self) {
        self.active = true;
    }

    pub fn exit(&mut self) {
        self.active = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    Ribbon,
    Panels,
    Toolbars,
    FileTabs,
    StatusBar,
    MenuBar,
    QuickAccess,
}

pub fn hides(app: &CadApp, _part: Chrome) -> bool {
    app.focus.active()
}

pub fn exit_button(ctx: &Context, app: &mut CadApp) {
    if !app.focus.active() {
        return;
    }
    egui::Area::new(Id::new("cc_exit_focus"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, 8.0))
        .constrain_to(ctx.content_rect())
        .movable(false)
        .show(ctx, |ui| {
            // Symmetric clearance keeps the centred notice away from the ViewCube on narrow screens.
            ui.set_max_width((ctx.content_rect().width() - 352.0).clamp(1.0, 320.0));
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if ui.button(crate::tl!("Exit Focus Mode (Esc)")).clicked() {
                    let _ = app.run("ui.focusmode", json!({"on": false}));
                }
                if app.fullscreen.fallback_focus
                    && let Some((message, _)) = &app.status
                {
                    ui.add(egui::Label::new(message).wrap());
                }
            });
        });
}

pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let on = match id {
        "cleanscreenon" => true,
        "cleanscreenoff" => false,
        "ui.focusmode" => match params.get("on") {
            Some(Value::Bool(on)) => *on,
            Some(_) => return Some(Err(crate::tl!("on must be a boolean").to_string())),
            None => !app.focus.active(),
        },
        _ => return None,
    };
    let command_line = match params.get("command_line") {
        None => app.focus.command_line,
        Some(Value::String(value)) if value == "floating" => CommandLineInFocus::Floating,
        Some(Value::String(value)) if value == "hidden" => CommandLineInFocus::Hidden,
        Some(_) => return Some(Err(crate::tl!("command_line must be floating or hidden").to_string())),
    };
    app.focus.command_line = command_line;
    if on {
        app.focus.enter();
    } else {
        app.focus.exit();
        app.fullscreen.fallback_focus = false;
    }
    Some(Ok(json!({"on": app.focus.active()})))
}

pub(crate) fn escape(app: &mut CadApp, ctx: &Context) {
    // Options closes on Esc without touching the command; other dialogs are non-modal, so Esc still cancels drawing.
    if app.ui.dialog.as_deref().is_some_and(|name| name == "options" || name.starts_with("options:")) {
        app.ui.dialog = None;
        return;
    }
    if !app.focus.active() && app.fullscreen.phase == crate::fullscreen::Phase::Off {
        app.cmd.buffer.clear();
        app.session.cancel();
        return;
    }
    if app.ui.dialog.is_some() || app.closing.is_some() || app.opening.busy() || app.hatch_dlg.is_some() {
        return;
    }
    let busy = !app.cmd.buffer.is_empty()
        || app.session.prompting()
        || app.session.lisp_busy()
        || app.session.pending_window.is_some()
        || !app.session.selection().is_empty();
    if busy {
        app.cmd.buffer.clear();
        app.session.cancel();
    } else if app.focus.active() {
        let _ = app.run("ui.focusmode", json!({"on": false}));
    } else if app.services.fullscreen.is_none() && app.fullscreen.phase != crate::fullscreen::Phase::Off {
        let _ = crate::fullscreen::toggle(app, ctx, false);
    } else {
        app.session.cancel();
    }
}
