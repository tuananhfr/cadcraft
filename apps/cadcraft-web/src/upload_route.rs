//! Upload routing stays independent of browser APIs so workspace files cannot become drawings.

use cadcraft_engine::cmd::file::base64_encode;
use cadcraft_engine::doc::MAX_TABLE_FILE;
use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::wsprofile::store::MAX_SET_BYTES;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    Drawing,
    Workspace,
    /// The file for xref `name` of drawing `doc` (its uid), loaded into it rather than opened.
    Xref {
        doc: u64,
        name: String,
    },
    /// An AutoCAD plot style table (`.ctb`/`.stb`) for the Plot Style Manager.
    PlotStyle,
}

pub fn accept(purpose: &Purpose) -> &'static str {
    match purpose {
        Purpose::Drawing => ".dxf,.dwg",
        Purpose::Workspace => ".json",
        Purpose::Xref { .. } => ".dwg,.dxf",
        Purpose::PlotStyle => ".ctb,.stb",
    }
}

pub fn size_error(purpose: &Purpose, name: &str, len: u64) -> Option<String> {
    match purpose {
        Purpose::Drawing | Purpose::Xref { .. } => CadApp::open_size_error(name, len),
        Purpose::Workspace => (len > MAX_SET_BYTES as u64)
            .then(|| cadcraft_ui_egui::tf!("Workspace import: {name} is larger than {limit} bytes", name = name, limit = MAX_SET_BYTES)),
        Purpose::PlotStyle => (len > MAX_TABLE_FILE as u64)
            .then(|| cadcraft_ui_egui::tf!("Plot style import: {name} is larger than {limit} bytes", name = name, limit = MAX_TABLE_FILE)),
    }
}

pub fn route(purpose: &Purpose, name: &str, bytes: &[u8]) -> Result<(String, Value), String> {
    if let Some(error) = size_error(purpose, name, bytes.len() as u64) {
        return Err(error);
    }
    match purpose {
        Purpose::Drawing => Ok(("open".into(), json!({ "name": name, "data": base64_encode(bytes) }))),
        // The bytes go to `CadApp::attach_xref_file` as they are: no base64 copy of a large DWG.
        Purpose::Xref { doc, name: xref } => Ok(("xref.file".into(), json!({ "doc": doc, "xref": xref, "file": name }))),
        Purpose::PlotStyle => Ok(("stylesmanager".into(), json!({ "action": "load", "name": name, "data": base64_encode(bytes) }))),
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
    fn xref_upload_is_loaded_into_its_drawing_not_opened() {
        let purpose = Purpose::Xref { doc: 7, name: "Khung ten A3".into() };
        assert_eq!(accept(&purpose), ".dwg,.dxf");
        let (id, params) = route(&purpose, "frame.dwg", b"AC1032\0\xff").unwrap();
        assert_eq!(id, "xref.file");
        assert_eq!(params, json!({ "doc": 7, "xref": "Khung ten A3", "file": "frame.dwg" }));
        assert!(params.get("data").is_none());
        assert!(size_error(&purpose, "frame.dwg", u64::MAX).is_some(), "the drawing size limits apply");
    }

    #[test]
    fn plot_style_upload_loads_the_table() {
        assert_eq!(accept(&Purpose::PlotStyle), ".ctb,.stb");
        let bytes = b"PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec";
        let (id, params) = route(&Purpose::PlotStyle, "office.ctb", bytes).unwrap();
        assert_eq!(id, "stylesmanager");
        assert_eq!((params["action"].as_str(), params["name"].as_str()), (Some("load"), Some("office.ctb")));
        assert_eq!(base64_decode(params["data"].as_str().unwrap()).unwrap(), bytes);
        assert!(size_error(&Purpose::PlotStyle, "office.ctb", MAX_TABLE_FILE as u64).is_none());
        assert!(size_error(&Purpose::PlotStyle, "office.ctb", MAX_TABLE_FILE as u64 + 1).is_some());
        assert!(route(&Purpose::PlotStyle, "huge.ctb", &vec![0; MAX_TABLE_FILE + 1]).is_err());
    }
}
