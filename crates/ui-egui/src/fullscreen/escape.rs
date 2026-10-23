//! Browser Escape routing uses the state before egui dismisses a focused widget or popup.

use egui::{Context, Event, Key, Modifiers, RawInput};

use crate::CadApp;

#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    fullscreen: bool,
    modal: bool,
    dialog: Option<String>,
    popup: bool,
    command_options: bool,
    text_focus: bool,
    command_buffer: bool,
    command_prompt: bool,
    active_command: bool,
    drag: bool,
    selection: bool,
    grips: bool,
    focus_mode: bool,
    opening: bool,
    pending_input: bool,
}

fn should_exit_fullscreen(before: &Snapshot) -> bool {
    before.fullscreen
        && !before.modal
        && before.dialog.is_none()
        && !before.popup
        && !before.command_options
        && !before.text_focus
        && !before.opening
        && !before.command_buffer
        && !before.command_prompt
        && !before.active_command
        && !before.drag
        && !before.selection
        && !before.grips
        && !before.focus_mode
        && !before.pending_input
}

/// Called by the web host before egui processes this frame's input.
pub fn capture(app: &mut CadApp, ctx: &Context, raw: &RawInput) {
    app.fullscreen.escape = None;
    let fullscreen = app.services.fullscreen.as_ref().is_some_and(|host| host.is_active());
    if !fullscreen || !raw.events.iter().any(|event| matches!(event, Event::Key { key: Key::Escape, pressed: true, .. })) {
        return;
    }
    let pending_input = raw.events.iter().any(|event| {
        matches!(event, Event::Text(_) | Event::Paste(_) | Event::Ime(_) | Event::PointerButton { pressed: true, .. })
            || matches!(event, Event::Key { key, pressed: true, .. } if *key != Key::Escape)
    });
    app.fullscreen.escape = Some(Snapshot {
        fullscreen,
        modal: ctx.memory(|memory| memory.top_modal_layer().is_some()) || app.closing.is_some() || !app.recovery.items.is_empty(),
        dialog: app.ui.dialog.clone(),
        popup: ctx.any_popup_open() || app.canvas.dyn_menu.is_some() || app.autohide.active.is_some(),
        command_options: app.cmd.options.is_some(),
        text_focus: ctx.text_edit_focused() || app.cmd.ime_composing(),
        command_buffer: !app.cmd.buffer.is_empty(),
        command_prompt: app.session.prompting() || app.session.pending_window.is_some(),
        active_command: app.session.running.is_some() || app.session.lisp_busy(),
        drag: ctx.dragged_id().is_some()
            || ctx.input(|input| input.pointer.any_down())
            || !matches!(app.ws.toolbar_drag, crate::toolbars::DragPhase::Idle),
        selection: !app.session.selection().is_empty(),
        grips: !app.canvas.hot_grips.is_empty() || app.canvas.dyn_hot.is_some(),
        focus_mode: app.focus.active(),
        opening: app.opening.busy(),
        pending_input,
    });
}

/// Exit only for an already idle app; busy states keep their existing Escape handlers.
pub fn handle(app: &mut CadApp, ctx: &Context) {
    if app.fullscreen.escape.as_ref().is_some_and(should_exit_fullscreen) && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        let _ = super::toggle(app, ctx, false);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use cadcraft_engine::Session;

    use super::*;
    use crate::{Services, fullscreen::FullscreenHost};

    fn idle() -> Snapshot {
        Snapshot { fullscreen: true, ..Snapshot::default() }
    }

    #[test]
    fn idle_exits_only_when_fullscreen() {
        assert!(should_exit_fullscreen(&idle()));
        assert!(!should_exit_fullscreen(&Snapshot::default()));
    }

    #[test]
    fn each_busy_layer_prevents_exit() {
        for before in [
            Snapshot { modal: true, ..idle() },
            Snapshot { dialog: Some("about".into()), ..idle() },
            Snapshot { popup: true, ..idle() },
            Snapshot { command_options: true, ..idle() },
            Snapshot { text_focus: true, ..idle() },
            Snapshot { command_buffer: true, ..idle() },
            Snapshot { command_prompt: true, ..idle() },
            Snapshot { active_command: true, ..idle() },
            Snapshot { drag: true, ..idle() },
            Snapshot { selection: true, ..idle() },
            Snapshot { grips: true, ..idle() },
            Snapshot { focus_mode: true, ..idle() },
            Snapshot { opening: true, ..idle() },
            Snapshot { pending_input: true, ..idle() },
        ] {
            assert!(!should_exit_fullscreen(&before), "{before:?}");
        }
    }

    struct Host(Rc<Cell<bool>>);

    impl FullscreenHost for Host {
        fn request(&self) -> Result<(), String> {
            self.0.set(true);
            Ok(())
        }

        fn exit(&self) {
            self.0.set(false);
        }

        fn is_active(&self) -> bool {
            self.0.get()
        }
    }

    fn app() -> (CadApp, Rc<Cell<bool>>) {
        let active = Rc::new(Cell::new(true));
        let services = Services { fullscreen: Some(Box::new(Host(Rc::clone(&active)))), ..Services::default() };
        (CadApp::new(Session::new(), services), active)
    }

    fn escape_input() -> RawInput {
        RawInput {
            events: vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            ..RawInput::default()
        }
    }

    fn frame(app: &mut CadApp, ctx: &Context) {
        let raw = escape_input();
        capture(app, ctx, &raw);
        let _ = ctx.run_ui(raw, |ui| {
            handle(app, ui.ctx());
            crate::cmdline::keyboard(app, ui.ctx());
        });
    }

    #[test]
    fn cancelling_a_command_requires_another_escape_to_exit() {
        let (mut app, active) = app();
        app.start("line");
        assert!(app.session.prompting());
        let ctx = Context::default();
        frame(&mut app, &ctx);
        assert!(!app.session.prompting());
        assert!(active.get());
        frame(&mut app, &ctx);
        assert!(!active.get());
    }

    #[test]
    fn typed_fullscreen_exits_without_a_gesture_but_enter_still_needs_one() {
        let (mut app, active) = app();
        assert!(app.cmdline("ui.fullscreen").is_none());
        assert!(!active.get());
        assert!(app.cmdline("ui.fullscreen").is_some());
        assert!(!active.get());
        assert_eq!(app.fullscreen.phase, super::super::Phase::Off);
    }
}
