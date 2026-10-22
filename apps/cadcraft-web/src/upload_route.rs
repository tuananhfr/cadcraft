//! Upload routing stays independent of browser APIs so workspace files cannot become drawings.

use cadcraft_engine::cmd::file::base64_encode;
use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::wsprofile::store::MAX_SET_BYTES;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Drawing,
    Workspace,
}

pub fn accept(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Drawing => ".dxf,.dwg",
        Purpose::Workspace => ".json",
    }
}

pub fn size_error(purpose: Purpose, name: &str, len: u64) -> Option<String> {
    match purpose {
        Purpose::Drawing => CadApp::open_size_error(name, len),
        Purpose::Workspace => (len > MAX_SET_BYTES as u64)
            .then(|| cadcraft_ui_egui::tf!("Workspace import: {name} is larger than {limit} bytes", name = name, limit = MAX_SET_BYTES)),
    }
}

pub fn route(purpose: Purpose, name: &str, bytes: &[u8]) -> Result<(String, Value), String> {
    if let Some(error) = size_error(purpose, name, bytes.len() as u64) {
        return Err(error);
    }
    match purpose {
        Purpose::Drawing => Ok(("open".into(), json!({ "name": name, "data": base64_encode(bytes) }))),
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
        let (id, params) = route(Purpose::Workspace, "Workspace.json", text.as_bytes()).unwrap();
        assert_eq!(accept(Purpose::Workspace), ".json");
        assert_eq!(id, "preview");
        assert_eq!(params, json!({ "text": text }));
        assert!(params.get("path").is_none());
        assert!(params.get("data").is_none());
    }

    #[test]
    fn drawing_upload_is_not_a_workspace() {
        assert_eq!(accept(Purpose::Drawing), ".dxf,.dwg");
        for name in ["plan.dxf", "plan.dwg", "Workspace.json"] {
            let bytes = b"AC1032\0\xff";
            let (id, params) = route(Purpose::Drawing, name, bytes).unwrap();
            assert_eq!(id, "open");
            assert_eq!(params["name"], name);
            assert_eq!(base64_decode(params["data"].as_str().unwrap()).unwrap(), bytes);
            assert!(params.get("text").is_none());
        }
    }

    #[test]
    fn workspace_upload_rejects_binary_and_oversize() {
        assert!(route(Purpose::Workspace, "workspace.json", b"\xff\xfe").is_err());
        assert!(route(Purpose::Workspace, "workspace.json", &vec![b' '; MAX_SET_BYTES + 1]).is_err());
        assert!(route(Purpose::Workspace, "workspace.json", &vec![b' '; MAX_SET_BYTES]).is_ok());
        assert!(size_error(Purpose::Workspace, "workspace.json", u64::MAX).is_some());
    }
}
