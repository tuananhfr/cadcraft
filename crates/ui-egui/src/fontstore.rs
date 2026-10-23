//! The desktop's copy of the fonts the user added: font files in a folder of the settings
//! directory. [`FontStore::load`] adds them at startup and [`FontStore::sync`] writes or deletes
//! files as [`CadApp::take_font_changes`] reports. The web keeps them in the browser instead.

use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cadcraft_fonts::ttf;

use crate::CadApp;
use crate::fontmgr::FontChange;

const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc", "shx"];

/// Read a desktop font without allocating for an unbounded file, even if it grows while read.
pub fn read_font_file(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
    read_font_bytes(file, &name)
}

fn read_font_bytes(reader: impl Read, name: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(ttf::MAX_USER_FONT_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > ttf::MAX_USER_FONT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{name} is larger than {} MB", ttf::MAX_USER_FONT_BYTES >> 20)));
    }
    Ok(bytes)
}

/// A failed keep is retried after this long: each try may write 48 MB and sync it on the UI thread.
const RETRY_DELAY: Duration = Duration::from_secs(10);
const TEMPORARY_PREFIX: &str = ".cadcraft-font-";

pub struct FontStore {
    dir: PathBuf,
    retry_at: Cell<Option<Instant>>,
}

impl FontStore {
    pub fn new(dir: PathBuf) -> Self {
        FontStore { dir, retry_at: Cell::new(None) }
    }

