//! The desktop profile store: files under `<settings dir>/workspace/`.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cadcraft_ui_egui::wsprofile::store::{MAX_SET_BYTES, ProfileStore, expired_snapshots};

const CURRENT: &str = "current.json";
const BACKUP: &str = "backup-v1.json";
const POLICY: &str = "policy.json";
const SNAPSHOTS: &str = "snapshots";
const DAMAGED: &str = "damaged";
/// Damaged copies kept, oldest dropped first; they are evidence, so they outlive snapshot pruning.
const MAX_DAMAGED_COPIES: usize = 10;
/// Larger files are kept cut at this size.
const MAX_DAMAGED_BYTES: usize = 16 << 20;
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

/// Reads at most `limit` bytes as they are; `None` when the file does not exist.
fn read_bytes_capped(path: &Path, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// `<unix>-<n>.json` as (n, unix): `n` only grows, so it orders copies even if the clock goes back.
fn damaged_order(name: &str) -> Option<(u64, u64)> {
    let (unix, n) = name.strip_suffix(".json")?.split_once('-')?;
    Some((n.parse().ok()?, unix.parse().ok()?))
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
    fn keep_backup(&mut self, version: &str, raw: &str) -> Result<(), String> {
        if version != "1.0.0" {
            return Err("unsupported backup version".into());
        }
        within(raw, MAX_SET_BYTES)?;
        let backup = self.dir.join(BACKUP);
        if read_capped(&backup, MAX_SET_BYTES + 1).map_err(|error| error.to_string())?.is_some() {
            return Ok(());
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let staged = self.dir.join(format!(".backup-{}-{sequence}.tmp", std::process::id()));
        write_atomic(&staged, raw)?;
        // Publishing a complete inode avoids both partial backups and replacing another writer's original.
        let result = fs::hard_link(&staged, &backup);
        let _ = fs::remove_file(&staged);
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(format!("{}: {error}", backup.display())),
        }
    }

    fn read_backup(&self, version: &str) -> Option<String> {
        if version != "1.0.0" {
            return None;
        }
        read_capped(&self.dir.join(BACKUP), MAX_SET_BYTES + 1).ok().flatten().filter(|text| text.len() <= MAX_SET_BYTES)
    }

    fn read_current(&self) -> Option<String> {
        // Too large or not UTF-8 text goes to startup as bytes, which keeps it as it is.
        let bytes = self.read_current_bytes().filter(|bytes| bytes.len() <= MAX_SET_BYTES)?;
        String::from_utf8(bytes).ok()
    }

    fn read_current_bytes(&self) -> Option<Vec<u8>> {
        match read_bytes_capped(&self.dir.join(CURRENT), MAX_DAMAGED_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("workspace: {CURRENT}: {error}");
                self.current_unreadable.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    fn keep_damaged(&mut self, raw: &[u8]) -> Result<String, String> {
        let dir = self.dir.join(DAMAGED);
        fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let mut kept: Vec<((u64, u64), String)> = fs::read_dir(&dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                Some((damaged_order(&name)?, name))
            })
            .collect();
        kept.sort();
        let raw = raw.get(..MAX_DAMAGED_BYTES).unwrap_or(raw);
        let same = kept.iter().find(|(_, name)| {
            let path = dir.join(name);
            fs::metadata(&path).is_ok_and(|meta| meta.len() == raw.len() as u64) && fs::read(&path).is_ok_and(|bytes| bytes == raw)
        });
        if let Some((_, name)) = same {
            return Ok(format!("{DAMAGED}/{name}"));
        }
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
        let mut n = kept.last().map_or(0, |((n, _), _)| n.saturating_add(1));
        let name = loop {
            let name = format!("{unix}-{n}.json");
            let path = dir.join(&name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            match options.open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(raw).and_then(|()| file.sync_all()) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(format!("{}: {error}", path.display()));
                    }
                    break name;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && n < u64::MAX => n += 1,
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
        };
        kept.push(((n, unix), name.clone()));
        let excess = kept.len().saturating_sub(MAX_DAMAGED_COPIES);
        for (_, old) in kept.iter().take(excess) {
            if let Err(error) = fs::remove_file(dir.join(old)) {
                log::warn!("workspace: could not remove old damaged copy {old}: {error}");
            }
        }
        Ok(format!("{DAMAGED}/{name}"))
    }

    fn write_current(&mut self, json: &str) -> Result<(), String> {
        if self.current_unreadable.load(Ordering::Relaxed) {
            return Err(format!("{CURRENT} could not be read and was left unchanged"));
        }
        within(json, MAX_SET_BYTES)?;
        write_atomic(&self.dir.join(CURRENT), json)
    }

    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        within(json, MAX_SET_BYTES)?;
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
    fn file_store_keeps_backup_once() {
        let dir = temp_dir();
        let mut store = FileProfileStore::new(dir.join("workspace"));
        let original = "{\r\n  \"schema_version\": \"1.0.0\"\r\n}";
        store.keep_backup("1.0.0", original).unwrap();
        store.keep_backup("1.0.0", "replacement").unwrap();
        assert_eq!(store.read_backup("1.0.0").as_deref(), Some(original));
        assert_eq!(fs::read(dir.join("workspace/backup-v1.json")).unwrap(), original.as_bytes());
        assert_eq!(fs::read_dir(dir.join("workspace")).unwrap().count(), 1);
        assert_eq!(store.read_backup("4"), None);
        fs::remove_dir_all(dir).unwrap();
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
        assert_eq!(damaged_copies(&workspace), [b"{".to_vec()]);
        assert!(store.snapshots().is_empty(), "a damaged copy is not a snapshot");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_newer_major_set_is_kept_as_a_damaged_copy() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let newer = r#"{"schema_version":"99.0.0","active":"SIMPLE"}"#;
        fs::write(workspace.join("current.json"), newer).unwrap();
        let mut store = FileProfileStore::new(workspace.clone());
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::NewerMajor(_))), "{:?}", restored.fallback);
        assert_eq!(fs::read_to_string(workspace.join("current.json")).unwrap(), newer);
        assert_eq!(damaged_copies(&workspace), [newer.as_bytes().to_vec()]);
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
        assert_eq!(damaged_copies(&workspace), [vec![b' '; MAX_SET_BYTES + 10]]);
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

    /// The kept damaged copies, oldest first.
    fn damaged_copies(workspace: &Path) -> Vec<Vec<u8>> {
        let mut names: Vec<_> =
            fs::read_dir(workspace.join("damaged")).map(|dir| dir.flatten().map(|entry| entry.file_name()).collect()).unwrap_or_default();
        names.sort_by_key(|name| damaged_order(&name.to_string_lossy()));
        names.iter().map(|name| fs::read(workspace.join("damaged").join(name)).unwrap()).collect()
    }

    #[test]
    fn a_second_damaged_copy_does_not_replace_the_first() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        let mut store = FileProfileStore::new(workspace.clone());
        let first = store.keep_damaged(b"{first").unwrap();
        let second = store.keep_damaged(b"{second").unwrap();
        assert_ne!(first, second);
        assert_eq!(damaged_copies(&workspace), [b"{first".to_vec(), b"{second".to_vec()]);
        // The same bytes again (a restart that changed nothing) add no copy.
        assert_eq!(store.keep_damaged(b"{first").unwrap(), first);
        assert_eq!(damaged_copies(&workspace).len(), 2);
        assert!(store.snapshots().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn non_utf8_bytes_are_kept_byte_for_byte() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let raw: Vec<u8> = vec![b'{', 0xff, 0xfe, 0x00, b'"', 0x80, b'}'];
        fs::write(workspace.join("current.json"), &raw).unwrap();
        let mut store = FileProfileStore::new(workspace.clone());
        assert_eq!(store.read_current(), None, "text that is not UTF-8 goes the bytes way");
        assert_eq!(store.read_current_bytes(), Some(raw.clone()));
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::Corrupt(_))), "{:?}", restored.fallback);
        assert_eq!(damaged_copies(&workspace), std::slice::from_ref(&raw));
        assert_eq!(fs::read(workspace.join("current.json")).unwrap(), raw);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_the_newest_damaged_copies_are_kept() {
        let dir = temp_dir();
        let workspace = dir.join("workspace");
        let mut store = FileProfileStore::new(workspace.clone());
        for n in 0..15u32 {
            store.keep_damaged(format!("{{{n}").as_bytes()).unwrap();
        }
        let kept = damaged_copies(&workspace);
        assert_eq!(kept.len(), MAX_DAMAGED_COPIES);
        assert_eq!(kept.first().map(Vec::as_slice), Some(b"{5".as_slice()));
        assert_eq!(kept.last().map(Vec::as_slice), Some(b"{14".as_slice()));
        fs::remove_dir_all(dir).unwrap();
    }

    // Lives here rather than in ui-egui's ws_perf.rs because the SLO covers the real disk store.
    #[test]
    #[ignore = "manual release performance measurement"]
    fn commit_p95() {
        let dir = temp_dir();
        let mut app = cadcraft_ui_egui::CadApp::new(cadcraft_engine::Session::new(), cadcraft_ui_egui::Services::default());
        app.skip_first_run_picker = true;
        app.attach_profile_store(Box::new(FileProfileStore::new(dir.join("workspace"))));
        let mut samples = Vec::with_capacity(100);
        for n in 0..100u64 {
            app.run("ui.cmdline.lines", serde_json::json!({"lines": n % 10 + 1})).unwrap();
            let started = std::time::Instant::now();
            app.commit_workspace_now().unwrap();
            samples.push(started.elapsed());
        }
        assert!(app.ws.store.as_deref().unwrap().snapshots().len() <= KEEP_SNAPSHOTS);
        fs::remove_dir_all(dir).unwrap();
        samples.sort_unstable();
        let p95 = samples[samples.len() * 95 / 100 - 1];
        println!("commit_p95: samples={}, P95={:.3} ms, SLO=250 ms", samples.len(), p95.as_secs_f64() * 1000.0);
        assert!(p95 <= std::time::Duration::from_millis(250), "commit_p95: P95 {p95:?} exceeds 250 ms");
    }
}
