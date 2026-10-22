use serde::{Deserialize, Serialize};

pub const REGISTRY_SCHEMA: &str = "2";
pub(crate) const MAX_REGISTRY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_TEMPLATES: usize = 1000;
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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TemplateStatus {
    Draft,
    #[default]
    Approved,
    Superseded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateSource {
    pub file_name: String,
    pub sha256: String,
    pub imported_ms: u64,
}

fn initial_version() -> u32 {
    1
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
    #[serde(default = "initial_version")]
    pub version: u32,
    #[serde(default)]
    pub status: TemplateStatus,
    #[serde(default)]
    pub source: Option<TemplateSource>,
}

impl FrameTemplate {
    pub fn versioned_id(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.id.contains('@') || self.version == 0 {
            return Err("template id must be nonempty without @, and version must be positive".into());
        }
        if self.block_names.len() > MAX_BLOCK_NAMES {
            return Err(format!("template exceeds {MAX_BLOCK_NAMES} block names"));
        }
        for rule in &self.geometry {
            if let GeometryRule::LocalRect { min, max } = rule {
                let [min_x, min_y] = *min;
                let [max_x, max_y] = *max;
                if ![min_x, min_y, max_x, max_y].into_iter().all(f64::is_finite) || min_x >= max_x || min_y >= max_y {
                    return Err(format!("template {} local rectangle must be finite with min < max on both axes", self.id));
                }
            }
        }
        if !self.media_mm.iter().flatten().all(|dimension| dimension.is_finite() && *dimension > 0.0) {
            return Err(format!("template {} media dimensions must be finite and positive", self.id));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrameRegistry {
    pub schema_version: String,
    pub templates: Vec<FrameTemplate>,
}

impl FrameRegistry {
    pub fn builtin() -> FrameRegistry {
        Self {
            schema_version: REGISTRY_SCHEMA.into(),
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
                version: 1,
                status: TemplateStatus::Approved,
                source: None,
            }],
        }
    }

    pub fn from_json(text: &str) -> Result<FrameRegistry, String> {
        super::registry_store::parse_registry(text, false).map(|(registry, _)| registry)
    }

    pub fn merged_with(&self, imported: FrameRegistry) -> FrameRegistry {
        let mut templates: Vec<FrameTemplate> = Vec::new();
        let mut generic = None;
        for template in self.templates.iter().take(MAX_TEMPLATES).cloned().chain(imported.templates.into_iter().take(MAX_TEMPLATES)) {
            if template.id == "generic" && template.version == 1 {
                generic = Some(template);
            } else if let Some(existing) = templates.iter_mut().find(|existing| existing.id == template.id && existing.version == template.version) {
                *existing = template;
            } else {
                templates.push(template);
            }
        }
        templates.extend(generic);
        Self { schema_version: REGISTRY_SCHEMA.into(), templates }
    }

    pub fn find(&self, effective_name: &str) -> Option<&FrameTemplate> {
        let name = effective_name.to_lowercase();
        let matches = |template: &FrameTemplate| {
            template.status == TemplateStatus::Approved
                && template.id != "generic"
                && template.block_names.iter().take(MAX_BLOCK_NAMES).any(|pattern| match pattern.strip_suffix('*') {
                    Some(prefix) => name.starts_with(&prefix.to_lowercase()),
                    None => name == pattern.to_lowercase(),
                })
        };
        let specific = self.templates.iter().take(MAX_TEMPLATES).find(|template| matches(template));
        specific
            .and_then(|specific| {
                self.templates.iter().take(MAX_TEMPLATES).filter(|template| template.id == specific.id && matches(template)).max_by_key(|t| t.version)
            })
            .or_else(|| {
                self.templates
                    .iter()
                    .take(MAX_TEMPLATES)
                    .filter(|template| template.id == "generic" && template.status == TemplateStatus::Approved)
                    .max_by_key(|t| t.version)
            })
    }
}
