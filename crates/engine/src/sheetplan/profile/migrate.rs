use serde_json::Value;

use super::{MAX_PROFILE_BYTES, PRINT_PROFILE_SCHEMA, ProfileDoc};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MigrateError {
    #[error("print profile schema {0} is newer than this application")]
    Future(u64),
    #[error("invalid print profile: {0}")]
    Invalid(String),
}

pub fn migrate(mut value: Value) -> Result<ProfileDoc, MigrateError> {
    let mut remaining = MAX_PROFILE_BYTES;
    super::merge::validate_value(&value, 0, &mut remaining).map_err(MigrateError::Invalid)?;
    let fields = value.as_object_mut().ok_or_else(|| MigrateError::Invalid("expected an object".into()))?;
    match fields.get("schema_version") {
        None => {
            fields.insert("schema_version".into(), Value::from(PRINT_PROFILE_SCHEMA));
        }
        Some(version) => {
            let version = version.as_u64().ok_or_else(|| MigrateError::Invalid("schema_version must be an unsigned integer".into()))?;
            if version > u64::from(PRINT_PROFILE_SCHEMA) {
                return Err(MigrateError::Future(version));
            }
            if version != u64::from(PRINT_PROFILE_SCHEMA) {
                return Err(MigrateError::Invalid("unsupported schema_version".into()));
            }
        }
    }
    let doc: ProfileDoc = serde_json::from_value(value).map_err(|error| MigrateError::Invalid(error.to_string()))?;
    super::merge::validate_doc(&doc).map_err(MigrateError::Invalid)?;
    super::store::validate_id(doc.level, &doc.id).map_err(MigrateError::Invalid)?;
    let (_, issues) = super::effective_with_issues(std::slice::from_ref(&doc));
    if let Some(issue) = issues.first() {
        return Err(MigrateError::Invalid(issue.message.clone()));
    }
    Ok(doc)
}
