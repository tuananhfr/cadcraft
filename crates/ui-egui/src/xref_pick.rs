//! Choosing the file of a missing xref where its saved path can't be followed (the web): the
//! External References palette asks the host's picker ([`crate::Services::pick_xref_file`]), the
//! host hands the file back to [`CadApp::attach_xref_file`], and a drawing that opens with missing
//! xrefs says where to do that.

use cadcraft_engine::EngineError;
use cadcraft_engine::doc::XrefStatus;
use serde_json::json;

use crate::CadApp;

/// Whether the palette offers Choose file… for an xref it lists with `status`; `can_pick` is
/// whether the host has a picker for xref files.
pub(crate) fn offers_file(can_pick: bool, status: &str) -> bool {
    can_pick && matches!(status, "Not Found" | "Unresolved")
}

/// Ask the host for the file of xref `name` of the active drawing.
pub(crate) fn pick(app: &CadApp, name: &str) {
    if let (Ok(st), Some(pick)) = (app.session.state(), &app.services.pick_xref_file) {
        pick(st.uid, name);
    }
}

/// After a drawing opens where the host can pick xref files: one line naming its missing xrefs.
pub(crate) fn hint_missing(app: &mut CadApp) {
    if app.services.pick_xref_file.is_none() {
        return;
    }
    let Ok(st) = app.session.state() else { return };
    let missing = |s: XrefStatus| matches!(s, XrefStatus::NotFound | XrefStatus::Unresolved);
    let names: Vec<&str> = st
        .doc
        .blocks
        .values()
        .filter(|b| b.xref_path.is_some() && !b.name.contains('|') && missing(b.xref_status))
        .map(|b| b.name.as_str())
        .collect();
    if names.is_empty() {
        return;
    }
    let line = crate::tf!("Xref not loaded: {names} — open the External References palette (XREF) and click Choose file…", names = names.join(", "));
    app.session.echo(line);
}

impl CadApp {
    /// Load xref `xref` of drawing `doc` (a uid) from `bytes`, the file `file` the host picked for
    /// it; that drawing becomes the active one. Failures go to the command line and status bar.
    pub fn attach_xref_file(&mut self, doc: u64, xref: &str, file: &str, bytes: &[u8]) {
        let Some(i) = self.session.docs.iter().position(|d| d.uid == doc) else {
            return self.open_failed(crate::tf!("Choose file: the drawing with xref \"{name}\" is no longer open", name = xref));
        };
        if i != self.session.active
            && let Err(e) = self.run("document.switch", json!({ "index": i }))
        {
            return self.open_failed(e);
        }
        match self.session.reload_xref_from_bytes(xref, file, bytes) {
            Ok(r) => {
                // The outcome line is echoed just before the notes about nested xrefs.
                let notes = r["nested"].as_array().map_or(0, Vec::len);
                let at = self.session.log.len().checked_sub(notes + 1);
                if let Some(line) = at.and_then(|i| self.session.log.get(i)).cloned() {
                    self.set_status(line);
                }
            }
            Err(EngineError::BadParams { msg, .. }) => self.open_failed(msg),
            Err(e) => self.open_failed(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_file_is_offered_for_missing_xrefs_where_the_host_can_pick() {
        for status in ["Not Found", "Unresolved"] {
            assert!(offers_file(true, status), "{status}");
            assert!(!offers_file(false, status), "{status}: the desktop has no picker for it");
        }
        for status in ["Loaded", "Unreferenced", "Unloaded", "Not loaded"] {
            assert!(!offers_file(true, status), "{status}");
        }
    }
}
