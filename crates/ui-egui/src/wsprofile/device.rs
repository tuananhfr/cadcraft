//! Screen-specific preferences stay local when a workspace is shared.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::schema::ui_scale;
use super::screen::ScreenProfile;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceState {
    #[serde(deserialize_with = "ui_scale")]
    pub ui_scale: f32,
    pub screen: Option<ScreenProfile>,
    pub layout_patch: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self { ui_scale: 1.0, screen: None, layout_patch: Value::Object(Map::new()), extra: Map::new() }
    }
}

/// Geometry must remain local because another screen may have different dimensions.
pub const GEOMETRY_KEYS: &[&str] = &["x_ratio", "y_ratio", "width_ratio", "height_ratio"];

fn take_ratios(fields: &mut Map<String, Value>) -> Map<String, Value> {
    GEOMETRY_KEYS.iter().filter_map(|key| fields.remove(*key).map(|value| ((*key).into(), value))).collect()
}

pub fn split_geometry(mut patch: Value) -> (Value, Value) {
    let mut device = Map::new();
    let Some(user) = patch.as_object_mut() else { return (patch, Value::Object(device)) };
    if let Some(tree) = user.remove("dock_tree") {
        device.insert("dock_tree".into(), tree);
    }
    if let Some(fields) = user.get_mut("command_line").and_then(Value::as_object_mut) {
        let geometry = take_ratios(fields);
        if !geometry.is_empty() {
            device.insert("command_line".into(), Value::Object(geometry));
        }
    }
    if user.get("command_line").and_then(Value::as_object).is_some_and(Map::is_empty) && device.contains_key("command_line") {
        user.remove("command_line");
    }
    for key in ["toolbars", "panels"] {
        let Some(items) = user.get_mut(key).and_then(Value::as_array_mut) else { continue };
        let mut geometry = Vec::new();
        for item in items.iter_mut().take(super::validate::MAX_ARRAY) {
            let Some(fields) = item.as_object_mut() else { continue };
            let mut ratios = take_ratios(fields);
            if !ratios.is_empty()
                && let Some(id) = fields.get("id")
            {
                ratios.insert("id".into(), id.clone());
                geometry.push(Value::Object(ratios));
            }
        }
        if !geometry.is_empty() {
            device.insert(key.into(), Value::Array(geometry));
        }
    }
    (patch, Value::Object(device))
}

pub(crate) fn portable_profile(mut value: Value) -> (Value, Vec<String>) {
    let mut ignored = Vec::new();
    let Some(root) = value.as_object_mut() else { return (value, ignored) };
    for key in ["ui_scale", "uiScale", "device", "deviceGeometry"] {
        if root.remove(key).is_some() {
            ignored.push(format!("/{key}"));
        }
    }
    for key in ["layout", "layout_patch"] {
        if let Some(layout) = root.get_mut(key) {
            let (user, device) = split_geometry(layout.take());
            ignored.extend(super::merge::changed_pointers(&device).into_iter().map(|pointer| format!("/{key}{pointer}")));
            *layout = user;
        }
    }
    let (user, device) = split_geometry(value);
    ignored.extend(super::merge::changed_pointers(&device));
    (user, ignored)
}
