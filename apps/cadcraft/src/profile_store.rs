//! The desktop profile store: files under `<settings dir>/workspace/`.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, MAX_SET_BYTES, ProfileStore, expired_snapshots};

const CURRENT: &str = "current.json";
const POLICY: &str = "policy.json";
const SNAPSHOTS: &str = "snapshots";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct FileProfileStore {
    dir: PathBuf,
    /// Set when `current.json` exists but could not be read: it is kept as it is until the user repairs it.
    current_unreadable: AtomicBool,
}

impl FileProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, current_unreadable: AtomicBool::new(false) }
    }

    pub fn has_current(&self) -> bool {
        self.dir.join(CURRENT).exists()
    }

    fn snapshot_path(&self, revision: u64) -> PathBuf {
        self.dir.join(SNAPSHOTS).join(format!("rev-{revision}.json"))
    }
}

/// Reads at most `limit` bytes; `None` when the file does not exist.
fn read_capped(path: &Path, limit: usize) -> std::io::Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// tmp -> fsync -> rename, so a crash leaves the old file or the new one, never half of either.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("workspace path has no parent")?;
    fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".profile-{}-{sequence}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|error| format!("{}: {error}", temporary.display()))?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: could not save workspace: {error}", path.display()));
    }
    Ok(())
}

fn within(json: &str, limit: usize) -> Result<(), String> {
    if json.len() > limit { Err(format!("the workspace is larger than {MAX_SET_BYTES} bytes")) } else { Ok(()) }
}

impl ProfileStore for FileProfileStore {
    fn read_current(&self) -> Option<String> {
        // One byte past the cap, so startup sees an oversized file as damaged instead of cutting it silently.
        match read_capped(&self.dir.join(CURRENT), MAX_SET_BYTES + 1) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("workspace: {CURRENT}: {error}");
                self.current_unreadable.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    fn write_current(&mut self, json: &str) -> Result<(), String> {
        if self.current_unreadable.load(Ordering::Relaxed) {
            return Err(format!("{CURRENT} could not be read and was left unchanged"));
        }
        within(json, MAX_SET_BYTES)?;
        write_atomic(&self.dir.join(CURRENT), json)
    }

    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        // A damaged copy may be the one-byte-over text `read_current` hands out.
        within(json, if revision == CORRUPT_SNAPSHOT { MAX_SET_BYTES + 1 } else { MAX_SET_BYTES })?;
        write_atomic(&self.snapshot_path(revision), json)?;
        for expired in expired_snapshots(&self.snapshots()) {
            if let Err(error) = fs::remove_file(self.snapshot_path(expired)) {
                log::warn!("workspace: could not remove snapshot {expired}: {error}");
            }
        }
        Ok(())
    }

