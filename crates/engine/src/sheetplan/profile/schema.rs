use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sheetplan::PlotScale;
use crate::sheetplan::queue::{DEFAULT_RETRY_LIMIT, OutputMode, OutputSpec};

pub const PRINT_PROFILE_SCHEMA: u32 = 1;
pub const MAX_PROFILE_BYTES: usize = 256 * 1024;
pub const MAX_PROFILE_DEPTH: usize = 32;
pub const MAX_PROFILE_LOCKS: usize = 256;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProfileLevel {
    Platform,
    Company,
    Project,
    SheetSet,
    Drawing,
    User,
}

impl fmt::Display for ProfileLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Platform => "platform",
            Self::Company => "company",
            Self::Project => "project",
            Self::SheetSet => "sheet set",
            Self::Drawing => "drawing",
            Self::User => "user",
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SheetOrder {
    Pick,
    RowsTopDown,
    SheetId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrintSettings {
    pub device: String,
    pub printer: Option<String>,
    pub plot_style_table: String,
    pub paper: Option<String>,
    pub landscape: Option<bool>,
    pub plot_scale: Option<PlotScale>,
    pub order: SheetOrder,
    pub frame_templates: Vec<String>,
    pub naming: String,
    pub output_mode: OutputMode,
    pub pages_per_file: u32,
    pub pdf_layers: bool,
    pub text_as_geometry: bool,
    pub lineweights: bool,
    pub retry_limit: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProfileDoc {
    pub schema_version: u32,
    pub level: ProfileLevel,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub locks: Vec<String>,
    pub updated_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EffectiveProfile {
    pub settings: PrintSettings,
    pub locked: BTreeMap<String, ProfileLevel>,
    pub origin: BTreeMap<String, ProfileLevel>,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Locked by {level} policy")]
pub struct ProfileLocked {
    pub pointer: String,
    pub level: ProfileLevel,
}

pub fn platform_defaults() -> PrintSettings {
    let output = OutputSpec::default();
    PrintSettings {
        device: "CADCraft PDF".into(),
        printer: None,
        plot_style_table: "None".into(),
        paper: None,
        landscape: None,
        plot_scale: None,
        order: SheetOrder::Pick,
        frame_templates: Vec::new(),
        naming: "{stem}".into(),
        output_mode: OutputMode::Merged,
        pages_per_file: output.pages_per_file,
        pdf_layers: true,
        text_as_geometry: false,
        lineweights: true,
        retry_limit: DEFAULT_RETRY_LIMIT,
    }
}
