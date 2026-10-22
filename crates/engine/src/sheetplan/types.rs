use std::collections::BTreeMap;

use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

pub const SHEETPLAN_SCHEMA: &str = "1.0";
pub const MAX_SCAN_FRAMES: usize = 10_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetPlan {
    pub schema_version: String,
    pub job_id: String,
    pub source_hash: String,
    pub sheets: Vec<Sheet>,
    #[serde(default)]
    pub issues: Vec<PreflightIssue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub at_ms: u64,
    pub by: String,
    pub action: String,
    pub sheet_key: String,
    pub code: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sheet {
    pub sheet_id: String,
    pub order: u32,
    pub mode: SheetMode,
    pub source: SheetSource,
    #[serde(default)]
    pub frame_template_id: Option<String>,
    #[serde(serialize_with = "serialize_paper_mm")]
    pub paper_mm: [f64; 2],
    pub landscape: bool,
    pub plot_scale: PlotScale,
    #[serde(default)]
    pub scale_label: Option<String>,
    #[serde(default)]
    pub paper_name: Option<String>,
    pub status: SheetStatus,
    #[serde(default)]
    pub issues: Vec<PreflightIssue>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SheetMode {
    Block,
    Window,
    Layout,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SheetSource {
    Block { block_handle: String, effective_name: String, outline: [[f64; 2]; 4], rotation_deg: f64 },
    Window { outline: [[f64; 2]; 4], rotation_deg: f64 },
    Layout { name: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlotScale {
    Fit,
    Ratio { paper_mm: f64, drawing_units: f64 },
}

impl PlotScale {
    pub fn parse(text: &str) -> Option<PlotScale> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("fit") {
            return Some(Self::Fit);
        }
        let (paper, drawing) = text.split_once([':', '/'])?;
        let paper_mm = paper.trim().parse::<f64>().ok()?;
        let drawing_units = drawing.trim().parse::<f64>().ok()?;
        Self::ratio(paper_mm, drawing_units)
    }

    /// Real-world lengths must be converted independently of the drawing's paper-space convention.
    pub fn from_real_ratio(text: &str, unit_mm: f64) -> Option<PlotScale> {
        if !positive_finite(unit_mm) {
            return None;
        }
        match Self::parse(text)? {
            Self::Fit => Some(Self::Fit),
            Self::Ratio { paper_mm, drawing_units } => {
                let drawing_units = drawing_units / unit_mm;
                if !positive_finite(paper_mm / drawing_units) {
                    return None;
                }
                Self::ratio(paper_mm, drawing_units)
            }
        }
    }

    fn ratio(paper_mm: f64, drawing_units: f64) -> Option<Self> {
        (positive_finite(paper_mm) && positive_finite(drawing_units)).then_some(Self::Ratio { paper_mm, drawing_units })
    }
}

impl Serialize for PlotScale {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Fit => serializer.serialize_str("fit"),
            Self::Ratio { paper_mm, drawing_units } if positive_finite(*paper_mm) && positive_finite(*drawing_units) => {
                serializer.serialize_str(&format!("{paper_mm}:{drawing_units}"))
            }
            Self::Ratio { .. } => Err(serde::ser::Error::custom("plot scale terms must be finite and positive")),
        }
    }
}

impl<'de> Deserialize<'de> for PlotScale {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Ratio {
            paper_mm: f64,
            drawing_units: f64,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Text(String),
            Object { ratio: Ratio },
        }
        let scale = match Input::deserialize(deserializer)? {
            Input::Text(text) => Self::parse(&text),
            Input::Object { ratio } => Self::ratio(ratio.paper_mm, ratio.drawing_units),
        };
        scale.ok_or_else(|| serde::de::Error::custom("plot scale must be fit or a finite positive ratio"))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SheetStatus {
    NeedsPreview,
    Ready,
    Blocked,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Pass,
    Info,
    Warn,
    Block,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PreflightIssue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub sheet_id: Option<String>,
    #[serde(default)]
    pub approved_by_user: bool,
}

#[derive(Deserialize)]
struct PlanHeader {
    schema_version: String,
    job_id: String,
    #[serde(default)]
    source_hash: String,
    #[serde(default)]
    issues: Vec<PreflightIssue>,
}

#[derive(Deserialize)]
struct SheetInput {
    sheet_id: String,
    order: u32,
    mode: SheetMode,
    #[serde(default)]
    source: Option<SheetSource>,
    #[serde(default)]
    block_handle: Option<String>,
    #[serde(default)]
    frame_template_id: Option<String>,
    paper_mm: [f64; 2],
    #[serde(default)]
    landscape: Option<bool>,
    plot_scale: PlotScale,
    #[serde(default)]
    scale_label: Option<String>,
    #[serde(default)]
    paper_name: Option<String>,
    status: SheetStatus,
    #[serde(default)]
    issues: Vec<PreflightIssue>,
    #[serde(default)]
    attributes: BTreeMap<String, String>,
}

pub fn parse_plan(v: &Value) -> Result<SheetPlan, String> {
    parse(v).map_err(|message| format!("PLAN_INVALID: {message}"))
}

fn parse(v: &Value) -> Result<SheetPlan, String> {
    let input = v.get("sheets").and_then(Value::as_array).ok_or_else(|| "sheets must be an array".to_string())?;
    if input.len() > MAX_SCAN_FRAMES {
        return Err(format!("sheet count exceeds {MAX_SCAN_FRAMES}"));
    }
    validate_input_size(v)?;
    let header = PlanHeader::deserialize(v).map_err(|error| error.to_string())?;
    if header.schema_version != SHEETPLAN_SCHEMA {
        return Err(format!("unsupported schema_version {}", header.schema_version));
    }
    let mut sheets = Vec::with_capacity(input.len());
    for value in input {
        sheets.push(parse_sheet(value)?);
    }
    sheets.sort_by_key(|sheet| sheet.order);
    let mut previous = None;
    for sheet in &sheets {
        if previous == Some(sheet.order) {
            return Err(format!("duplicate sheet order {}", sheet.order));
        }
        previous = Some(sheet.order);
    }
    Ok(SheetPlan { schema_version: header.schema_version, job_id: header.job_id, source_hash: header.source_hash, sheets, issues: header.issues })
}

pub(crate) fn validate_input_size(v: &Value) -> Result<(), String> {
    const MAX_DEPTH: usize = 64;
    const MAX_VALUES: usize = MAX_SCAN_FRAMES * 256;
    const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;

    let mut pending = vec![(v, 0_usize)];
    let mut values_left = MAX_VALUES;
    let mut text_left = MAX_TEXT_BYTES;
    // Serde buffers tagged/untagged enums recursively, even when decoding an existing Value.
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(format!("plan nesting exceeds {MAX_DEPTH}"));
        }
        values_left = values_left.checked_sub(1).ok_or_else(|| "plan contains too many values".to_string())?;
        let children = match value {
            Value::Array(values) => values.len(),
            Value::Object(values) => values.len(),
            _ => 0,
        };
        if children > values_left.saturating_sub(pending.len()) {
            return Err("plan contains too many values".into());
        }
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth.saturating_add(1)))),
            Value::Object(values) => {
                for (key, value) in values {
                    text_left = text_left.checked_sub(key.len()).ok_or_else(|| "plan text exceeds 64 MiB".to_string())?;
                    pending.push((value, depth.saturating_add(1)));
                }
            }
            Value::String(text) => {
                text_left = text_left.checked_sub(text.len()).ok_or_else(|| "plan text exceeds 64 MiB".to_string())?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_sheet(v: &Value) -> Result<Sheet, String> {
    let input = SheetInput::deserialize(v).map_err(|error| error.to_string())?;
    if input.order == 0 {
        return Err("sheet order must be at least 1".into());
    }
    if !input.paper_mm.into_iter().all(positive_finite) {
        return Err("paper_mm dimensions must be finite and positive".into());
    }
    let [width, height] = input.paper_mm;
    let (mut source, status) = match input.source {
        Some(source) => (source, input.status),
        None if input.mode == SheetMode::Block => {
            let block_handle = input.block_handle.ok_or_else(|| "block_handle is required without source".to_string())?;
            // A flat request has no trusted geometry until scan resolves the handle.
            (SheetSource::Block { block_handle, effective_name: String::new(), outline: [[0.0; 2]; 4], rotation_deg: 0.0 }, SheetStatus::NeedsPreview)
        }
        None => return Err("source is required for window and layout sheets".into()),
    };
    let source_mode = match &mut source {
        SheetSource::Block { block_handle, outline, rotation_deg, .. } => {
            if block_handle.is_empty() || !block_handle.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("block_handle must be hexadecimal".into());
            }
            block_handle.make_ascii_uppercase();
            validate_outline(outline, *rotation_deg)?;
            SheetMode::Block
        }
        SheetSource::Window { outline, rotation_deg } => {
            validate_outline(outline, *rotation_deg)?;
            SheetMode::Window
        }
        SheetSource::Layout { .. } => SheetMode::Layout,
    };
    if source_mode != input.mode {
        return Err("sheet mode must match source kind".into());
    }
    Ok(Sheet {
        sheet_id: input.sheet_id,
        order: input.order,
        mode: input.mode,
        source,
        frame_template_id: input.frame_template_id,
        paper_mm: input.paper_mm,
        landscape: input.landscape.unwrap_or(width > height),
        plot_scale: input.plot_scale,
        scale_label: input.scale_label,
        paper_name: input.paper_name,
        status,
        issues: input.issues,
        attributes: input.attributes,
    })
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn validate_outline(outline: &[[f64; 2]; 4], rotation_deg: f64) -> Result<(), String> {
    if !rotation_deg.is_finite() || !outline.iter().flatten().all(|value| value.is_finite()) {
        return Err("source outline and rotation must be finite".into());
    }
    Ok(())
}

fn serialize_paper_mm<S: Serializer>(paper_mm: &[f64; 2], serializer: S) -> Result<S::Ok, S::Error> {
    let mut tuple = serializer.serialize_tuple(2)?;
    for dimension in paper_mm {
        if !positive_finite(*dimension) {
            return Err(serde::ser::Error::custom("paper_mm dimensions must be finite and positive"));
        }
        // Integer dimensions retain the appendix's JSON form without lossy float casts.
        let number = dimension.to_string().parse::<serde_json::Number>().map_err(<S::Error as serde::ser::Error>::custom)?;
        tuple.serialize_element(&number)?;
    }
    tuple.end()
}