    fn snapshots(&self) -> Vec<u64> {
        let Ok(entries) = fs::read_dir(self.dir.join(SNAPSHOTS)) else { return Vec::new() };
        let mut revisions: Vec<u64> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let digits = name.to_str()?.strip_prefix("rev-")?.strip_suffix(".json")?;
                if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                digits.parse().ok()
            })
            .collect();
        revisions.sort_unstable();
        revisions.dedup();
        revisions
    }

    fn read_snapshot(&self, revision: u64) -> Option<String> {
        read_capped(&self.snapshot_path(revision), MAX_SET_BYTES + 1).ok().flatten()
    }

    fn read_policy(&self) -> Option<String> {
        match read_capped(&self.dir.join(POLICY), MAX_SET_BYTES + 1) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("workspace: {POLICY}: {error}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_ui_egui::wsprofile::startup::{Fallback, restore};
    use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MAX_SET_BYTES};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cadcraft-profile-store-test-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn writes_replace_current_without_leaving_temporary_files() {
        let dir = temp_dir();
        let mut store = FileProfileStore::new(dir.join("workspace"));
        assert_eq!(store.read_current(), None);
        store.write_current(r#"{"active":"SIMPLE"}"#).unwrap();
        store.write_current(r#"{"active":"MODELER"}"#).unwrap();
        assert_eq!(store.read_current().as_deref(), Some(r#"{"active":"MODELER"}"#));
        let leftovers: Vec<_> = fs::read_dir(dir.join("workspace")).unwrap().flatten().map(|entry| entry.file_name()).collect();
        assert_eq!(leftovers, ["current.json"], "{leftovers:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn snapshots_pruned_to_keep_limit() {
        let dir = temp_dir();
        let mut store = FileProfileStore::new(dir.join("workspace"));
        store.push_snapshot(CORRUPT_SNAPSHOT, "{damaged").unwrap();
        for revision in 1..=25u64 {
            store.push_snapshot(revision, &format!(r#"{{"rev":{revision}}}"#)).unwrap();
        }
        let kept = store.snapshots();
        assert_eq!(kept.len(), KEEP_SNAPSHOTS + 1, "{kept:?}");
        assert_eq!(kept.first(), Some(&CORRUPT_SNAPSHOT), "the damaged copy is never pruned");
        assert_eq!(kept.get(1), Some(&6));
        assert_eq!(kept.last(), Some(&25));
        assert!(kept.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(store.read_snapshot(25).as_deref(), Some(r#"{"rev":25}"#));
        assert_eq!(store.read_snapshot(5), None);
        assert_eq!(fs::read_dir(dir.join("workspace/snapshots")).unwrap().count(), KEEP_SNAPSHOTS + 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_current_json_is_reported_not_deleted() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("current.json"), "{").unwrap();
        let mut store = FileProfileStore::new(workspace.clone());
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::Corrupt(_))), "{:?}", restored.fallback);
        assert_eq!(fs::read_to_string(workspace.join("current.json")).unwrap(), "{");
        assert_eq!(store.read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some("{"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_newer_major_set_is_kept_as_a_snapshot() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let newer = r#"{"schema_version":"99.0.0","active":"SIMPLE"}"#;
        fs::write(workspace.join("current.json"), newer).unwrap();
        let mut store = FileProfileStore::new(workspace.clone());
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::NewerMajor(_))), "{:?}", restored.fallback);
        assert_eq!(fs::read_to_string(workspace.join("current.json")).unwrap(), newer);
        assert_eq!(store.read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some(newer));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_oversized_file_is_reported_and_kept() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("current.json"), vec![b' '; MAX_SET_BYTES + 10]).unwrap();
        let mut store = FileProfileStore::new(workspace.clone());
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::Corrupt(_))), "{:?}", restored.fallback);
        assert_eq!(fs::metadata(workspace.join("current.json")).unwrap().len(), (MAX_SET_BYTES + 10) as u64);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_over_the_size_cap_are_refused() {
        let dir = temp_dir();
        let mut store = FileProfileStore::new(dir.join("workspace"));
        let big = "x".repeat(MAX_SET_BYTES + 1);
        assert!(store.write_current(&big).is_err());
        assert!(store.push_snapshot(1, &big).is_err());
        assert_eq!(store.read_current(), None);
        assert!(store.snapshots().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_current_file_is_never_overwritten() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        // A directory where the file should be: reading fails with something other than "not found".
        fs::create_dir_all(workspace.join("current.json")).unwrap();
        let mut store = FileProfileStore::new(workspace.clone());
        assert_eq!(store.read_current(), None);
        let error = store.write_current("{}").unwrap_err();
        assert!(error.contains("left unchanged"), "{error}");
        assert!(workspace.join("current.json").is_dir());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn policy_is_read_when_present() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        let store = FileProfileStore::new(workspace.clone());
        assert_eq!(store.read_policy(), None);
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("policy.json"), r#"{"layers":[]}"#).unwrap();
        assert_eq!(store.read_policy().as_deref(), Some(r#"{"layers":[]}"#));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unrelated_files_in_the_snapshot_folder_are_ignored() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        let mut store = FileProfileStore::new(workspace.clone());
        store.push_snapshot(3, "{}").unwrap();
        fs::write(workspace.join("snapshots/notes.txt"), "x").unwrap();
        fs::write(workspace.join("snapshots/rev-x.json"), "x").unwrap();
        fs::write(workspace.join("snapshots/rev-4.json.bak"), "x").unwrap();
        assert_eq!(store.snapshots(), [3]);
        fs::remove_dir_all(dir).unwrap();
    }
}
