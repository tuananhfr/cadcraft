//! Stable-id metadata for every engine command: the contract workspaces, palettes and saved
//! layouts rely on. Derived from `CommandSpec`, so the command definitions stay the single source.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use serde::Serialize;
use serde_json::{Value, json};

use super::{CommandSpec, command_specs};

/// What a command may do to the user's data or machine; drives permission prompts and policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskClass {
    Read,
    Edit,
    File,
    Execute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandStatus {
    Available,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Desktop,
    Web,
}

impl Target {
    pub fn current() -> Self {
        if cfg!(target_arch = "wasm32") { Self::Web } else { Self::Desktop }
    }
}

pub const COMMAND_VERSION: &str = "1.0.0";

/// Pin an existing namespace here before moving its command to another menu category.
pub const NAMESPACE_PINS: &[(&str, &str)] = &[];
pub const MODEL_ONLY: &[&str] = &["plotwindow"];
pub const PAPER_ONLY: &[&str] = &["mspace", "pspace", "mview", "mview.polygonal", "vpclip", "vplayer"];

#[derive(Clone, Debug)]
pub struct CommandMeta {
    pub stable_id: &'static str,
    pub namespaced_id: String,
    pub label_key: &'static str,
    pub contexts: &'static [&'static str],
    pub status: CommandStatus,
    pub version: &'static str,
    pub label: &'static str,
    pub aliases: Vec<String>,
    pub category: &'static str,
    pub capabilities: &'static [&'static str],
    pub permissions: &'static [&'static str],
    pub undoable: bool,
    pub risk_class: RiskClass,
}

/// Optional engine features a command can depend on; a workspace hides commands whose capability
/// is missing instead of showing something that fails.
const CAPABILITIES: &[&str] = &["lisp", "xref", "fonts"];

/// `(old_id, new_id)`: an id is renamed here instead of being dropped, so saved layouts keep working.
pub const RENAMED: &[(&str, &str)] = &[];

// Run code or scripts supplied by the user or a file.
pub const EXECUTE_IDS: &[&str] = &["appload", "load", "lisp", "script", "vbarun"];

// Read or write files outside the open drawing, or hand it to a device or another app.
pub const FILE_IDS: &[&str] = &[
    "lisptrust",
    "open",
    "qsave",
    "saveas",
    "import",
    "export",
    #[cfg(not(target_arch = "wasm32"))]
    "lispsandbox",
    "exportpdf",
    "plot",
    "publish",
    "pdfbook",
    "ubplot",
    "ubplot.publish",
    "ubplot.folder",
    "ubplot.jobs",
    "ubplot.report",
    "xref",
    "-xref",
    "xattach",
    "xopen",
    "attach",
    "wblock",
    "recover",
    "drawingrecovery",
    "compare",
    "dataextraction",
    "eattext",
    "tableexport",
    "datalink",
    "datalinkupdate",
    "share",
    "new",
    "close",
    "closeall",
];

// Not undoable (they steer or persist the session, not the drawing's undo stack) yet change state.
pub const EDIT_IDS: &[&str] = &["undo", "u", "redo", "mredo", "bsave", "bsaveas", "bclose", "layout.set", "ubplot.approve"];

pub const LISP_IDS: &[&str] = &[
    "lisptrust",
    "appload",
    "load",
    "lisp",
    #[cfg(not(target_arch = "wasm32"))]
    "lispsandbox",
];
pub const XREF_IDS: &[&str] = &["xref", "-xref", "xattach", "xopen", "xbind"];

fn risk_of(spec: &CommandSpec) -> RiskClass {
    if EXECUTE_IDS.contains(&spec.id) {
        RiskClass::Execute
    } else if FILE_IDS.contains(&spec.id) {
        RiskClass::File
    } else if EDIT_IDS.contains(&spec.id) || spec.undoable {
        RiskClass::Edit
    } else {
        RiskClass::Read
    }
}

fn permissions_of(risk: RiskClass) -> &'static [&'static str] {
    match risk {
        RiskClass::Read => &[],
        RiskClass::Edit => &["cad.edit"],
        RiskClass::File => &["cad.file"],
        RiskClass::Execute => &["cad.execute"],
    }
}

fn capabilities_of(id: &str) -> &'static [&'static str] {
    if LISP_IDS.contains(&id) {
        &["lisp"]
    } else if XREF_IDS.contains(&id) {
        &["xref"]
    } else if id == "fonts" || id.starts_with("fonts.") {
        &["fonts"]
    } else {
        &[]
    }
}

