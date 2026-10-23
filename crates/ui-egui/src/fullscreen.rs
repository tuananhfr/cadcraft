use cadcraft_engine::Outcome;
use egui::{Context, ViewportCommand};
use serde_json::json;

use crate::{CadApp, dispatch::Dispatched};

pub mod escape;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Off,
    Requested,
    On,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CancelledRequest {
    Waiting,
    Exiting,
}

#[derive(Default)]
pub struct State {
    pub phase: Phase,
    pub fallback_focus: bool,
    pub(crate) escape: Option<escape::Snapshot>,
    cancelled_request: Option<CancelledRequest>,
}

pub trait FullscreenHost {
    fn request(&self) -> Result<(), String>;
    fn exit(&self);
    fn is_active(&self) -> bool;
}

pub fn toggle(app: &mut CadApp, ctx: &Context, from_user_gesture: bool) -> Outcome {
    let host_active = app.services.fullscreen.as_ref().map(|host| host.is_active());
    if host_active == Some(false) && !from_user_gesture {
        let message = crate::i18n::with_language(app.language(), || crate::tl!("Full screen needs a click in the browser").to_string());
        app.set_status(message);
        return Outcome::Unsupported;
    }
    if host_active == Some(true) || app.fullscreen.phase != Phase::Off {
        if let Some(host) = &app.services.fullscreen {
            if app.fullscreen.phase == Phase::Requested {
                let cancellation = if host_active == Some(true) { CancelledRequest::Exiting } else { CancelledRequest::Waiting };
                app.fullscreen.cancelled_request = Some(cancellation);
            }
            host.exit();
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        app.fullscreen.phase = Phase::Off;
        app.fullscreen.fallback_focus = false;
    } else {
        app.fullscreen.cancelled_request = None;
        app.fullscreen.fallback_focus = false;
        if let Some(host) = &app.services.fullscreen {
            if host.request().is_err() {
                rejected(app);
                return Outcome::Success;
            }
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
        }
        app.fullscreen.phase = Phase::Requested;
    }
    ctx.request_repaint();
    Outcome::Success
}

// Browser errors arrive after request() returns, so the host also calls this from its event queue.
pub fn rejected(app: &mut CadApp) {
    app.fullscreen.cancelled_request = None;
    app.fullscreen.phase = Phase::Off;
    app.fullscreen.fallback_focus = true;
    app.focus.enter();
    let message = crate::i18n::with_language(app.language(), || {
        crate::tl!("Full screen is blocked here (iframe needs allow=\"fullscreen\"); using Focus mode").to_string()
    });
    app.set_status(message);
}

/// A host event confirms completion; an inactive poll may precede a cancelled request's late entry.
pub fn host_inactive(app: &mut CadApp) {
    app.fullscreen.cancelled_request = None;
    if app.fullscreen.phase == Phase::Requested {
        rejected(app);
    }
}

pub fn sync(app: &mut CadApp, ctx: &Context) {
    let active = match &app.services.fullscreen {
        Some(host) => Some(host.is_active()),
        None => ctx.input(|input| input.viewport().fullscreen),
    };
    match (app.fullscreen.cancelled_request, active) {
        (Some(_), Some(true)) => {
            if let Some(host) = &app.services.fullscreen {
                host.exit();
            }
            app.fullscreen.cancelled_request = Some(CancelledRequest::Exiting);
            app.fullscreen.phase = Phase::Off;
            ctx.request_repaint();
            return;
        }
        (Some(CancelledRequest::Exiting), Some(false)) => app.fullscreen.cancelled_request = None,
        _ => {}
    }
    match (app.fullscreen.phase, active) {
        (_, Some(true)) => app.fullscreen.phase = Phase::On,
        (Phase::On, Some(false)) => app.fullscreen.phase = Phase::Off,
        _ => {}
    }
}

pub(crate) fn dispatch(app: &mut CadApp, from_user_gesture: bool) -> Dispatched {
    let ctx = app.command_context.clone();
    let outcome = toggle(app, &ctx, from_user_gesture);
    let result = if outcome == Outcome::Unsupported {
        Err(crate::i18n::tr(app.language(), "Full screen needs a click in the browser").to_string())
    } else {
        Ok(json!({"fullscreen": app.fullscreen.phase != Phase::Off, "fallback_focus": app.fullscreen.fallback_focus}))
    };
    Dispatched { outcome, result }
}
