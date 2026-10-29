//! Edit gate: a host-installed policy decides whether commands that change drawing content may start.

use serde_json::Value;

use crate::cmd::{CommandSpec, str_param};
use crate::{EngineError, Result, Session};

/// Decides whether content-changing commands may run; installed by the host with [`Session::set_edit_policy`].
pub trait EditPolicy: Send + Sync {
    /// `Err(reason)` refuses the command; the reason is shown to the user.
    fn check(&self) -> std::result::Result<(), String>;
}

/// Ids that never change drawing content (files, view, plot, settings, read-only inquiry).
#[rustfmt::skip]
const VIEW_ONLY_IDS: &[&str] = &[
    "new", "open", "close", "closeall", "qsave", "saveas", "document.switch", "document.bytes", "script",
    "export", "preview", "publish", "pdfbook", "share", "3dorbit", "pagesetup", "plot", "exportpdf", "plotstyle", "stylesmanager",
    "plotwindow", "plottermanager", "-plottermanager",
    "layfrz", "layoff", "laylck", "layulk", "layon", "laythw", "layiso", "layuniso", "layerp", "layer.current", "laymcur",
    "layerstate.save", "layerstate.restore", "layerstate.list", "layerstate.delete", "layerstate.rename",
    "zoom", "pan", "regen", "regenall", "redraw", "view", "view.set", "view.get", "layout.set", "-vports", "vports.activate", "vports",
    "vports.1", "vports.2", "vports.3", "vports.4", "viewport.set", "vplayer", "mspace", "pspace", "layout.list",
    "ortho", "grid", "snap", "polar", "otrack", "dynmode", "lwdisplay", "isodraft", "isoplane", "transparencydisplay", "qpmode",
    "selectioncycling", "osnap", "dsettings", "setvar", "getvar", "sysvars",
    "audit", "recover", "drawingrecovery", "xref", "xopen",
    "options", "aliasedit", "reinit", "fonts",
    "dist", "id", "list", "area", "time", "status", "drawing.inspect", "entities", "texts", "countlist", "count", "copyclip", "copybase",
    "select", "selectall", "ai_deselect", "qselect", "qselect.info", "compare", "lispaudit", "u", "undo", "redo", "mredo",
    "compare.next", "compare.previous", "compareclose", "kcsplotter", "tableexport", "wblock",
    "measuregeom", "radius.measure", "angle.measure", "massprop", "cal", "quickcalc.pick", "properties",
    "blocks.list", "blocks.dynamic", "blocks.reference", "blocks.reference.grips", "groups.list", "style.list", "dimstyle.list",
    "hatch.patterns", "constraints.inspect", "constraints.dof", "selectsimilar",
    "isolateobjects", "hideobjects", "unisolateobjects", "annovisible.current", "annovisible.all", "annovisible.toggle", "attdisp",
    "constraintbar", "constraintbar.showall", "constraintbar.hideall", "dcdisplay", "dcdisplay.showall", "dcdisplay.hideall",
    "constraintsettings", "constraintinfer", "pickstyle", "color", "lweight", "style.current", "dimstyle.current",
    "lisptrust", "lispsandbox",
];

/// Families whose every member is view-only: `zoom.*`, `pan.*`, `ubplot*`.
fn in_view_only_family(id: &str) -> bool {
    id.starts_with("zoom.") || id.starts_with("pan.") || id.starts_with("ubplot")
}

pub(crate) fn is_view_only_id(id: &str) -> bool {
    VIEW_ONLY_IDS.contains(&id) || in_view_only_family(id)
}

/// Keys that only change layer state (visibility, lock), not what the layer is.
const LAYER_STATE_KEYS: &[&str] = &["on", "frozen", "locked"];

/// Whether running `spec` with `params` leaves drawing content untouched.
pub fn is_view_only(spec: &CommandSpec, params: &Value) -> bool {
    match spec.id {
        "layer.set" => params.as_object().is_some_and(|o| o.keys().all(|k| k == "name" || LAYER_STATE_KEYS.contains(&k.as_str()))),
        "-xref" => params
            .get("option")
            .and_then(Value::as_str)
            .is_some_and(|o| matches!(o.trim().to_ascii_lowercase().as_str(), "reload" | "r" | "unload" | "u")),
        // Without `replace` FIND only selects the matches.
        "find" => str_param(params, "replace").is_none(),
        // With `at` the extraction inserts a table; typed (no params) it may ask to.
        "dataextraction" | "eattext" => params.as_object().is_some_and(|o| o.get("at").is_none_or(Value::is_null)),
        // Without `name` or `delete` PARAMETERS only lists.
        "parameters" => str_param(params, "name").is_none() && str_param(params, "delete").is_none(),
        _ => spec.view_only,
    }
}

impl Session {
    /// Installs the policy consulted before every content-changing command; none means everything is allowed.
    pub fn set_edit_policy(&mut self, policy: Box<dyn EditPolicy>) {
        self.edit_policy = Some(policy);
    }

    pub(crate) fn check_edit(&self) -> Result<()> {
        match &self.edit_policy {
            Some(p) => p.check().map_err(EngineError::Unlicensed),
            None => Ok(()),
        }
    }

    /// Gate for `execute`/`start` only: input to a command that already started is never checked.
    pub(crate) fn gate(&self, spec: &CommandSpec, params: &Value) -> Result<()> {
        if is_view_only(spec, params) { Ok(()) } else { self.check_edit() }
    }
}

#[cfg(test)]
mod tests {
    use super::VIEW_ONLY_IDS;
    use crate::find_command;

    #[test]
    fn view_only_ids_exist_in_registry() {
        // A typo would otherwise fail closed without notice.
        for id in VIEW_ONLY_IDS {
            assert!(find_command(id).is_some_and(|s| s.id == *id), "`{id}` is on the view-only list but is not a command");
        }
    }
}
