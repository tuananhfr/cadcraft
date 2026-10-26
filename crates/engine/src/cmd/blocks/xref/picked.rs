//! Loading an xref from a file the user picked instead of from its saved path: on the web there is
//! no disk to follow paths on, so the host reads the file and hands over its bytes. It runs as a
//! -XREF Reload of that xref (same checks, undo step and post-command passes), parsed and brought
//! in by the same code as a file read from disk ([`super::load`]).

use cadcraft_doc::XrefStatus;
use serde_json::{Value, json};

use super::{echo_all, host, load, xrefs};
use crate::cmd::{bad, find_command};
use crate::{EngineError, Result, Session};

impl Session {
    /// Load xref `name` of the active drawing from `bytes`, the contents of DWG or DXF file `file`
    /// the user picked for it (its name need not match the saved path's). When it can't be
    /// loaded the drawing is left as it was and the error says why.
    pub fn reload_xref_from_bytes(&mut self, name: &str, file: &str, bytes: &[u8]) -> Result<Value> {
        let spec = find_command("-xref").ok_or_else(|| EngineError::UnknownCommand("-xref".into()))?;
        let params = json!({ "option": "reload", "names": name, "file": file });
        self.execute_as("-xref", spec, &params, &mut |s, _| reload_picked(s, name, file, bytes))
    }
}

fn reload_picked(s: &mut Session, name: &str, file: &str, bytes: &[u8]) -> Result<Value> {
    let key = xrefs(s.doc()?).find(|b| b.name.eq_ignore_ascii_case(name)).map(|b| b.name.clone());
    let key = key.ok_or_else(|| bad("-xref", format!("no xref \"{name}\"")))?;
    let host = host(s);
    // Loaded into a copy, so a file that fails leaves the drawing (and its undo) untouched.
    let mut d = s.doc()?.clone();
    let out = load::load_picked(&mut d, &key, file, bytes, host.as_deref());
    match out.first() {
        Some(o) if o.status == XrefStatus::Loaded => {}
        Some(o) => return Err(bad("-xref", o.message())),
        None => return Err(bad("-xref", format!("no xref \"{name}\""))),
    }
    *s.doc_mut()? = d;
    echo_all(s, &out);
    let nested: Vec<&str> = out.iter().skip(1).map(|o| o.detail.as_str()).collect();
    Ok(json!({ "name": key, "status": XrefStatus::Loaded.label(), "file": file, "nested": nested }))
}