    /// Add the fonts kept from earlier runs. A file that can't be read or isn't a font is skipped.
    pub fn load(&self, app: &mut CadApp) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        // A run that stopped mid-write leaves its temporary copy behind.
        for path in paths.iter().filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(TEMPORARY_PREFIX))) {
            let _ = std::fs::remove_file(path);
        }
        let mut files: Vec<PathBuf> = paths.into_iter().filter(|p| is_font_file(p)).collect();
        files.sort();
        for path in files {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            match read_font_file(&path) {
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
        if self.retry_at.get().is_some_and(|at| Instant::now() < at) {
            return;
        }
        let failed = self.sync_with(app, Self::write);
        self.retry_at.set(failed.then(|| Instant::now() + RETRY_DELAY));
    }

    /// Returns whether a change failed and was put back for a later try.
    fn sync_with(&self, app: &mut CadApp, mut write: impl FnMut(&Self, &str, &[u8]) -> io::Result<()>) -> bool {
        let mut changes = app.take_font_changes().into_iter();
        while let Some(change) = changes.next() {
            let r = match &change {
                FontChange::Added { file_name, bytes } => write(self, file_name, bytes),
                FontChange::Removed { stem } => self.delete(stem),
            };
            if let Err(e) = r {
                app.font_add_failed(crate::tf!("The font could not be kept for the next run: {e}", e = e));
                // Keep event order so a failed add cannot replay after a later removal.
                let mut pending = vec![change];
                pending.extend(changes);
                app.retry_font_changes(pending);
                return true;
            }
        }
        false
    }

    fn write(&self, file_name: &str, bytes: &[u8]) -> io::Result<()> {
        self.write_with(file_name, bytes, |file, bytes| file.write_all(bytes), |from, to| std::fs::rename(from, to))
    }

    fn write_with(
        &self,
        file_name: &str,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
        rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        if matches!(name, "" | "." | "..") {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file name"));
        }
        let target = self.dir.join(name);
        let (temporary, mut file) = self.temporary()?;
        let result = (|| {
            write(&mut file, bytes)?;
            file.flush()?;
            file.sync_all()?;
            // Windows requires the temporary handle closed before replacement.
            drop(file);
            rename(&temporary, &target)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
        self.delete_except(&ttf::key(name), Some(&target))
    }

    fn temporary(&self) -> io::Result<(PathBuf, File)> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let path = self.dir.join(format!("{TEMPORARY_PREFIX}{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::from(io::ErrorKind::AlreadyExists))
    }

    fn delete(&self, stem: &str) -> io::Result<()> {
        self.delete_except(stem, None)
    }

    fn delete_except(&self, stem: &str, keep: Option<&Path>) -> io::Result<()> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Ok(()) };
        for path in entries.flatten().map(|e| e.path()).filter(|p| is_font_file(p)) {
            let retained =
                keep.is_some_and(|keep| path.as_path() == keep || cfg!(windows) && path.as_os_str().eq_ignore_ascii_case(keep.as_os_str()));
            let same = path.file_name().is_some_and(|n| ttf::key(&n.to_string_lossy()) == stem);
            if same && !retained {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }
}

fn is_font_file(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|e| FONT_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use cadcraft_engine::Session;

    struct CountingReader<'a> {
        consumed: &'a Cell<usize>,
        requested: &'a Cell<usize>,
    }

    impl Read for CountingReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.requested.set(self.requested.get() + buffer.len());
            self.consumed.set(self.consumed.get() + buffer.len());
            buffer.fill(0);
            Ok(buffer.len())
        }
    }

    #[test]
    fn a_failed_keep_is_not_retried_every_frame() {
        let blocker = std::env::temp_dir().join(format!("cadcraft-font-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"a file where the font folder should be").unwrap();
        let store = FontStore::new(blocker.clone());
        let mut app = CadApp::new(Session::new(), crate::Services::default());
        let change = FontChange::Added { file_name: "kept.ttf".into(), bytes: b"bytes".to_vec() };
        app.retry_font_changes(vec![change.clone()]);
        store.sync(&mut app);
        assert!(store.retry_at.get().is_some());
        store.sync(&mut app);
        assert_eq!(app.take_font_changes(), vec![change], "still pending, not tried again yet");
        std::fs::remove_file(blocker).unwrap();
    }

    #[test]
    fn oversized_desktop_font_is_rejected_before_full_read() {
        let consumed = Cell::new(0);
        let requested = Cell::new(0);
        let mut app = CadApp::new(Session::new(), crate::Services::default());
        let result = read_font_bytes(CountingReader { consumed: &consumed, requested: &requested }, "oversized-desktop.shx")
            .and_then(|bytes| app.restore_font("oversized-desktop.shx", bytes).map_err(io::Error::other));
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "oversized-desktop.shx is larger than 48 MB");
        assert_eq!(consumed.get(), ttf::MAX_USER_FONT_BYTES + 1);
        assert_eq!(requested.get(), ttf::MAX_USER_FONT_BYTES + 1);
        assert!(!ttf::user_fonts().iter().any(|font| font.stem == "oversized-desktop.shx"));
        assert!(app.take_font_changes().is_empty());
    }

    #[test]
    fn desktop_font_reader_preserves_bytes_and_io_errors() {
        let bytes = b"synthetic font bytes";
        assert_eq!(read_font_bytes(bytes.as_slice(), "small.ttf").unwrap(), bytes);

        struct FailedReader;
        impl Read for FailedReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
        }
        assert_eq!(read_font_bytes(FailedReader, "unreadable.ttf").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("cadcraft-font-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn failed_font_replacement_preserves_previous_file() {
        let dir = TestDirectory::new("replacement-failure");
        let store = FontStore::new(dir.0.clone());
        let mut app = CadApp::new(Session::new(), crate::Services::default());
        for (original_name, replacement_name) in [
            ("kept.ttf", "kept.ttf"),
            ("kept.ttf", "kept.otf"),
            ("kept.otf", "kept.ttc"),
            ("kept.ttc", "kept.otc"),
            ("kept.shx", "kept.shx"),
            ("KEPT.TTF", "kept.ttf"),
        ] {
            for fail_rename in [false, true] {
                let original = store.dir.join(original_name);
                let target = store.dir.join(replacement_name);
                std::fs::write(&original, b"previous durable bytes").unwrap();
                let change = FontChange::Added { file_name: replacement_name.into(), bytes: b"replacement bytes".to_vec() };
                app.retry_font_changes(vec![change.clone()]);
                store.sync_with(&mut app, |store, name, bytes| {
                    store.write_with(
                        name,
                        bytes,
                        |file, bytes| {
                            assert_eq!(std::fs::read(&original).unwrap(), b"previous durable bytes");
                            if fail_rename {
                                file.write_all(bytes)
                            } else {
                                file.write_all(b"partial write")?;
                                Err(io::Error::from(io::ErrorKind::StorageFull))
                            }
                        },
                        |temporary, target| {
                            assert!(fail_rename);
                            assert_eq!(temporary.parent(), target.parent());
                            assert_eq!(std::fs::read(&original).unwrap(), b"previous durable bytes");
                            assert_eq!(std::fs::read(temporary).unwrap(), b"replacement bytes");
                            Err(io::Error::from(io::ErrorKind::PermissionDenied))
                        },
                    )
                });
                assert_eq!(std::fs::read(&original).unwrap(), b"previous durable bytes");
                assert_eq!(std::fs::read_dir(&store.dir).unwrap().count(), 1, "failed temporary files are removed");
                let pending = app.take_font_changes();
                assert_eq!(pending, vec![change]);
                app.retry_font_changes(pending);
                store.sync(&mut app);
                assert_eq!(std::fs::read(&target).unwrap(), b"replacement bytes");
                assert_eq!(std::fs::read_dir(&store.dir).unwrap().count(), 1, "obsolete extensions are removed only after success");
                assert!(app.take_font_changes().is_empty());
                std::fs::remove_file(&target).unwrap();
            }
        }
    }

    #[test]
    fn failed_font_add_retries_before_a_later_removal() {
        let dir = TestDirectory::new("retry-order");
        let store = FontStore::new(dir.0.clone());
        let mut app = CadApp::new(Session::new(), crate::Services::default());
        std::fs::write(store.dir.join("kept.ttf"), b"original bytes").unwrap();
        let changes = vec![
            FontChange::Added { file_name: "kept.otf".into(), bytes: b"replacement bytes".to_vec() },
            FontChange::Removed { stem: "kept".into() },
        ];
        app.retry_font_changes(changes.clone());
        store.sync_with(&mut app, |_, _, _| Err(io::Error::from(io::ErrorKind::StorageFull)));
        assert_eq!(std::fs::read(store.dir.join("kept.ttf")).unwrap(), b"original bytes");
        let pending = app.take_font_changes();
        assert_eq!(pending, changes);
        app.retry_font_changes(pending);
        store.sync(&mut app);
        assert_eq!(std::fs::read_dir(&store.dir).unwrap().count(), 0);
        assert!(app.take_font_changes().is_empty());
    }
}
