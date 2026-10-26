//! Loading an xref from a file the user picked instead of from its saved path: on the web there is
//! no disk to follow paths on, so the host reads the file and hands over its bytes. It runs as a
//! -XREF Reload of that xref (same checks, undo step and post-command passes), parsed and brought
//! in by the same code as a file read from disk ([`super::load`]). The other missing xrefs whose
//! saved paths name the same file get it in the same step.

use cadcraft_doc::{Drawing, XrefStatus};
use serde_json::{Value, json};

use super::{echo_all, host, load, xrefs};
use crate::cmd::{bad, find_command};
use crate::{EngineError, Result, Session};

impl Session {
    /// Load xref `name` of the active drawing from `bytes`, the contents of DWG or DXF file `file`
    /// the user picked for it (its name need not match the saved path's). Every other xref that
    /// is Not Found or Unresolved and whose saved path has the same file name is loaded from it
    /// too, in the same undo step. When `name` can't be loaded the drawing is left as it was and
    /// the error says why.
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
    let mut out = load::load_picked(&mut d, &key, file, bytes, host.as_deref());
    match out.first() {
        Some(o) if o.status == XrefStatus::Loaded => {}
        Some(o) => return Err(bad("-xref", o.message())),
        None => return Err(bad("-xref", format!("no xref \"{name}\""))),
    }
    let line = out.first().map(load::Outcome::message).unwrap_or_default();
    let mut also = Vec::new();
    for other in same_file(&d, &key) {
        // Tried on a copy, so one that fails stays as it was; its nested notes are the first one's.
        let mut trial = d.clone();
        let o = load::load_picked(&mut trial, &other, file, bytes, host.as_deref());
        if let Some(first) = o.into_iter().next().filter(|o| o.status == XrefStatus::Loaded) {
            d = trial;
            out.push(first);
            also.push(other);
        }
    }
    *s.doc_mut()? = d;
    echo_all(s, &out);
    let nested: Vec<&str> = out.iter().filter(|o| o.name.is_empty()).map(|o| o.detail.as_str()).collect();
    Ok(json!({ "name": key, "status": XrefStatus::Loaded.label(), "file": file, "also": also, "nested": nested, "line": line }))
}

/// The file name a saved xref path ends in, for comparing: either separator, any case.
fn file_name(path: &str) -> String {
    path.trim().rsplit(['/', '\\']).next().unwrap_or_default().to_lowercase()
}

/// The xrefs of `d` other than `key` that are missing and whose saved paths name the same file.
fn same_file(d: &Drawing, key: &str) -> Vec<String> {
    let Some(wanted) = d.block(key).and_then(|b| b.xref_path.as_deref()).map(file_name).filter(|f| !f.is_empty()) else {
        return Vec::new();
    };
    let missing = |s: XrefStatus| matches!(s, XrefStatus::NotFound | XrefStatus::Unresolved);
    xrefs(d)
        .filter(|b| !b.name.eq_ignore_ascii_case(key) && missing(b.xref_status))
        .filter(|b| b.xref_path.as_deref().map(file_name).is_some_and(|f| f == wanted))
        .map(|b| b.name.clone())
        .collect()
}
