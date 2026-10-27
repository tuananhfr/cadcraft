//! Upload routing stays independent of browser APIs so workspace files cannot become drawings.

use cadcraft_engine::cmd::file::base64_encode;
use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::wsprofile::store::MAX_SET_BYTES;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    Drawing,
    Workspace,
    /// An AutoLISP file for Load Application; it still goes through the review prompt.
    Lisp,
    /// The file for xref `name` of drawing `doc` (its uid), loaded into it rather than opened.
    Xref {
        doc: u64,
        name: String,
    },
}

pub fn accept(purpose: &Purpose) -> &'static str {
    match purpose {
        Purpose::Drawing => ".dxf,.dwg",
        Purpose::Workspace => ".json",
        Purpose::Lisp => ".lsp",
        Purpose::Xref { .. } => ".dwg,.dxf",
    }
}

pub fn size_error(purpose: &Purpose, name: &str, len: u64) -> Option<String> {
    match purpose {
        Purpose::Drawing | Purpose::Lisp | Purpose::Xref { .. } => CadApp::open_size_error(name, len),
        Purpose::Workspace => (len > MAX_SET_BYTES as u64)
            .then(|| cadcraft_ui_egui::tf!("Workspace import: {name} is larger than {limit} bytes", name = name, limit = MAX_SET_BYTES)),
    }
}

pub fn route(purpose: &Purpose, name: &str, bytes: &[u8]) -> Result<(String, Value), String> {
    if let Some(error) = size_error(purpose, name, bytes.len() as u64) {
        return Err(error);
    }
    match purpose {
        Purpose::Drawing => Ok(("open".into(), json!({ "name": name, "data": base64_encode(bytes) }))),
        // The picker's filter is only a hint: "All files" must not turn Load Application into OPEN.
        Purpose::Lisp if !cadcraft_ui_egui::is_lisp_file(name) => {
            Err(cadcraft_ui_egui::tf!("APPLOAD: {name} is not an AutoLISP (.lsp) file", name = name).to_string())
        }
        Purpose::Lisp => Ok(("appload".into(), json!({ "name": name }))),
        // The bytes go to `CadApp::attach_xref_file` as they are: no base64 copy of a large DWG.
        Purpose::Xref { doc, name: xref } => Ok(("xref.file".into(), json!({ "doc": doc, "xref": xref, "file": name }))),
        Purpose::Workspace => {
            let text = std::str::from_utf8(bytes).map_err(|_| cadcraft_ui_egui::tl!("Workspace import: the file is not UTF-8 text").to_string())?;
            Ok(("preview".into(), json!({ "text": text })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_engine::cmd::file::base64_decode;

    #[test]
    fn web_upload_opens_the_preview() {
        let text = r#"{"schema_version":"1.0.0","name":"Workspace"}"#;
        let (id, params) = route(&Purpose::Workspace, "Workspace.json", text.as_bytes()).unwrap();
        assert_eq!(accept(&Purpose::Workspace), ".json");
        assert_eq!(id, "preview");
        assert_eq!(params, json!({ "text": text }));
        assert!(params.get("path").is_none());
        assert!(params.get("data").is_none());
    }

    #[test]
    fn drawing_upload_is_not_a_workspace() {
        assert_eq!(accept(&Purpose::Drawing), ".dxf,.dwg");
        for name in ["plan.dxf", "plan.dwg", "Workspace.json"] {
            let bytes = b"AC1032\0\xff";
            let (id, params) = route(&Purpose::Drawing, name, bytes).unwrap();
            assert_eq!(id, "open");
            assert_eq!(params["name"], name);
            assert_eq!(base64_decode(params["data"].as_str().unwrap()).unwrap(), bytes);
            assert!(params.get("text").is_none());
        }
    }

    #[test]
    fn workspace_upload_rejects_binary_and_oversize() {
        assert!(route(&Purpose::Workspace, "workspace.json", b"\xff\xfe").is_err());
        assert!(route(&Purpose::Workspace, "workspace.json", &vec![b' '; MAX_SET_BYTES + 1]).is_err());
        assert!(route(&Purpose::Workspace, "workspace.json", &vec![b' '; MAX_SET_BYTES]).is_ok());
        assert!(size_error(&Purpose::Workspace, "workspace.json", u64::MAX).is_some());
    }

    #[test]
    fn lisp_upload_loads_only_lsp_files() {
        assert_eq!(accept(&Purpose::Lisp), ".lsp");
        let (id, params) = route(&Purpose::Lisp, "UBB_PLOT.LSP", b"(princ)").unwrap();
        assert_eq!(id, "appload");
        assert_eq!(params, json!({ "name": "UBB_PLOT.LSP" }));
        for name in ["plan.dxf", "plan.dwg", "notes.txt"] {
            assert!(route(&Purpose::Lisp, name, b"(princ)").is_err(), "{name} is not loaded as LISP");
        }
        assert!(size_error(&Purpose::Lisp, "big.lsp", u64::MAX).is_some());
    }

    #[test]
    fn xref_upload_is_loaded_into_its_drawing_not_opened() {
        let purpose = Purpose::Xref { doc: 7, name: "Khung ten A3".into() };
        assert_eq!(accept(&purpose), ".dwg,.dxf");
        let (id, params) = route(&purpose, "frame.dwg", b"AC1032\0\xff").unwrap();
        assert_eq!(id, "xref.file");
        assert_eq!(params, json!({ "doc": 7, "xref": "Khung ten A3", "file": "frame.dwg" }));
        assert!(params.get("data").is_none());
        assert!(size_error(&purpose, "frame.dwg", u64::MAX).is_some(), "the drawing size limits apply");
    }
}
