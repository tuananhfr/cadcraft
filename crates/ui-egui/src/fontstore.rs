//! The desktop's copy of the fonts the user added: font files in a folder of the settings
//! directory. [`FontStore::load`] adds them at startup and [`FontStore::sync`] writes or deletes
//! files as [`CadApp::take_font_changes`] reports. The web keeps them in the browser instead.

use std::path::{Path, PathBuf};

use cadcraft_fonts::ttf;

use crate::CadApp;
use crate::fontmgr::FontChange;

const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc", "shx"];

pub struct FontStore {
    dir: PathBuf,
}

impl FontStore {
    pub fn new(dir: PathBuf) -> Self {
        FontStore { dir }
    }

    /// Add the fonts kept from earlier runs. A file that can't be read or isn't a font is skipped.
    pub fn load(&self, app: &mut CadApp) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_font_file(p)).collect();
        files.sort();
        for path in files {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if let Err(e) = app.restore_font(&name, bytes) {
                        log::warn!("kept font {}: {e}", path.display());
                    }
                }
                Err(e) => log::warn!("kept font {}: {e}", path.display()),
            }
        }
    }

    /// Keep what the user added or removed since the last call. A failure shows in the status bar:
    /// the font still works for this run.
    pub fn sync(&self, app: &mut CadApp) {
        for change in app.take_font_changes() {
            let r = match &change {
                FontChange::Added { file_name, bytes } => self.write(file_name, bytes),
                FontChange::Removed { stem } => self.delete(stem),
            };
            if let Err(e) = r {
                app.font_add_failed(format!("The font could not be kept for the next run: {e}"));
            }
        }
    }

    fn write(&self, file_name: &str, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        if matches!(name, "" | "." | "..") {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a file name"));
        }
        // The same font added again replaces the old file, whatever its extension.
        self.delete(&ttf::key(name))?;
        std::fs::write(self.dir.join(name), bytes)
    }

    fn delete(&self, stem: &str) -> std::io::Result<()> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Ok(()) };
        for path in entries.flatten().map(|e| e.path()).filter(|p| is_font_file(p)) {
            let same = path.file_name().is_some_and(|n| ttf::key(&n.to_string_lossy()) == stem);
            if same {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }
}

fn is_font_file(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|e| FONT_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}
