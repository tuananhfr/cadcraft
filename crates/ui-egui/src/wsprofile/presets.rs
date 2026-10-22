//! The nine workspace presets shipped inside the binary.

use super::schema::WorkspaceProfile;

pub const PRESET_IDS: &[&str] =
    &["SIMPLE", "AUTOCAD_FAMILIAR", "PLANNING_GIS", "ARCHITECTURE", "STRUCTURE", "MEP_FIRE", "INFRA", "QS_QA", "SHEET_PUBLISH"];

/// What startup falls back to when a profile cannot be used.
pub const SAFE_PRESET: &str = "SIMPLE";

/// The embedded JSON text of a preset.
pub fn preset_json(id: &str) -> Option<&'static str> {
    Some(match id {
        "SIMPLE" => include_str!("presets/SIMPLE.json"),
        "AUTOCAD_FAMILIAR" => include_str!("presets/AUTOCAD_FAMILIAR.json"),
        "PLANNING_GIS" => include_str!("presets/PLANNING_GIS.json"),
        "ARCHITECTURE" => include_str!("presets/ARCHITECTURE.json"),
        "STRUCTURE" => include_str!("presets/STRUCTURE.json"),
        "MEP_FIRE" => include_str!("presets/MEP_FIRE.json"),
        "INFRA" => include_str!("presets/INFRA.json"),
        "QS_QA" => include_str!("presets/QS_QA.json"),
        "SHEET_PUBLISH" => include_str!("presets/SHEET_PUBLISH.json"),
        _ => return None,
    })
}

pub fn preset(id: &str) -> Option<WorkspaceProfile> {
    let mut profile: WorkspaceProfile = serde_json::from_str(preset_json(id)?).ok()?;
    profile.schema_version = super::schema::SCHEMA_VERSION;
    Some(profile)
}
