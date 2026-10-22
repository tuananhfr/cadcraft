//! Field-name conversion at the persistence boundary; user data remains opaque.

use serde_json::{Map, Value};

pub const KNOWN_KEYS: &[&str] = &[
    "schema_version",
    "profile_id",
    "tenant_id",
    "owner_type",
    "owner_id",
    "name",
    "base_id",
    "revision",
    "etag",
    "status",
    "locale",
    "theme",
    "created_at",
    "updated_at",
    "layout",
    "layout_patch",
    "active",
    "profiles",
    "user",
    "device",
    "checksum",
    "ui_scale",
    "screen",
    "monitor_count",
    "work_area",
    "pixel_ratio",
    "window_mode",
    "ribbon",
    "visible",
    "collapsed",
    "tabs",
    "id",
    "label",
    "order",
    "contextual",
    "groups",
    "commands",
    "toolbars",
    "dock",
    "row",
    "locked",
    "x_ratio",
    "y_ratio",
    "panels",
    "mode",
    "side",
    "tab_group",
    "width_ratio",
    "height_ratio",
    "dock_tree",
    "command_line",
    "lines",
    "quick_access",
    "status_bar",
    "items",
    "shortcuts",
    "aliases",
    "favorites",
    "toolsets",
    "tab",
    "collapsed_groups",
    "hidden_tools",
    "view",
    "file_tabs",
    "viewcube",
    "ucs_icon",
    "layer_list",
    "properties_all",
    "in_window_menu",
    "quick_access_position",
    "density",
    "layers",
    "level",
    "patch",
    "locks",
    "ribbonMode",
    "quickAccessPosition",
];

const RENAMED: &[(&str, &str)] = &[
    ("schema_version", "schemaVersion"),
    ("profile_id", "profileId"),
    ("tenant_id", "tenantId"),
    ("owner_type", "scope"),
    ("owner_id", "ownerId"),
    ("base_id", "baseId"),
    ("created_at", "createdAt"),
    ("updated_at", "updatedAt"),
    ("layout_patch", "layoutPatch"),
    ("device", "deviceGeometry"),
    ("ui_scale", "uiScale"),
    ("monitor_count", "monitorCount"),
    ("work_area", "workArea"),
    ("pixel_ratio", "pixelRatio"),
    ("window_mode", "windowMode"),
    ("x_ratio", "xRatio"),
    ("y_ratio", "yRatio"),
    ("tab_group", "tabGroup"),
    ("width_ratio", "widthRatio"),
    ("height_ratio", "heightRatio"),
    ("dock_tree", "dockTree"),
    ("command_line", "commandLine"),
    ("quick_access", "quickAccess"),
    ("status_bar", "statusBar"),
    ("collapsed_groups", "collapsedGroups"),
    ("hidden_tools", "hiddenTools"),
    ("file_tabs", "fileTabs"),
    ("ucs_icon", "ucsIcon"),
    ("layer_list", "layerList"),
    ("properties_all", "propertiesAll"),
    ("in_window_menu", "inWindowMenu"),
    ("quick_access_position", "quickAccessPosition"),
];

fn field_name(key: &str, parent: &str, wire: bool) -> String {
    if parent == "ribbon" && key == if wire { "mode" } else { "ribbonMode" } {
        return if wire { "ribbonMode" } else { "mode" }.into();
    }
    RENAMED
        .iter()
        .find_map(|(snake, camel)| (key == if wire { *snake } else { *camel }).then_some(if wire { *camel } else { *snake }))
        .unwrap_or(key)
        .into()
}

fn opaque(key: &str) -> bool {
    matches!(key, "shortcuts" | "aliases" | "items" | "dock_tree")
}

fn pointer(text: &str, wire: bool) -> String {
    let mut parent = String::new();
    let mut data = false;
    text.split('/')
        .map(|part| {
            let id = matches!(parent.as_str(), "panels" | "toolbars" | "tabs" | "groups");
            let internal = if id { String::new() } else { field_name(part, &parent, false) };
            let result = if data || id { part.into() } else { field_name(part, &parent, wire) };
            data |= opaque(&internal);
            parent = internal;
            result
        })
        .collect::<Vec<String>>()
        .join("/")
}

fn convert(value: &Value, parent: &str, wire: bool, depth: usize) -> Value {
    if depth > super::validate::MAX_DEPTH {
        return value.clone();
    }
    match value {
        Value::Object(fields) => {
            let mut out = Map::new();
            for (key, child) in fields {
                let internal = if wire { key.clone() } else { field_name(key, parent, false) };
                if wire && internal == "ui_scale" && parent != "device" {
                    continue;
                }
                let known = KNOWN_KEYS.contains(&internal.as_str());
                let renamed = if known { field_name(key, parent, wire) } else { key.clone() };
                let child = if !known || opaque(&internal) {
                    child.clone()
                } else if internal == "locks" {
                    match child.as_array() {
                        Some(items) => Value::Array(
                            items
                                .iter()
                                .map(|item| item.as_str().map(|text| Value::String(pointer(text, wire))).unwrap_or_else(|| item.clone()))
                                .collect(),
                        ),
                        None => child.clone(),
                    }
                } else {
                    convert(child, &internal, wire, depth + 1)
                };
                out.insert(renamed, child);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|item| convert(item, parent, wire, depth + 1)).collect()),
        _ => value.clone(),
    }
}

pub fn to_wire(set: &Value) -> Value {
    convert(set, "", true, 0)
}

pub fn from_wire(value: &Value) -> Value {
    convert(value, "", false, 0)
}

pub fn is_wire(value: &Value) -> bool {
    value.get("schemaVersion").is_some()
}
