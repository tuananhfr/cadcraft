//! Stable-id metadata for every engine command: the contract workspaces, palettes and saved
//! layouts rely on. Derived from `CommandSpec`, so the command definitions stay the single source.

use serde::Serialize;

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

#[derive(Clone, Debug)]
pub struct CommandMeta {
    pub stable_id: &'static str,
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
    "open",
    "qsave",
    "saveas",
    "import",
    "export",
    "exportpdf",
    "plot",
    "publish",
    "pdfbook",
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
pub const EDIT_IDS: &[&str] = &["undo", "u", "redo", "mredo", "bsave", "bsaveas", "bclose", "layout.set"];

pub const LISP_IDS: &[&str] = &["appload", "load", "lisp"];
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
