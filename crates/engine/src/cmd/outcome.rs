//! Preserves command failure categories for callers that cannot inspect engine errors.

use serde::Serialize;
use serde_json::Value;

use crate::{EngineError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Cancelled,
    Unsupported,
    /// reserved for tenant permissions (deferred, master §1); nothing returns it yet
    PermissionDenied,
    ValidationError,
    EngineError,
}

pub fn classify(result: &Result<Value>) -> Outcome {
    match result {
        Ok(value) if value.get("cancelled").and_then(Value::as_bool) == Some(true) => Outcome::Cancelled,
        Ok(_) => Outcome::Success,
        Err(EngineError::UnknownCommand(_)) => Outcome::Unsupported,
        Err(EngineError::Disabled(_, _) | EngineError::BadParams { .. } | EngineError::NoDocument) => Outcome::ValidationError,
        Err(EngineError::Other(_) | EngineError::Internal(_, _)) => Outcome::EngineError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineError;
    use serde_json::json;

    #[test]
    fn every_engine_error_maps_to_one_outcome_and_never_permission_denied() {
        let cases = [
            (Ok(json!({})), Outcome::Success, "success"),
            (Ok(json!({"cancelled": true})), Outcome::Cancelled, "cancelled"),
            (Err(EngineError::UnknownCommand("nope".into())), Outcome::Unsupported, "unsupported"),
            (Err(EngineError::Disabled("line".into(), "disabled".into())), Outcome::ValidationError, "validation_error"),
            (Err(EngineError::BadParams { cmd: "line".into(), msg: "invalid points".into() }), Outcome::ValidationError, "validation_error"),
            (Err(EngineError::NoDocument), Outcome::ValidationError, "validation_error"),
            (Err(EngineError::Other("failed".into())), Outcome::EngineError, "engine_error"),
            (Err(EngineError::Internal("line".into(), "guarded failure".into())), Outcome::EngineError, "engine_error"),
        ];
        for (result, expected, serialized) in cases {
            let outcome = classify(&result);
            assert_eq!(outcome, expected, "{result:?}");
            assert_ne!(outcome, Outcome::PermissionDenied);
            assert_eq!(serde_json::to_value(outcome).ok(), Some(json!(serialized)));
        }
        for value in [json!({"cancelled": false}), json!({"cancelled": "true"}), json!(null)] {
            assert_eq!(classify(&Ok(value)), Outcome::Success);
        }
        assert_eq!(serde_json::to_value(Outcome::PermissionDenied).ok(), Some(json!("permission_denied")));
    }
}
