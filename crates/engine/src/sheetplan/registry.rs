use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::SHEETPLAN_SCHEMA;

const MAX_REGISTRY_BYTES: usize = 256 * 1024;
const MAX_TEMPLATES: usize = 500;
const MAX_BLOCK_NAMES: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeometryRule {
    LargestClosedRect,
    LayerRect { layer: String },
    LocalRect { min: [f64; 2], max: [f64; 2] },
    EnclosingRect,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AttributeTags {
    #[serde(default)]
    pub sheet_id: Vec<String>,
    #[serde(default)]
    pub revision: Vec<String>,
    #[serde(default)]
    pub title: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrameTemplate {
    pub id: String,
    pub revision: String,
    pub block_names: Vec<String>,
    pub geometry: Vec<GeometryRule>,
    #[serde(default)]
    pub attributes: AttributeTags,
    #[serde(default)]
    pub media_mm: Vec<[f64; 2]>,
    #[serde(default)]
    pub superseded_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrameRegistry {
    pub schema_version: String,
    pub templates: Vec<FrameTemplate>,
}

impl FrameRegistry {
    pub fn builtin() -> FrameRegistry {
        Self {
            schema_version: SHEETPLAN_SCHEMA.into(),
            templates: vec![FrameTemplate {
                id: "generic".into(),
                revision: "1".into(),
                block_names: vec!["*".into()],
                geometry: vec![GeometryRule::LargestClosedRect, GeometryRule::EnclosingRect],
                attributes: AttributeTags {
                    sheet_id: ["SHEET_ID", "SHEET_NO", "SHEETNO", "DWG_NO", "DRAWING_NO"].map(String::from).into(),
                    revision: ["REV", "REVISION"].map(String::from).into(),
                    title: vec!["TITLE".into()],
                },
                media_mm: Vec::new(),
                superseded_by: None,
            }],
        }
    }

    pub fn from_json(text: &str) -> Result<FrameRegistry, String> {
        if text.len() > MAX_REGISTRY_BYTES {
            return Err("registry import exceeds 256 KiB".into());
        }
        // Parsing Value first bounds nesting before serde buffers internally tagged geometry rules.
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let templates = value.get("templates").and_then(Value::as_array).ok_or_else(|| "templates must be an array".to_string())?;
        if templates.len() > MAX_TEMPLATES {
            return Err(format!("registry import exceeds {MAX_TEMPLATES} templates"));
        }
        for template in templates {
            if template.get("block_names").and_then(Value::as_array).is_some_and(|names| names.len() > MAX_BLOCK_NAMES) {
                return Err(format!("template exceeds {MAX_BLOCK_NAMES} block names"));
            }
        }
        let registry = Self::deserialize(&value).map_err(|error| error.to_string())?;
        if registry.schema_version != SHEETPLAN_SCHEMA {
            return Err(format!("unsupported registry schema_version {}", registry.schema_version));
        }
        for template in &registry.templates {
            for rule in &template.geometry {
                if let GeometryRule::LocalRect { min, max } = rule {
                    let [min_x, min_y] = *min;
                    let [max_x, max_y] = *max;
                    if ![min_x, min_y, max_x, max_y].into_iter().all(f64::is_finite) || min_x >= max_x || min_y >= max_y {
                        return Err(format!("template {} local rectangle must be finite with min < max on both axes", template.id));
                    }
                }
            }
            if !template.media_mm.iter().flatten().all(|dimension| dimension.is_finite() && *dimension > 0.0) {
                return Err(format!("template {} media dimensions must be finite and positive", template.id));
            }
        }
        Ok(registry)
    }

    pub fn merged_with(&self, imported: FrameRegistry) -> FrameRegistry {
        let mut templates: Vec<FrameTemplate> = Vec::new();
        let mut generic = None;
        for template in self.templates.iter().cloned().chain(imported.templates) {
            if template.id == "generic" {
                generic = Some(template);
            } else if let Some(existing) = templates.iter_mut().find(|existing| existing.id == template.id) {
                *existing = template;
            } else {
                templates.push(template);
            }
        }
        templates.extend(generic);
        Self { schema_version: self.schema_version.clone(), templates }
    }

    pub fn find(&self, effective_name: &str) -> Option<&FrameTemplate> {
        let name = effective_name.to_lowercase();
        self.templates
            .iter()
            .find(|template| {
                template.id != "generic"
                    && template.block_names.iter().any(|pattern| match pattern.strip_suffix('*') {
                        Some(prefix) => name.starts_with(&prefix.to_lowercase()),
                        None => name == pattern.to_lowercase(),
                    })
            })
            .or_else(|| self.templates.iter().find(|template| template.id == "generic"))
    }
}
