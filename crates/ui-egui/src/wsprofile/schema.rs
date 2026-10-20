//! The workspace profile document: serde types for schema 1.0.0.

use std::collections::BTreeMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

pub const SCHEMA_VERSION: &str = "1.0.0";

/// Profile layers from the most general to the most specific.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerType {
    #[default]
    Platform,
    Product,
    Tenant,
    Company,
    Discipline,
    Project,
    User,
    Session,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Draft,
    #[default]
    Published,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Floating,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelMode {
    #[default]
    Dock,
    Float,
    #[serde(alias = "autohide")]
    AutoHide,
    Hidden,
}

fn clamp_finite(value: f32, low: f32, high: f32) -> f32 {
    if value.is_nan() { low } else { value.clamp(low, high) }
}

fn unit_ratio<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    Ok(clamp_finite(f32::deserialize(deserializer)?, 0.0, 1.0))
}

fn ui_scale<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    Ok(clamp_finite(f32::deserialize(deserializer)?, 0.75, 2.0))
}

/// The command line docks to the top or bottom edge or floats; a side dock is not a layout it can take.
fn command_line_dock<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Dock, D::Error> {
    Ok(match Dock::deserialize(deserializer)? {
        Dock::Left | Dock::Right => Dock::Bottom,
        other => other,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RibbonGroup {
    pub id: String,
    pub label: String,
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RibbonTab {
    pub id: String,
    pub label: String,
    pub order: u32,
    pub visible: bool,
    /// Selection kind that makes the tab appear ("dimension", "hatch"); `None` for a standing tab.
    pub contextual: Option<String>,
    pub groups: Vec<RibbonGroup>,
}

impl Default for RibbonTab {
    fn default() -> Self {
        Self { id: String::new(), label: String::new(), order: 0, visible: true, contextual: None, groups: Vec::new() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ribbon {
    pub visible: bool,
    pub collapsed: bool,
    pub tabs: Vec<RibbonTab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarState {
    pub id: String,
    pub label: String,
    pub dock: Dock,
    pub order: u32,
    pub row: u32,
    pub visible: bool,
    pub locked: bool,
    #[serde(deserialize_with = "unit_ratio")]
    pub x_ratio: f32,
    #[serde(deserialize_with = "unit_ratio")]
    pub y_ratio: f32,
    pub commands: Vec<String>,
}

impl Default for ToolbarState {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            dock: Dock::Top,
            order: 0,
            row: 0,
            visible: true,
            locked: false,
            x_ratio: 0.0,
            y_ratio: 0.0,
            commands: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelState {
    pub id: String,
    pub mode: PanelMode,
    pub side: Option<Dock>,
    pub tab_group: Option<String>,
    pub visible: bool,
    #[serde(deserialize_with = "unit_ratio")]
    pub x_ratio: f32,
    #[serde(deserialize_with = "unit_ratio")]
    pub y_ratio: f32,
    #[serde(deserialize_with = "unit_ratio")]
    pub width_ratio: f32,
    #[serde(deserialize_with = "unit_ratio")]
    pub height_ratio: f32,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            id: String::new(),
            mode: PanelMode::Dock,
            side: None,
            tab_group: None,
            visible: true,
            x_ratio: 0.0,
            y_ratio: 0.0,
            width_ratio: 0.2,
            height_ratio: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandLineState {
    /// Spec samples spell this `side`.
    #[serde(alias = "side", deserialize_with = "command_line_dock")]
    pub dock: Dock,
    #[serde(deserialize_with = "unit_ratio")]
    pub height_ratio: f32,
    pub lines: u32,
    pub visible: bool,
    #[serde(deserialize_with = "unit_ratio")]
    pub x_ratio: f32,
    #[serde(deserialize_with = "unit_ratio")]
    pub y_ratio: f32,
}

impl Default for CommandLineState {
    fn default() -> Self {
        Self { dock: Dock::Bottom, height_ratio: 0.1, lines: 3, visible: true, x_ratio: 0.0, y_ratio: 0.0 }
    }
}

/// Status bar toggles the shipped bar offers, keyed by the name the layout stores them under.
pub const STATUS_BAR_ITEMS: &[&str] = &[
    "annotation",
    "coordinates",
    "dynmode",
    "grid",
    "isodraft",
    "layouts",
    "lineweight",
    "ortho",
    "osnap",
    "otrack",
    "polar",
    "quickprops",
    "settings",
    "snap",
    "transparency",
    "workspace",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarState {
    pub visible: bool,
    pub items: BTreeMap<String, bool>,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self { visible: true, items: STATUS_BAR_ITEMS.iter().map(|item| ((*item).to_string(), true)).collect() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSetsState {
    pub visible: bool,
    pub tab: String,
    pub collapsed_groups: Vec<String>,
    pub hidden_tools: Vec<String>,
}

impl Default for ToolSetsState {
    fn default() -> Self {
        Self { visible: true, tab: "Drafting".into(), collapsed_groups: Vec::new(), hidden_tools: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceLayout {
    pub ribbon: Ribbon,
    pub toolbars: Vec<ToolbarState>,
    pub panels: Vec<PanelState>,
    /// The craft-ui dock layout, opaque to everything except the code that applies it.
    pub dock_tree: Option<Value>,
    pub command_line: CommandLineState,
    pub quick_access: Vec<String>,
    pub status_bar: StatusBarState,
    pub shortcuts: BTreeMap<String, String>,
    pub aliases: BTreeMap<String, String>,
    pub favorites: Vec<String>,
    pub toolsets: ToolSetsState,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Command ids of the shipped tool bar (`chrome::TOOLBAR`), in display order.
const STANDARD_TOOLBAR: &[&str] = &[
    "new",
    "open",
    "qsave",
    "saveas",
    "u",
    "redo",
    "plot",
    "publish",
    "pagesetup",
    "preview",
    "import",
    "export",
    "attach",
    "share",
    "zoom.window",
    "pan",
    "3dorbit",
    "zoom.extents",
    "ui.toggle.palettes",
    "ui.dialog.layers",
    "ui.dialog.blocks",
    "dist",
    "list",
    "area",
    "ui.dialog.about",
];

/// Written out in code, not read from the SIMPLE preset: serde calls `Default` while parsing, so a
/// default that parsed JSON would recurse. A test keeps this equal to the preset.
impl Default for WorkspaceLayout {
    fn default() -> Self {
        let docked = |id: &str, side: Dock, width_ratio: f32, height_ratio: f32| PanelState {
            id: id.into(),
            side: Some(side),
            width_ratio,
            height_ratio,
            ..PanelState::default()
        };
        Self {
            ribbon: Ribbon::default(),
            toolbars: vec![ToolbarState {
                id: "standard".into(),
                label: "Standard".into(),
                commands: STANDARD_TOOLBAR.iter().map(|id| (*id).to_string()).collect(),
                ..ToolbarState::default()
            }],
            panels: vec![
                docked("toolSets", Dock::Left, 0.15, 1.0),
                docked("layers", Dock::Right, 0.21, 0.4),
                docked("properties", Dock::Right, 0.21, 0.6),
                PanelState { mode: PanelMode::Hidden, visible: false, ..docked("externalReferences", Dock::Right, 0.21, 0.5) },
            ],
            dock_tree: None,
            command_line: CommandLineState::default(),
            quick_access: Vec::new(),
            status_bar: StatusBarState::default(),
            shortcuts: BTreeMap::new(),
            aliases: BTreeMap::new(),
            favorites: Vec::new(),
            toolsets: ToolSetsState::default(),
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, remote = "Self")]
pub struct WorkspaceProfile {
    pub schema_version: String,
    pub profile_id: String,
    pub tenant_id: Option<String>,
    pub owner_type: OwnerType,
    pub owner_id: Option<String>,
    pub name: String,
    pub base_id: Option<String>,
    pub revision: u64,
    pub etag: Option<String>,
    pub status: ProfileStatus,
    pub locale: Option<String>,
    pub theme: Option<String>,
    #[serde(deserialize_with = "ui_scale")]
    pub ui_scale: f32,
    /// RFC 3339.
    pub created_at: String,
    /// RFC 3339.
    pub updated_at: String,
    pub layout: WorkspaceLayout,
    /// Fields this version does not know, kept verbatim so a newer writer's data survives a save.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub const PRESET_TIMESTAMP: &str = "2026-10-21T00:00:00Z";

impl Default for WorkspaceProfile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.into(),
            profile_id: "preset.SIMPLE".into(),
            tenant_id: None,
            owner_type: OwnerType::Platform,
            owner_id: None,
            name: "Simple".into(),
            base_id: None,
            revision: 1,
            etag: None,
            status: ProfileStatus::Published,
            locale: None,
            theme: None,
            ui_scale: 1.0,
            created_at: PRESET_TIMESTAMP.into(),
            updated_at: PRESET_TIMESTAMP.into(),
            layout: WorkspaceLayout::default(),
            extra: Map::new(),
        }
    }
}

/// Spec-style profiles keep these at the root; the model keeps them in `layout`.
const ROOT_LAYOUT_KEYS: [&str; 3] = ["panels", "command_line", "shortcuts"];

/// Moves root-level layout keys into `layout` unless `layout` already has its own.
fn hoist_root_layout_keys(value: &mut Value) {
    let Some(root) = value.as_object_mut() else { return };
    for key in ROOT_LAYOUT_KEYS {
        let Some(moved) = root.remove(key) else { continue };
        let layout_is_free = match root.get("layout") {
            None => true,
            Some(Value::Object(layout)) => !layout.contains_key(key),
            Some(_) => false,
        };
        if !layout_is_free {
            root.insert(key.to_string(), moved);
            continue;
        }
        if let Some(layout) = root.entry("layout").or_insert_with(|| Value::Object(Map::new())).as_object_mut() {
            layout.insert(key.to_string(), moved);
        }
    }
}

// `remote = "Self"` turns the derives into inherent functions, so the root-key hoisting can run
// before the generated field-by-field code without a second copy of the struct.
impl Serialize for WorkspaceProfile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WorkspaceProfile::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for WorkspaceProfile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        hoist_root_layout_keys(&mut value);
        WorkspaceProfile::deserialize(value).map_err(D::Error::custom)
    }
}
