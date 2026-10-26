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
    let names = names.join(", ");
    let line = english(|| crate::tf!("Xref not loaded: {names} — open the External References palette (XREF) and click Choose file…", names = names));
    app.session.echo(line);
}

/// A line for the command line or status bar, kept in English like the engine's: both translate
/// it when shown (`echo` templates), whatever language was current when it was made.
fn english(line: impl FnOnce() -> String) -> String {
    crate::i18n::with_language("en", line)
}

impl CadApp {
    /// Load xref `xref` of drawing `doc` (a uid) from `bytes`, the file `file` the host picked for
    /// it; that drawing becomes the active one. Failures go to the command line and status bar.
    pub fn attach_xref_file(&mut self, doc: u64, xref: &str, file: &str, bytes: &[u8]) {
        let Some(i) = self.session.docs.iter().position(|d| d.uid == doc) else {
            return self.open_failed(english(|| crate::tf!("Choose file: the drawing with xref \"{name}\" is no longer open", name = xref)));
        };
        if i != self.session.active
            && let Err(e) = self.run("document.switch", json!({ "index": i }))
        {
            return self.open_failed(e);
        }
        match self.session.reload_xref_from_bytes(xref, file, bytes) {
            Ok(r) => {
                if let Some(line) = r["line"].as_str() {
                    self.set_status(line.to_string());
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

    fn app_with_missing_xref() -> CadApp {
        use cadcraft_engine::doc::{Block, Drawing};
        let mut d = Drawing::new_metric();
        let mut b = Block::new("Khung");
        b.xref_path = Some("Khung ten A3.dwg".into());
        b.xref_status = XrefStatus::NotFound;
        d.blocks.insert("Khung".into(), std::sync::Arc::new(b));
        let mut session = cadcraft_engine::Session::empty();
        session.open_drawing(d, "host", None);
        let services = crate::Services { pick_xref_file: Some(Box::new(|_, _| {})), ..crate::Services::default() };
        CadApp::new(session, services)
    }

    #[test]
    fn lines_are_logged_in_english_and_shown_in_the_ui_language() {
        let hint = "Xref not loaded: Khung — open the External References palette (XREF) and click Choose file…";
        let gone = "Choose file: the drawing with xref \"Khung\" is no longer open";
        // The web can open a drawing before the UI language is applied, or while it is.
        for current in ["en", "vi"] {
            let mut app = app_with_missing_xref();
            app.session.log.clear();
            crate::i18n::with_language(current, || {
                hint_missing(&mut app);
                app.attach_xref_file(u64::MAX, "Khung", "Khung ten A3.dwg", b"");
            });
            assert_eq!(app.session.log, [hint, gone], "{current}");
            assert_eq!(app.status.as_ref().map(|(s, _)| s.as_str()), Some(gone), "{current}");
        }
        let shown = |language, line| crate::i18n::with_language(language, || crate::i18n::echo(line).into_owned());
        assert_eq!(shown("vi", hint), "Xref chưa nạp: Khung — mở bảng External References (XREF) và bấm Chọn file…");
        assert_eq!(shown("vi", gone), "Chọn file: bản vẽ chứa tham chiếu ngoài \"Khung\" không còn mở");
        for line in [hint, gone] {
            assert_ne!(shown("uk", line), line);
            assert_eq!(shown("en", line), line);
        }
    }
}