fn meta_of(spec: &CommandSpec) -> CommandMeta {
    let risk_class = risk_of(spec);
    CommandMeta {
        stable_id: spec.id,
        namespaced_id: namespaced_id(spec),
        label_key: spec.label,
        contexts: if MODEL_ONLY.contains(&spec.id) {
            &["model"]
        } else if PAPER_ONLY.contains(&spec.id) {
            &["paper"]
        } else {
            &["model", "paper"]
        },
        status: status_on(spec.id, Target::current()),
        version: COMMAND_VERSION,
        label: spec.label,
        aliases: spec.aliases.iter().map(|a| a.to_string()).collect(),
        category: spec.menu.first().copied().unwrap_or("Other"),
        capabilities: capabilities_of(spec.id),
        permissions: permissions_of(risk_class),
        undoable: spec.undoable,
        risk_class,
    }
}

/// Engine commands only; `ui-egui` adds its UI-only ids on top.
pub fn command_registry() -> Vec<CommandMeta> {
    command_specs().iter().map(meta_of).collect()
}

pub fn capability_present(id: &str) -> bool {
    CAPABILITIES.contains(&id)
}

/// Maps a possibly renamed id to the live one; `None` when no such command exists.
pub fn resolve_stable(id: &str) -> Option<&'static str> {
    let id = RENAMED.iter().find(|(old, _)| *old == id).map_or(id, |(_, new)| *new);
    command_specs().iter().find(|c| c.id == id).map(|c| c.id)
}

/// UI ids already carry a namespace; engine ids use their first menu category.
pub fn namespaced_id(spec: &CommandSpec) -> String {
    if let Some((_, pinned)) = NAMESPACE_PINS.iter().find(|(id, _)| *id == spec.id) {
        return (*pinned).to_string();
    }
    if spec.id.starts_with("ui.") {
        return spec.id.to_string();
    }
    let category = spec.menu.first().copied().unwrap_or("Other").to_ascii_lowercase();
    format!("cad.{category}.{}", spec.id)
}

pub fn status_on(id: &str, target: Target) -> CommandStatus {
    if target == Target::Web && capabilities_of(id).contains(&"xref") { CommandStatus::Unsupported } else { CommandStatus::Available }
}

/// Resolve ids independently of user aliases, which must not change saved workspace references.
pub fn resolve_command(id: &str) -> Option<&'static str> {
    static IDS: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let ids = IDS.get_or_init(|| {
        let mut ids = HashMap::new();
        for spec in command_specs() {
            ids.insert(spec.id.to_ascii_lowercase(), spec.id);
            ids.insert(namespaced_id(spec).to_ascii_lowercase(), spec.id);
        }
        ids
    });
    let id = id.to_ascii_lowercase();
    let id = RENAMED.iter().find(|(old, _)| old.eq_ignore_ascii_case(&id)).map_or(id.as_str(), |(_, new)| *new);
    ids.get(id).copied()
}

pub fn registry_json(meta: &CommandMeta) -> Value {
    let aliases: BTreeSet<String> =
        std::iter::once(meta.stable_id.to_ascii_uppercase()).chain(meta.aliases.iter().map(|a| a.to_ascii_uppercase())).collect();
    json!({
        "id": meta.namespaced_id,
        "commandId": meta.stable_id,
        "version": meta.version,
        "labelKey": meta.label_key,
        "defaultAliases": aliases,
        "contexts": meta.contexts,
        "capabilities": meta.capabilities,
        "undoable": meta.undoable,
        "status": meta.status,
    })
}

pub fn command_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Command registry entry",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "commandId", "version", "labelKey", "defaultAliases", "contexts", "capabilities", "undoable", "status"],
        "properties": {
            "id": { "type": "string", "minLength": 1 },
            "commandId": { "type": "string", "minLength": 1 },
            "version": { "type": "string", "const": COMMAND_VERSION },
            "labelKey": { "type": "string", "minLength": 1 },
            "defaultAliases": { "type": "array", "minItems": 1, "uniqueItems": true, "items": { "type": "string" } },
            "contexts": { "type": "array", "minItems": 1, "uniqueItems": true, "items": { "type": "string", "enum": ["model", "paper"] } },
            "capabilities": { "type": "array", "uniqueItems": true, "items": { "type": "string" } },
            "undoable": { "type": "boolean" },
            "status": { "type": "string", "enum": ["available", "unsupported"] },
        },
    })
}
