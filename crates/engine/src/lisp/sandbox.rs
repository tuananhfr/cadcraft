//! Desktop write permissions shared by LISP file streams and output commands.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

pub const MAX_WRITE_ROOTS: usize = 64;
static WRITE_ROOTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn write_roots() -> Vec<PathBuf> {
    WRITE_ROOTS.lock().map(|roots| roots.clone()).unwrap_or_default()
}

pub fn set_write_roots(roots: Vec<PathBuf>) {
    let mut canonical: Vec<PathBuf> = Vec::new();
    for root in roots.into_iter().take(MAX_WRITE_ROOTS) {
        if let Ok(root) = root.canonicalize()
            && root.is_dir()
            && !canonical.iter().any(|existing| same_path(existing, &root))
        {
            canonical.push(root);
        }
    }
    if let Ok(mut roots) = WRITE_ROOTS.lock() {
        *roots = canonical;
    }
}

pub fn blocked(path: &str) -> String {
    format!("LISP write blocked: {path} is outside the allowed folders (LISPSANDBOX)")
}

#[cfg(not(target_os = "windows"))]
fn under(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(target_os = "windows")]
fn under(path: &Path, root: &Path) -> bool {
    let mut components = path.components();
    root.components().all(|root| {
        components.next().is_some_and(|part| match (part.as_os_str().to_str(), root.as_os_str().to_str()) {
            (Some(part), Some(root)) => part.to_lowercase() == root.to_lowercase(),
            _ => false,
        })
    })
}

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    under(a, b) && under(b, a)
}

pub fn check_write(path: &str) -> Result<PathBuf, String> {
    let refuse = || blocked(path);
    if path.is_empty() || path.len() > 4096 {
        return Err(refuse());
    }
    let requested = Path::new(path);
    let roots = write_roots();
    let allowed = |target: &Path| roots.iter().any(|root| under(target, root));
    let existing = match std::fs::symlink_metadata(requested) {
        Ok(_) => Some(requested.canonicalize().map_err(|_| refuse())?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(refuse()),
    };
    // Folder outputs may name the allowed root itself, whose parent is outside it.
    if let Some(target) = &existing
        && target.is_dir()
    {
        return if allowed(target) { Ok(target.clone()) } else { Err(refuse()) };
    }
    let name = requested.file_name().ok_or_else(refuse)?;
    let parent = requested.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().map_err(|_| refuse())?;
    if !parent.is_dir() || !allowed(&parent) || existing.as_deref().is_some_and(|target| !allowed(target)) {
        return Err(refuse());
    }
    Ok(existing.unwrap_or_else(|| parent.join(name)))
}

pub(crate) fn check_command(session: &crate::Session, id: &str, params: &Value) -> crate::Result<()> {
    if !session.lisp_running() || !(matches!(id, "export" | "exportpdf" | "plot" | "publish") || id.starts_with("ubplot.")) {
        return Ok(());
    }
    let keys: &[&str] = if id.starts_with("ubplot.") { &["path", "out", "dir"] } else { &["path"] };
    for key in keys {
        if let Some(path) = params.get(key).and_then(Value::as_str) {
            check_write(path).map_err(|error| crate::cmd::bad(id, error))?;
            if matches!(id, "export" | "exportpdf" | "plot") {
                check_write(&format!("{path}.cadcraft-tmp")).map_err(|error| crate::cmd::bad(id, error))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_covers_future_ubplot_folder_outputs() {
        let mut session = crate::Session::new();
        session.cmdline("(getstring)").unwrap();
        assert!(session.lisp_running());
        for key in ["path", "out", "dir"] {
            let params = serde_json::json!({(key): "\0"});
            let error = check_command(&session, "ubplot.folder", &params).unwrap_err().to_string();
            assert!(error.contains("LISP write blocked:"), "{key}: {error}");
        }
    }
}
