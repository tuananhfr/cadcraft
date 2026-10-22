//! Keeps engine failure categories intact when UI callers need string errors.

use cadcraft_engine::cmd::outcome::classify;
use cadcraft_engine::cmd::registry::{CommandStatus, Target, resolve_command, status_on};
use cadcraft_engine::{EngineError, Outcome};
use serde_json::Value;

use crate::{CadApp, i18n, menus, wsprofile};

pub struct Dispatched {
    pub outcome: Outcome,
    pub result: Result<Value, String>,
}

impl CadApp {
    pub fn dispatch(&mut self, id: &str, params: Value) -> Dispatched {
        let dispatched = dispatch_command(self, id, params);
        if let Err(reason) = &dispatched.result {
            let message = i18n::with_language(self.language(), || match dispatched.outcome {
                Outcome::Unsupported | Outcome::PermissionDenied => crate::tf!("Not available: {reason}", reason = reason),
                Outcome::ValidationError => crate::tf!("Check the input: {reason}", reason = reason),
                _ => reason.clone(),
            });
            self.set_status(message);
        }
        dispatched
    }
}

fn dispatch_command(app: &mut CadApp, id: &str, params: Value) -> Dispatched {
    let command = resolve_command(id).unwrap_or(id);
    if status_on(command, Target::current()) == CommandStatus::Unsupported {
        let reason = format!("{id} is only available in the desktop app");
        app.session.echo(reason.clone());
        return Dispatched { outcome: Outcome::Unsupported, result: Err(reason) };
    }
    if let Some(result) = menus::run_ui_command(app, id, &params) {
        let outcome = match &result {
            Ok(value) if value.get("cancelled").and_then(Value::as_bool) == Some(true) => Outcome::Cancelled,
            Ok(_) => Outcome::Success,
            Err(_) => Outcome::ValidationError,
        };
        return Dispatched { outcome, result };
    }
    let aliases = cadcraft_engine::cmd::file::pgp::aliases();
    let result = if id.starts_with("ui.") { Err(EngineError::UnknownCommand(id.to_string())) } else { app.session.execute(id, &params) };
    let outcome = classify(&result);
    let result = result.map_err(|error| error.to_string());
    if let Err(error) = wsprofile::sync_aliases(app, &aliases) {
        return Dispatched { outcome: Outcome::ValidationError, result: Err(error) };
    }
    if let Ok(value) = &result
        && let Some(message) = value.get("message").and_then(Value::as_str)
    {
        for line in message.lines() {
            app.session.echo(line.to_string());
        }
    }
    if let Err(error) = &result {
        app.session.echo(error.clone());
    }
    Dispatched { outcome, result }
}
