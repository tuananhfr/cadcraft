//! A host's house workspace: a first run starts in it without the picker, and a stored set that
//! has not met it yet switches to it once. Saved profiles stay; the person can switch back.

use serde_json::{Value, json};

use super::presets::preset_json;
use super::startup::Fallback;
use crate::CadApp;

/// Set-level key recording which house workspace this set was already moved to.
pub const ADOPTED_KEY: &str = "house_workspace";

pub(crate) fn adopt(app: &mut CadApp) {
    let Some(id) = app.preferred_workspace else { return };
    if preset_json(id).is_none() || app.ws.safe_mode || app.ws.suspended || app.ws.store.is_none() {
        return;
    }
    // A damaged set is the user's to recover; switching now would bury the notice.
    if !matches!(app.ws.fallback, None | Some(Fallback::NoProfile)) {
        return;
    }
    if app.ws.set_extra.get(ADOPTED_KEY).and_then(Value::as_str) == Some(id) {
        return;
    }
    app.ws.first_run = false;
    app.ws.set_extra.insert(ADOPTED_KEY.into(), Value::from(id));
    let switched = if app.ws.active == id {
        app.ws.mark_changed();
        app.commit_workspace(false).map(|()| Value::Null)
    } else {
        app.run("ui.workspace.select", json!({"id": id}))
    };
    if let Err(error) = switched {
        // Try again next start rather than record a move that did not happen.
        app.ws.set_extra.remove(ADOPTED_KEY);
        log::warn!("workspace: could not switch to {id}: {error}");
    }
}
