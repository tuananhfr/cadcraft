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
            let message = i18n::with_language(self.language(), || failure_message(dispatched.outcome, reason));
            self.set_status(message);
        }
        dispatched
    }
}

fn failure_message(outcome: Outcome, reason: &str) -> String {
    match outcome {
        Outcome::Unsupported | Outcome::PermissionDenied => crate::tf!("Not available: {reason}", reason = i18n::echo(reason)),
        Outcome::ValidationError => crate::tf!("Check the input: {reason}", reason = i18n::echo(reason)),
        _ => i18n::echo(reason).into_owned(),
    }
}

fn dispatch_command(app: &mut CadApp, id: &str, params: Value) -> Dispatched {
    let command = resolve_command(id).unwrap_or(id);
    if status_on(command, Target::current()) == CommandStatus::Unsupported {
        let reason = crate::tf!("{id} is only available in the desktop app", id = id);
        app.session.echo(reason.clone());
        return Dispatched { outcome: Outcome::Unsupported, result: Err(reason) };
    }
    if id.eq_ignore_ascii_case("ui.fullscreen") {
        return crate::fullscreen::dispatch(app, false);
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
    if matches!(command, "qsave" | "saveas")
        && let Ok(saved) = &result
    {
        app.record_drawing_save(saved);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_status_translates_catalog_reasons_and_keeps_unknown_data() {
        i18n::with_language("vi", || {
            assert_eq!(failure_message(Outcome::ValidationError, "Requires a positive radius."), "Kiểm tra dữ liệu nhập: Cần nhập bán kính dương.");
            assert_eq!(failure_message(Outcome::Unsupported, "No frames selected"), "Không khả dụng: Chưa chọn khung bản vẽ nào");
            assert_eq!(failure_message(Outcome::ValidationError, "User data {reason}"), "Kiểm tra dữ liệu nhập: User data {reason}");
            assert_eq!(
                failure_message(Outcome::ValidationError, "Block \"Office\" not found."),
                "Kiểm tra dữ liệu nhập: Không tìm thấy khối \"Office\"."
            );
        });
        i18n::with_language("en", || {
            assert_eq!(failure_message(Outcome::ValidationError, "Requires a positive radius."), "Check the input: Requires a positive radius.");
        });
    }

    #[test]
    fn dispatch_keeps_english_results_errors_and_log_under_vietnamese() {
        let mut app = CadApp::new(cadcraft_engine::Session::new(), crate::Services::default());
        let result = i18n::with_language("vi", || dispatch_command(&mut app, "regen", serde_json::json!({})));
        assert_eq!(result.result.unwrap(), serde_json::json!({"message": "Regenerating model."}));
        assert_eq!(app.session.log.last().map(String::as_str), Some("Regenerating model."));

        let params = serde_json::json!({"center": [0, 0], "radius": -1});
        let expected = cadcraft_engine::Session::new().execute("circle", &params).map_err(|error| error.to_string());
        assert!(expected.is_err());
        let result = i18n::with_language("vi", || dispatch_command(&mut app, "circle", params));
        assert_eq!(result.result, expected);
        assert_eq!(app.session.log.last().map(String::as_str), expected.as_ref().err().map(String::as_str));
    }
}
