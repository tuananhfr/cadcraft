//! SHX fonts the user added, by lower-case file name with the `.shx` extension. Kept apart from
//! the TrueType database so that finding a stroke-font name never scans the system fonts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use super::ShxFont;

fn fonts() -> MutexGuard<'static, HashMap<String, Arc<ShxFont>>> {
    static S: OnceLock<Mutex<HashMap<String, Arc<ShxFont>>>> = OnceLock::new();
    S.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn insert(key: &str, font: ShxFont) {
    fonts().insert(key.to_string(), Arc::new(font));
}

pub(crate) fn remove(key: &str) {
    fonts().remove(key);
}

/// The font a style's `name` asks for ("vhtime.shx"); a bare "txt" finds "txt.shx".
pub(crate) fn find(name: &str) -> Option<Arc<ShxFont>> {
    let k = name.trim().to_ascii_lowercase();
    let fonts = fonts();
    fonts.get(&k).or_else(|| if k.contains('.') { None } else { fonts.get(&format!("{k}.shx")) }).cloned()
}
